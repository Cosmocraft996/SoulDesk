//! 原WidgetJS主流虚拟币的固定网页频道。它与公开指数及币安警报分别管理。
use crate::{
    coin,
    network::{self, NetworkConfig},
    store::{err, now_ms, AppResult},
    widgets, Core,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const URL: &str = "wss://wspri.okx.com:8443/ws/v5/ipublic";
const CHANNEL: &str = "cup-tickers-3s";
fn currencies() -> Vec<String> {
    let coins: Vec<Value> = serde_json::from_str(include_str!(
        "../../components/widgetjs/coin-common/coins.json"
    ))
    .expect("固定币种目录需有效");
    coins
        .into_iter()
        .filter_map(|coin| coin["ccy"].as_str().map(str::to_owned))
        .collect()
}
fn subscription() -> Value {
    json!({"op":"subscribe","args":currencies().iter().map(|ccy|json!({"channel":CHANNEL,"ccy":ccy})).collect::<Vec<_>>()})
}
fn quote(value: &Value, epoch: u64) -> AppResult<Value> {
    let ccy = value["ccy"]
        .as_str()
        .filter(|ccy| currencies().iter().any(|known| known == ccy))
        .ok_or("DATA_UNAVAILABLE: 未知币种身份")?;
    let price = coin::positive(&value["last"])?.ok_or("DATA_UNAVAILABLE: 网页币种报价缺失")?;
    let reference = coin::positive(&value["open24h"])?;
    let timestamp = value["ts"]
        .as_str()
        .filter(|text| {
            !text.is_empty() && text.len() <= 16 && text.bytes().all(|c| c.is_ascii_digit())
        })
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|time| *time > 0 && *time <= 8_640_000_000_000_000)
        .ok_or("DATA_UNAVAILABLE: 报价源时间无效")?;
    Ok(
        json!({"currency":ccy,"price":price,"reference":reference,"timestamp":timestamp,"receivedAt":now_ms(),"epoch":epoch}),
    )
}
#[derive(Clone)]
struct State {
    id: String,
    epoch: u64,
    connection: String,
    error: Option<String>,
    quotes: BTreeMap<String, Value>,
    errors: BTreeMap<String, String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            epoch: 0,
            connection: "connecting".into(),
            error: None,
            quotes: Default::default(),
            errors: Default::default(),
        }
    }
}
impl State {
    fn begin(&mut self) {
        self.epoch += 1;
        self.connection = "connecting".into();
        self.error = None;
        self.errors.clear();
    }
    fn snapshot(&self) -> Value {
        json!({"provider":"OKX","source":"coin-currencies","channel":CHANNEL,"connectionId":self.id,"epoch":self.epoch,"connection":self.connection,"error":self.error,"quotes":self.quotes.values().collect::<Vec<_>>(),"currencyErrors":self.errors})
    }
    fn apply(&mut self, text: &str) -> AppResult<()> {
        if text.len() > 64 * 1024 {
            return Err("LIMIT: 网页行情消息过大".into());
        }
        let value: Value =
            serde_json::from_str(text).map_err(|_| "DATA_UNAVAILABLE: 网页行情JSON无效")?;
        if value["event"] == "error" {
            let code = value["code"].as_str().unwrap_or("unknown");
            let message = format!(
                "来源拒绝订阅（{}）：{}",
                code,
                value["msg"]
                    .as_str()
                    .unwrap_or("未提供说明")
                    .chars()
                    .take(300)
                    .collect::<String>()
            );
            if let Some(ccy) = value["arg"]["ccy"]
                .as_str()
                .filter(|ccy| currencies().iter().any(|known| known == ccy))
            {
                self.errors.insert(ccy.into(), message);
                return Ok(());
            }
            return Err(message);
        }
        if value.get("data").is_none() {
            return Ok(());
        }
        if value["arg"]["channel"] != CHANNEL {
            return Err("DATA_UNAVAILABLE: 网页报价频道不匹配".into());
        }
        let data = value["data"]
            .as_array()
            .filter(|values| values.len() <= 100)
            .ok_or("DATA_UNAVAILABLE: 网页报价条目无效")?;
        for item in data {
            let Some(ccy) = item["ccy"]
                .as_str()
                .filter(|ccy| currencies().iter().any(|known| known == ccy))
            else {
                continue;
            };
            let parsed = (|| {
                if value["arg"]["ccy"]
                    .as_str()
                    .is_some_and(|expected| expected != ccy)
                {
                    return Err("DATA_UNAVAILABLE: 报价币种与频道身份不符".into());
                }
                let parsed = quote(item, self.epoch)?;
                if self
                    .quotes
                    .get(ccy)
                    .is_some_and(|old| old["timestamp"].as_u64() > parsed["timestamp"].as_u64())
                {
                    return Err("DATA_UNAVAILABLE: 来源返回旧时间，保留上次报价".into());
                }
                Ok(parsed)
            })();
            match parsed {
                Ok(parsed) => {
                    self.quotes.insert(ccy.into(), parsed);
                    self.errors.remove(ccy);
                }
                Err(error) => {
                    self.errors.insert(ccy.into(), error);
                }
            }
        }
        Ok(())
    }
}
struct Connection {
    owners: HashSet<String>,
    state: Arc<Mutex<State>>,
    cancel: CancellationToken,
}
#[derive(Default)]
struct Registry {
    connections: HashMap<String, Connection>,
    owners: HashMap<String, String>,
    retained: HashMap<String, (State, Instant)>,
}
impl Registry {
    fn release(&mut self, owner: &str) {
        if let Some(key) = self.owners.remove(owner) {
            if let Some(connection) = self.connections.get_mut(&key) {
                connection.owners.remove(owner);
                if connection.owners.is_empty() {
                    connection.cancel.cancel();
                    if let Ok(state) = connection.state.lock() {
                        if !state.quotes.is_empty() {
                            self.retained
                                .insert(key.clone(), (state.clone(), Instant::now()));
                        }
                    }
                    self.connections.remove(&key);
                    // 只留最近8种有效网络口径的报价；没有所有者时仍关闭套接字。
                    if self.retained.len() > 8 {
                        if let Some(oldest) = self
                            .retained
                            .iter()
                            .min_by_key(|(_, (_, at))| *at)
                            .map(|(key, _)| key.clone())
                        {
                            self.retained.remove(&oldest);
                        }
                    }
                }
            }
        }
    }
}
pub struct Service {
    registry: Mutex<Registry>,
    endpoint: String,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            registry: Default::default(),
            endpoint: URL.into(),
        }
    }
}
impl Service {
    // 仅隔离验收读取所有者数量与连接身份，不暴露代理地址或配置键。
    pub(crate) fn diagnostics(&self) -> AppResult<Value> {
        let registry = self.registry.lock().map_err(err)?;
        let mut values = Vec::new();
        for connection in registry.connections.values() {
            let state = connection.state.lock().map_err(err)?;
            let last_received_at = state
                .quotes
                .values()
                .filter_map(|q| q["receivedAt"].as_u64())
                .max();
            values.push(json!({"connectionId":state.id,"owners":connection.owners.len(),"epoch":state.epoch,"connection":state.connection,"quoteCount":state.quotes.len(),"lastReceivedAt":last_received_at}));
        }
        Ok(json!({"connections":values,"ownerCount":registry.owners.len()}))
    }
    // 仅异步宿主网关调用；组件不能指定地址或订阅参数。
    fn poll(&self, owner: &str, config: NetworkConfig) -> AppResult<Value> {
        config.validate()?;
        let key = serde_json::to_string(&config).map_err(err)?;
        let mut registry = self.registry.lock().map_err(err)?;
        if registry.owners.get(owner).is_some_and(|old| old != &key) {
            registry.release(owner);
        }
        if !registry.connections.contains_key(&key) {
            let mut initial = registry
                .retained
                .remove(&key)
                .map(|(state, _)| state)
                .unwrap_or_default();
            // 新逻辑连接使用独立状态，避免已取消旧任务的晚到结果污染恢复画面。
            initial.id = uuid::Uuid::new_v4().to_string();
            initial.connection = "connecting".into();
            initial.error = None;
            let state = Arc::new(Mutex::new(initial));
            let cancel = CancellationToken::new();
            tokio::spawn(run(
                config,
                self.endpoint.clone(),
                state.clone(),
                cancel.clone(),
            ));
            registry.connections.insert(
                key.clone(),
                Connection {
                    owners: HashSet::new(),
                    state,
                    cancel,
                },
            );
        }
        registry.owners.insert(owner.into(), key.clone());
        let connection = registry.connections.get_mut(&key).unwrap();
        connection.owners.insert(owner.into());
        let value = connection.state.lock().map_err(err)?.snapshot();
        Ok(value)
    }
    pub fn release_owners(&self, owners: &HashSet<String>) -> AppResult<()> {
        let mut registry = self.registry.lock().map_err(err)?;
        for owner in owners {
            registry.release(owner);
        }
        Ok(())
    }
    pub fn retain_owners(&self, owners: &HashSet<String>) -> AppResult<()> {
        let mut registry = self.registry.lock().map_err(err)?;
        let removed = registry
            .owners
            .keys()
            .filter(|owner| !owners.contains(*owner))
            .cloned()
            .collect::<Vec<_>>();
        for owner in removed {
            registry.release(&owner);
        }
        Ok(())
    }
    pub fn stop_all(&self) -> AppResult<()> {
        let mut registry = self.registry.lock().map_err(err)?;
        for connection in registry.connections.values() {
            connection.cancel.cancel();
        }
        registry.owners.clear();
        registry.connections.clear();
        registry.retained.clear();
        Ok(())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.stop_all();
    }
}
#[derive(Clone, Copy)]
struct Timings {
    tick: Duration,
    ping: Duration,
    silence: Duration,
}
impl Default for Timings {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            ping: Duration::from_secs(20),
            silence: Duration::from_secs(35),
        }
    }
}
async fn connected(
    config: &NetworkConfig,
    endpoint: &str,
    state: &Arc<Mutex<State>>,
    cancel: &CancellationToken,
    timing: Timings,
) -> AppResult<()> {
    let mut socket = network::connect_ws(config, endpoint, cancel).await?;
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(subscription().to_string().into())),
    )
    .await
    .map_err(err)?
    .map_err(err)?;
    state.lock().map_err(err)?.connection = "connected".into();
    let mut clock = tokio::time::interval(timing.tick);
    let mut received = tokio::time::Instant::now();
    let mut ping = received;
    loop {
        tokio::select! {biased;
            _=cancel.cancelled()=>return Ok(()),
            _=clock.tick()=>{
                if received.elapsed()>timing.silence{return Err("DATA_UNAVAILABLE: 网页行情心跳超时".into());}
                if ping.elapsed()>=timing.ping{tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Text("ping".into()))).await.map_err(err)?.map_err(err)?;ping=tokio::time::Instant::now();}
            },
            frame=socket.next()=>{
                let frame=frame.ok_or("DATA_UNAVAILABLE: 网页行情连接已结束")?.map_err(err)?;received=tokio::time::Instant::now();
                match frame {
                    Message::Text(text) if text=="pong"=>{},
                    Message::Text(text)=>state.lock().map_err(err)?.apply(&text)?,
                    Message::Ping(value)=>{tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Pong(value))).await.map_err(err)?.map_err(err)?;},
                    Message::Close(_)=>return Err("DATA_UNAVAILABLE: 来源关闭网页行情连接".into()),
                    Message::Binary(_)=>return Err("DATA_UNAVAILABLE: 网页行情返回了未知二进制消息".into()),
                    _=>{},
                }
            }
        }
    }
}
async fn run(
    config: NetworkConfig,
    endpoint: String,
    state: Arc<Mutex<State>>,
    cancel: CancellationToken,
) {
    let mut delay = 1u64;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Ok(mut state) = state.lock() {
            state.begin();
        } else {
            return;
        }
        let started = tokio::time::Instant::now();
        let result = connected(&config, &endpoint, &state, &cancel, Timings::default()).await;
        if started.elapsed() > Duration::from_secs(30) {
            delay = 1;
        }
        if cancel.is_cancelled() {
            return;
        }
        if let Ok(mut state) = state.lock() {
            state.connection = "retrying".into();
            state.error = Some(result.err().unwrap_or_else(|| "网页行情连接已断开".into()));
        } else {
            return;
        }
        tokio::select! {biased;_=cancel.cancelled()=>return,_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
        delay = (delay * 2).min(30);
    }
}
pub async fn gateway(app: &AppHandle, window: &str, token: &str, args: Value) -> AppResult<Value> {
    if args["source"] != "coin-currencies" {
        return Err("CAPABILITY_DENIED: 未开放的实时内容源".into());
    }
    let core = app.state::<Core>();
    let instance =
        widgets::require_capability(&core.store, &core.sessions, window, token, "content.live")?;
    let settings: String = core
        .store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT settings FROM widget_instances WHERE id=?1",
            [instance],
            |row| row.get(0),
        )
        .map_err(err)?;
    let settings: Value = serde_json::from_str(&settings).map_err(err)?;
    let inherited = if settings["proxyMode"].as_str().unwrap_or("inherit") == "inherit" {
        let config: Value = serde_json::from_str(
            &core
                .store
                .preference("network")
                .unwrap_or("{\"mode\":\"system\"}".into()),
        )
        .map_err(err)?;
        crate::business::resolve_network(&config)?
    } else {
        NetworkConfig::Direct
    };
    let config = coin::instance_network(&settings, inherited)?;
    let data = core.coin_live.poll(token, config)?;
    if let Err(error) =
        widgets::require_capability(&core.store, &core.sessions, window, token, "content.live")
    {
        core.coin_live
            .release_owners(&HashSet::from([token.to_owned()]))?;
        return Err(error);
    }
    Ok(json!({"source":"coin-currencies","fetchedAt":now_ms(),"data":data}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formal_instances_have_separate_settings_and_preview_cannot_acquire_stream_capability() {
        let store = crate::store::Store::open(
            std::env::temp_dir().join(format!("souldesk-coin-live-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.coin-medium")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let first =
            widgets::create_instance(&store, "local.souldesk.widgetjs.coin-medium").unwrap();
        let second =
            widgets::create_instance(&store, "local.souldesk.widgetjs.coin-medium").unwrap();
        let sessions = widgets::Sessions::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        assert_eq!(
            widgets::require_capability(&store, &sessions, &label, token, "content.live").unwrap(),
            first
        );
        let mut settings = defaults.clone();
        settings["proxyMode"] = json!("direct");
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "settings.update",
            settings.clone(),
        )
        .unwrap();
        assert_eq!(
            widgets::gateway(&store, &sessions, &label, token, "settings.get", json!({})).unwrap(),
            settings
        );
        assert_eq!(
            store.snapshot().unwrap()["instances"]
                .as_array()
                .unwrap()
                .iter()
                .find(|instance| instance["id"] == second)
                .unwrap()["settings"],
            defaults
        );
        let preview = Arc::new(Mutex::new(widgets::Preview {
            package: staged,
            settings: defaults,
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        sessions.0.lock().unwrap().insert(
            "preview".into(),
            widgets::Session {
                instance: first,
                window: "trial-coin".into(),
                shared_cursor: None,
                preview: Some(preview),
            },
        );
        assert!(widgets::require_capability(
            &store,
            &sessions,
            "trial-coin",
            "preview",
            "content.live"
        )
        .is_err());
        assert!(widgets::require_capability(
            &store,
            &sessions,
            "another-window",
            token,
            "content.live"
        )
        .is_err());
    }
    #[tokio::test]
    async fn application_ping_pong_and_silence_timeout_are_bounded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            let ping = socket.next().await.unwrap().unwrap();
            assert_eq!(ping.into_text().unwrap(), "ping");
            socket.send(Message::Text("pong".into())).await.unwrap();
            while socket.next().await.is_some() {}
        });
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().unwrap().begin();
        let cancel = CancellationToken::new();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            connected(
                &NetworkConfig::Direct,
                &endpoint,
                &state,
                &cancel,
                Timings {
                    tick: Duration::from_millis(10),
                    ping: Duration::from_millis(30),
                    silence: Duration::from_millis(90),
                },
            ),
        )
        .await
        .unwrap();
        assert!(result.unwrap_err().contains("心跳超时"));
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn peer_close_reconnects_with_a_new_epoch_and_resubscribes_original_currencies() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let peer = tokio::spawn(async move {
            for attempt in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let request = socket.next().await.unwrap().unwrap().into_text().unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&request).unwrap(),
                    subscription()
                );
                socket
                    .send(Message::Text(
                        frame(
                            "BTC",
                            if attempt == 0 { "101" } else { "102" },
                            if attempt == 0 { "1000" } else { "2000" },
                        )
                        .into(),
                    ))
                    .await
                    .unwrap();
                if attempt == 0 {
                    socket.close(None).await.unwrap();
                } else {
                    while socket.next().await.is_some() {}
                }
            }
        });
        let state = Arc::new(Mutex::new(State::default()));
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(run(
            NetworkConfig::Direct,
            endpoint,
            state.clone(),
            cancel.clone(),
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state
                    .lock()
                    .unwrap()
                    .quotes
                    .get("BTC")
                    .is_some_and(|quote| quote["price"] == "102")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        {
            let state = state.lock().unwrap();
            assert!(state.epoch >= 2);
            assert_eq!(state.quotes["BTC"]["epoch"], state.epoch);
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .unwrap()
            .unwrap();
    }
    fn frame(ccy: &str, price: &str, time: &str) -> String {
        json!({"arg":{"channel":CHANNEL,"ccy":ccy},"data":[{"ccy":ccy,"last":price,"open24h":"100","ts":time}]}).to_string()
    }
    #[test]
    fn original_currency_protocol_keeps_exact_prices_identity_and_rejects_old_or_invalid_frames() {
        assert_eq!(subscription()["args"].as_array().unwrap().len(), 14);
        let mut state = State::default();
        state.begin();
        state
            .apply(&frame("BTC", "12345678901234567890.123456", "1000"))
            .unwrap();
        assert_eq!(state.quotes["BTC"]["price"], "12345678901234567890.123456");
        state.apply(&frame("BTC", "1", "999")).unwrap();
        assert_eq!(state.quotes["BTC"]["timestamp"], 1000);
        assert!(state.errors.contains_key("BTC"));
        state.apply(&frame("BTC", "NaN", "2000")).unwrap();
        assert_eq!(state.quotes["BTC"]["timestamp"], 1000);
        let mut wrong: Value = serde_json::from_str(&frame("ETH", "100", "2000")).unwrap();
        wrong["arg"]["ccy"] = json!("BTC");
        state.apply(&wrong.to_string()).unwrap();
        assert!(!state.quotes.contains_key("ETH"));
        wrong["arg"]["channel"] = json!("tickers");
        assert!(state.apply(&wrong.to_string()).is_err());
        state.begin();
        assert_ne!(state.quotes["BTC"]["epoch"], state.epoch);
    }
    #[tokio::test]
    async fn instances_share_only_equal_networks_and_releasing_one_does_not_cancel_another() {
        let service = Service {
            registry: Default::default(),
            endpoint: "ws://127.0.0.1:1".into(),
        };
        service.poll("first", NetworkConfig::Direct).unwrap();
        service.poll("second", NetworkConfig::Direct).unwrap();
        assert_eq!(service.registry.lock().unwrap().connections.len(), 1);
        let original = service
            .registry
            .lock()
            .unwrap()
            .connections
            .values()
            .next()
            .unwrap()
            .cancel
            .clone();
        service
            .release_owners(&HashSet::from(["first".into()]))
            .unwrap();
        assert!(!original.is_cancelled());
        service
            .poll(
                "second",
                NetworkConfig::Http {
                    host: "127.0.0.1".into(),
                    port: 1,
                },
            )
            .unwrap();
        assert!(original.is_cancelled());
        let current = service
            .registry
            .lock()
            .unwrap()
            .connections
            .values()
            .next()
            .unwrap()
            .cancel
            .clone();
        service.retain_owners(&HashSet::new()).unwrap();
        assert!(current.is_cancelled());
        assert!(service.registry.lock().unwrap().connections.is_empty());
    }
    #[tokio::test]
    async fn reopening_views_retains_quotes_without_retaining_an_unowned_socket_or_relabelling_time(
    ) {
        let service = Service {
            registry: Default::default(),
            endpoint: "ws://127.0.0.1:1".into(),
        };
        let first = service.poll("first", NetworkConfig::Direct).unwrap();
        let (old_state, cancelled) = {
            let registry = service.registry.lock().unwrap();
            let connection = registry.connections.values().next().unwrap();
            (connection.state.clone(), connection.cancel.clone())
        };
        {
            let mut state = old_state.lock().unwrap();
            state.begin();
            state.apply(&frame("BTC", "100.25", "1000")).unwrap();
        }
        service
            .release_owners(&HashSet::from(["first".into()]))
            .unwrap();
        assert!(cancelled.is_cancelled());
        assert_eq!(service.diagnostics().unwrap()["ownerCount"], 0);
        assert!(service.registry.lock().unwrap().connections.is_empty());
        let restored = service.poll("restored", NetworkConfig::Direct).unwrap();
        assert_ne!(restored["connectionId"], first["connectionId"]);
        assert_eq!(restored["connection"], "connecting");
        assert_eq!(restored["quotes"][0]["price"], "100.25");
        assert_eq!(restored["quotes"][0]["timestamp"], 1000);
        let received = restored["quotes"][0]["receivedAt"].clone();
        old_state
            .lock()
            .unwrap()
            .apply(&frame("BTC", "999", "2000"))
            .unwrap();
        let current = service.poll("restored", NetworkConfig::Direct).unwrap();
        assert_eq!(current["quotes"][0]["price"], "100.25");
        assert_eq!(current["quotes"][0]["receivedAt"], received);
        let other = service
            .poll(
                "other",
                NetworkConfig::Http {
                    host: "127.0.0.1".into(),
                    port: 9,
                },
            )
            .unwrap();
        assert!(other["quotes"].as_array().unwrap().is_empty());
        service.stop_all().unwrap();
        assert!(service.registry.lock().unwrap().retained.is_empty());
    }
    #[tokio::test]
    async fn inactive_quote_cache_is_bounded_without_keeping_connections_alive() {
        let service = Service {
            registry: Default::default(),
            endpoint: "ws://127.0.0.1:1".into(),
        };
        for port in 1..=9 {
            service
                .poll(
                    "view",
                    NetworkConfig::Http {
                        host: "127.0.0.1".into(),
                        port,
                    },
                )
                .unwrap();
            {
                let registry = service.registry.lock().unwrap();
                let mut state = registry
                    .connections
                    .values()
                    .next()
                    .unwrap()
                    .state
                    .lock()
                    .unwrap();
                state.begin();
                state.apply(&frame("BTC", "100", "1000")).unwrap();
            }
            service
                .release_owners(&HashSet::from(["view".into()]))
                .unwrap();
        }
        let registry = service.registry.lock().unwrap();
        assert_eq!(registry.retained.len(), 8);
        assert!(registry.owners.is_empty());
        assert!(registry.connections.is_empty());
        let first = serde_json::to_string(&NetworkConfig::Http {
            host: "127.0.0.1".into(),
            port: 1,
        })
        .unwrap();
        assert!(!registry.retained.contains_key(&first));
    }
    #[tokio::test]
    async fn native_socket_subscribes_to_original_channel_and_cancel_releases_without_waiting_for_peer(
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&request).unwrap(),
                subscription()
            );
            socket
                .send(Message::Text(
                    frame("BTC", "101.0001", "1794657600000").into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        });
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().unwrap().begin();
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(run(
            NetworkConfig::Direct,
            endpoint,
            state.clone(),
            cancel.clone(),
        ));
        tokio::time::timeout(Duration::from_secs(3), async {
            while !state.lock().unwrap().quotes.contains_key("BTC") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), fixture)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    #[ignore = "显式检查原网页频道的真实可用性"]
    async fn real_original_currency_socket_checks_fresh_source_quote() {
        let state = Arc::new(Mutex::new(State::default()));
        state.lock().unwrap().begin();
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(run(
            network::system_proxy().unwrap(),
            URL.into(),
            state.clone(),
            cancel.clone(),
        ));
        let waited = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if !state.lock().unwrap().quotes.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(6), worker)
            .await
            .unwrap()
            .unwrap();
        let data = state.lock().unwrap().snapshot();
        let fresh = data["quotes"].as_array().unwrap().iter().any(|quote| {
            quote["timestamp"].as_u64().is_some_and(|timestamp| {
                timestamp <= now_ms() + 5000 && now_ms().saturating_sub(timestamp) < 120000
            })
        });
        let report = json!({"source":URL,"channel":CHANNEL,"checkedAt":now_ms(),"status":if waited.is_ok()&&fresh{"passed"}else{"failed"},"fresh":fresh,"snapshot":data,"scope":"原生固定网页频道；不等于桌面/全部币种/自定义代理验收","normalDataOpened":false});
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../artifacts/widgetjs-okx-cup-native-probe.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(fresh, "原网页频道未返回新鲜币种报价，详见隔离报告");
    }
}
