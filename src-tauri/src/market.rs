//! 币安 USD-M 公开行情：严格解析、共享订阅、连接代次与逐流恢复。
use crate::{
    alerts::{self, Clock, Context, MarketKey, Observation, PriceKind},
    network::{self, NetworkConfig},
    store::{err, now_ms, AppResult},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contract {
    pub symbol: String,
    pub base_asset: String,
    pub contract_type: String,
    pub status: String,
    pub tick_size: String,
}

pub fn parse_catalog(text: &str) -> AppResult<Vec<Contract>> {
    let value: Value = serde_json::from_str(text).map_err(err)?;
    let symbols = value["symbols"]
        .as_array()
        .ok_or("PROTOCOL_CHANGED: 目录缺少symbols")?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for v in symbols {
        if v["quoteAsset"] != "USDT"
            || !matches!(
                v["contractType"].as_str(),
                Some("PERPETUAL" | "TRADIFI_PERPETUAL")
            )
        {
            continue;
        }
        let symbol = v["symbol"]
            .as_str()
            .ok_or("PROTOCOL_CHANGED: 缺少合约代码")?;
        MarketKey::binance(symbol, PriceKind::Last).validate()?;
        if !seen.insert(symbol.to_owned()) {
            return Err("PROTOCOL_CHANGED: 目录合约重复".into());
        }
        let tick = v["filters"]
            .as_array()
            .and_then(|a| a.iter().find(|f| f["filterType"] == "PRICE_FILTER"))
            .and_then(|f| f["tickSize"].as_str())
            .ok_or("PROTOCOL_CHANGED: 缺少价格步长")?;
        alerts::parse_price(tick)?;
        result.push(Contract {
            symbol: symbol.into(),
            base_asset: v["baseAsset"]
                .as_str()
                .ok_or("PROTOCOL_CHANGED: 缺少基础资产")?
                .into(),
            contract_type: v["contractType"].as_str().unwrap().into(),
            status: v["status"]
                .as_str()
                .ok_or("PROTOCOL_CHANGED: 缺少合约状态")?
                .into(),
            tick_size: tick.into(),
        });
    }
    if result.is_empty() {
        return Err("DATA_UNAVAILABLE: 无有效USDT永续合约目录".into());
    }
    result.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(result)
}

pub enum Parsed {
    Session {
        event: String,
        value: Value,
    },
    ContractUpdate {
        symbol: String,
        status: String,
    },
    Price {
        key: MarketKey,
        price: String,
        time: u64,
        trade_id: Option<u64>,
    },
    Ticker {
        symbol: String,
        value: Value,
    },
}
pub fn parse_frame(
    text: &str,
    catalog: &[Contract],
    desired: &HashSet<MarketKey>,
) -> AppResult<Option<Parsed>> {
    let value: Value = serde_json::from_str(text).map_err(err)?;
    if value.get("id").is_some() {
        if value.get("code").is_some() {
            return Err("SUBSCRIBE_FAILED: 交易所拒绝订阅".into());
        }
        return Ok(None);
    }
    let data = value.get("data").unwrap_or(&value);
    let event = data["e"].as_str().ok_or("PROTOCOL_CHANGED: 缺少事件类型")?;
    if [
        "EquityUpdate",
        "CommodityUpdate",
        "KR_EquityUpdate",
        "HK_EquityUpdate",
        "CN_EquityUpdate",
        "FXUpdate",
    ]
    .contains(&event)
    {
        if !matches!(
            data["S"].as_str(),
            Some("PRE_MARKET" | "REGULAR" | "AFTER_MARKET" | "OVERNIGHT" | "NO_TRADING")
        ) || data["E"].as_u64().is_none()
            || data["t"].as_u64().is_none()
            || data["T"].as_u64().is_none()
        {
            return Err("PROTOCOL_CHANGED: 标的交易时段字段无效".into());
        }
        return Ok(Some(Parsed::Session {
            event: event.into(),
            value: json!({"session":data["S"],"start":data["t"],"end":data["T"],"time":data["E"]}),
        }));
    }
    let symbol = data["s"].as_str().ok_or("PROTOCOL_CHANGED: 行情缺少合约")?;
    // 当前合并市场协议明确带st；缺失时也不猜测市场身份。
    if event == "contractInfo" && data["st"] == 2 {
        return Ok(None);
    }
    if data["st"] != 1 {
        return Err("WRONG_MARKET: USD-M行情缺少或不匹配st=1".into());
    }
    if event == "contractInfo" {
        return Ok(Some(Parsed::ContractUpdate {
            symbol: symbol.into(),
            status: data["cs"]
                .as_str()
                .ok_or("PROTOCOL_CHANGED: 合约状态缺失")?
                .into(),
        }));
    }
    let contract = catalog
        .iter()
        .find(|c| c.symbol == symbol)
        .ok_or("UNKNOWN_CONTRACT: 报价不在当前目录")?;
    if contract.status != "TRADING" {
        return Ok(None);
    }
    let time = data["E"]
        .as_u64()
        .filter(|t| *t > 0)
        .ok_or("PROTOCOL_CHANGED: 缺少事件时间")?;
    let (kind, stream) = match data["e"].as_str() {
        Some("aggTrade") => (PriceKind::Last, "aggTrade"),
        Some("markPriceUpdate") => (PriceKind::Mark, "markPrice@1s"),
        Some("24hrTicker") => {
            if !desired.iter().any(|k| k.symbol == symbol) {
                return Ok(None);
            }
            alerts::parse_price(data["c"].as_str().ok_or("INVALID_PRICE: 缺少Last统计价")?)?;
            if value["stream"]
                .as_str()
                .is_some_and(|s| s != format!("{}@ticker", symbol.to_lowercase()))
            {
                return Err("WRONG_MARKET: 统计流名称不一致".into());
            }
            return Ok(Some(Parsed::Ticker {
                symbol: symbol.into(),
                value: json!({"priceKind":"last","changePercent":data["P"],"last":data["c"],"time":time}),
            }));
        }
        _ => return Err("PROTOCOL_CHANGED: 未识别的行情事件".into()),
    };
    if let Some(stream_name) = value.get("stream").and_then(Value::as_str) {
        if stream_name != format!("{}@{stream}", symbol.to_lowercase()) {
            return Err("WRONG_MARKET: 流名称与载荷不一致".into());
        }
    }
    let key = MarketKey::binance(symbol, kind);
    if !desired.contains(&key) {
        return Ok(None);
    }
    let price = data["p"].as_str().ok_or("INVALID_PRICE: 缺少价格")?;
    alerts::parse_price(price)?;
    Ok(Some(Parsed::Price {
        key,
        price: price.into(),
        time: if kind == PriceKind::Last {
            data["T"]
                .as_u64()
                .filter(|t| *t > 0 && *t <= time)
                .ok_or("PROTOCOL_CHANGED: 成交时间无效")?
        } else {
            time
        },
        trade_id: if kind == PriceKind::Last {
            Some(
                data["a"]
                    .as_u64()
                    .ok_or("PROTOCOL_CHANGED: 缺少聚合成交序号")?,
            )
        } else {
            None
        },
    }))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub key: MarketKey,
    pub price: String,
    pub source_utc_ms: u64,
    pub generation: u64,
    pub quality: String,
    #[serde(skip)]
    received: u64,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sessions: HashMap<String, Value>,
    pub catalog: Vec<Contract>,
    pub quotes: Vec<Quote>,
    pub tickers: HashMap<String, Value>,
    pub status: String,
    pub error: Option<String>,
    pub generation: u64,
    pub subscriptions: usize,
    pub received: u64,
    pub interruptions: u64,
    pub queue_overflows: u64,
    pub source_now: Option<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Desired {
    pub network: NetworkConfig,
    pub keys: HashSet<MarketKey>,
    pub reconnect: u64,
}
pub struct Feed {
    queue: tokio::sync::mpsc::WeakSender<Input>,
    pub processing: Mutex<()>,
    pub desired: watch::Sender<Desired>,
    pub state: Arc<Mutex<Snapshot>>,
    pub generation: Arc<AtomicU64>,
    pub cancel: CancellationToken,
    pub origin: Instant,
    source_clock: Mutex<Option<(u64, Instant)>>,
}
pub struct Input {
    pub observation: Observation,
    pub context: Context,
}
impl Feed {
    pub fn new(
        network: NetworkConfig,
    ) -> (
        Arc<Self>,
        watch::Receiver<Desired>,
        mpsc::Sender<Input>,
        mpsc::Receiver<Input>,
    ) {
        let (desired, rx) = watch::channel(Desired {
            network,
            keys: HashSet::new(),
            reconnect: 0,
        });
        let (tx, inputs) = mpsc::channel(2048);
        (
            Arc::new(Self {
                queue: tx.downgrade(),
                processing: Mutex::new(()),
                desired,
                state: Default::default(),
                generation: Arc::new(AtomicU64::new(0)),
                cancel: CancellationToken::new(),
                origin: Instant::now(),
                source_clock: Mutex::new(None),
            }),
            rx,
            tx,
            inputs,
        )
    }
    pub fn snapshot(&self) -> AppResult<Value> {
        let state = self.state.lock().map_err(err)?;
        let mut v = serde_json::to_value(&*state).map_err(err)?;
        let mono = self.origin.elapsed().as_millis() as u64;
        for (row, q) in v["quotes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&state.quotes)
        {
            row["quality"] = json!(if state.status != "connected"
                || q.generation != state.generation
            {
                "unavailable"
            } else if mono.saturating_sub(q.received) > 10_000 {
                "stale"
            } else {
                &q.quality
            });
            row["ageMs"] = json!(mono.saturating_sub(q.received));
        }
        v["sourceNow"] = json!(self.source_now());
        v["queueDepth"] = json!(self
            .queue
            .upgrade()
            .map(|s| s.max_capacity() - s.capacity())
            .unwrap_or(0));
        Ok(v)
    }
    pub fn fresh_mark_count(&self) -> AppResult<usize> {
        let s = self.state.lock().map_err(err)?;
        let mono = self.origin.elapsed().as_millis() as u64;
        if s.status != "connected" {
            return Ok(0);
        }
        Ok(s.quotes
            .iter()
            .filter(|q| {
                q.key.price_kind == PriceKind::Mark
                    && q.generation == s.generation
                    && mono.saturating_sub(q.received) <= 10000
            })
            .count())
    }
    pub fn source_now(&self) -> Option<u64> {
        self.source_now_at(Instant::now())
    }
    fn source_now_at(&self, now: Instant) -> Option<u64> {
        self.source_clock
            .lock()
            .ok()?
            .as_ref()
            .and_then(|(utc, at)| {
                let elapsed = now.checked_duration_since(*at)?;
                (elapsed < Duration::from_secs(900))
                    .then(|| utc.saturating_add(elapsed.as_millis() as u64))
            })
    }
    fn status(&self, status: &str, error: Option<String>) {
        if let Ok(mut s) = self.state.lock() {
            s.status = status.into();
            s.error = error;
        }
    }
    fn invalidate(&self) {
        let _guard = self.processing.lock().unwrap_or_else(|e| e.into_inner());
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut s) = self.state.lock() {
            s.generation = g;
            s.interruptions += 1;
            s.status = "reconnecting".into();
        }
    }
    pub fn reconnect(&self) {
        self.invalidate();
        self.desired.send_modify(|d| d.reconnect += 1);
    }
}
pub fn streams(keys: &HashSet<MarketKey>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for k in keys {
        result.insert(format!(
            "{}@{}",
            k.symbol.to_lowercase(),
            if k.price_kind == PriceKind::Last {
                "aggTrade"
            } else {
                "markPrice@1s"
            }
        ));
        result.insert(format!("{}@ticker", k.symbol.to_lowercase()));
    }
    if !keys.is_empty() {
        result.insert("!contractInfo".into());
        result.insert("tradingSession".into());
    }
    result
}

#[derive(Clone)]
struct Endpoints {
    time: String,
    catalog: String,
    ws: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            time: "https://fapi.binance.com/fapi/v1/time".into(),
            catalog: "https://fapi.binance.com/fapi/v1/exchangeInfo".into(),
            ws: "wss://fstream.binance.com/market/stream".into(),
        }
    }
}
pub async fn run(feed: Arc<Feed>, desired: watch::Receiver<Desired>, tx: mpsc::Sender<Input>) {
    run_endpoints(feed, desired, tx, Endpoints::default()).await;
}
async fn run_endpoints(
    feed: Arc<Feed>,
    mut desired: watch::Receiver<Desired>,
    tx: mpsc::Sender<Input>,
    endpoints: Endpoints,
) {
    let mut attempt = 0u32;
    loop {
        if feed.cancel.is_cancelled() {
            break;
        }
        let config = desired.borrow_and_update().clone();
        let result = tokio::select! {biased;_=feed.cancel.cancelled()=>Err("CANCELLED".into()),result=connection(&feed, &mut desired, &config, &tx,&endpoints)=>result};
        feed.invalidate();
        if feed.cancel.is_cancelled() {
            break;
        }
        match result {
            Ok(()) => {
                attempt = 0;
                continue;
            }
            Err(e) => feed.status("reconnecting", Some(e)),
        }
        let seconds = [1, 2, 4, 8, 16, 30][attempt.min(5) as usize];
        attempt = attempt.saturating_add(1);
        let jitter = now_ms() % 251;
        tokio::select! {_=feed.cancel.cancelled()=>break,_=desired.changed()=>{attempt=0;},_=tokio::time::sleep(Duration::from_millis(seconds*1000+jitter))=>{}}
    }
    feed.status("stopped", None);
}

async fn connection(
    feed: &Feed,
    desired: &mut watch::Receiver<Desired>,
    config: &Desired,
    tx: &mpsc::Sender<Input>,
    endpoints: &Endpoints,
) -> AppResult<()> {
    feed.status("connecting", None);
    let cancel = feed.cancel.child_token();
    let bootstrap = async {
        let start = Instant::now();
        let time = network::http_get(&config.network, &endpoints.time, &cancel).await?;
        let server: Value = serde_json::from_str(&time).map_err(err)?;
        let source = server["serverTime"]
            .as_u64()
            .ok_or("PROTOCOL_CHANGED: 无服务器时间")?;
        let latency = start.elapsed().as_millis() as u64;
        if latency > 5000 {
            return Err("CLOCK_UNCERTAIN: 时间请求往返超过5秒".into());
        }
        let anchor = Instant::now();
        let source = source + latency / 2;
        let catalog =
            parse_catalog(&network::http_get(&config.network, &endpoints.catalog, &cancel).await?)?;
        Ok::<_, String>((source, anchor, catalog))
    };
    let (source, anchor, catalog) = tokio::select! {biased;_=feed.cancel.cancelled()=>return Err("CANCELLED".into()),_=desired.changed()=>return Ok(()),r=bootstrap=>r?};
    *feed.source_clock.lock().map_err(err)? = Some((source, anchor));
    feed.state.lock().map_err(err)?.catalog = catalog.clone();
    if config.keys.is_empty() {
        feed.status("idle", None);
        tokio::select! {_=feed.cancel.cancelled()=>{},_=desired.changed()=>{},_=tokio::time::sleep(Duration::from_secs(600))=>{}}
        return Ok(());
    }
    let supported = |keys: &HashSet<MarketKey>| {
        keys.iter()
            .filter(|k| {
                catalog
                    .iter()
                    .any(|c| c.symbol == k.symbol && c.status == "TRADING")
            })
            .cloned()
            .collect::<HashSet<_>>()
    };
    let mut keys = supported(&config.keys);
    let mut active = streams(&keys);
    if active.len() > 900 {
        return Err("LIMIT: 行情订阅超过900条".into());
    }
    if active.is_empty() {
        return Err("DATA_UNAVAILABLE: 所需合约均不在可交易目录".into());
    }
    let url = format!(
        "{}?streams={}",
        endpoints.ws,
        active.iter().cloned().collect::<Vec<_>>().join("/")
    );
    let url = url::Url::parse(&url).map_err(err)?.to_string();
    let mut ws = tokio::select! {biased;_=desired.changed()=>return Ok(()),r=network::connect_ws(&config.network,&url,&cancel)=>r?};
    let generation = {
        let _guard = feed.processing.lock().map_err(err)?;
        feed.generation.fetch_add(1, Ordering::SeqCst) + 1
    };
    {
        let mut s = feed.state.lock().map_err(err)?;
        s.generation = generation;
        s.subscriptions = active.len();
        s.status = "connected".into();
        s.error = None;
    }
    let mut ordinal = 0;
    let mut last = HashMap::<MarketKey, (u64, String, Option<u64>, u64)>::new();
    let mut last_frame = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.tick().await;
    let mut controls = tokio::time::interval(Duration::from_secs(1));
    controls.tick().await;
    let mut target = active.clone();
    let mut ack: Option<(String, Instant)> = None;
    let mut request = 0u64;
    loop {
        tokio::select! {biased;
            _=feed.cancel.cancelled()=>return Err("CANCELLED".into()),
            changed=desired.changed()=>{changed.map_err(err)?;let next=desired.borrow_and_update().clone();if next.network!=config.network||next.reconnect!=config.reconnect||next.keys.is_empty(){return Ok(());}keys=supported(&next.keys);target=streams(&keys);if target.len()>900{return Err("LIMIT: 行情订阅超过900条".into());}},
            _=controls.tick()=>{
                if ack.as_ref().is_some_and(|(_,at)|at.elapsed()>Duration::from_secs(10)){return Err("SUBSCRIBE_TIMEOUT: 订阅确认超时".into());}
                if ack.is_none()&&target!=active {let removed=active.difference(&target).cloned().collect::<Vec<_>>();let added=target.difference(&active).cloned().collect::<Vec<_>>();let(method,params)=if !removed.is_empty(){("UNSUBSCRIBE",removed)}else{("SUBSCRIBE",added)};request+=1;let id=format!("g{generation}-{request}");ws.send(Message::Text(json!({"method":method,"params":params,"id":id}).to_string().into())).await.map_err(err)?;ack=Some((id,Instant::now()));if method=="UNSUBSCRIBE"{for p in params{active.remove(&p);}}else{active.extend(params);}feed.state.lock().map_err(err)?.subscriptions=active.len();}
            },
            _=heartbeat.tick()=>{
                if last_frame.elapsed()>Duration::from_secs(60){return Err("HEARTBEAT_TIMEOUT: 60秒未收到任何数据或心跳".into());}
                // 定期校正源时钟并轮换连接，寿命远短于官方24小时上限。
                if anchor.elapsed()>Duration::from_secs(600){return Ok(());}
                ws.send(Message::Ping(Vec::new().into())).await.map_err(err)?;
            },
            frame=ws.next()=>{let frame=frame.ok_or("DISCONNECTED: 行情连接结束")?.map_err(err)?;last_frame=Instant::now();match frame {
                Message::Ping(_)=>{ws.flush().await.map_err(err)?;},Message::Close(_)=>return Err("DISCONNECTED: 服务端关闭连接".into()),
                Message::Text(text)=>{
                    let v:Value=serde_json::from_str(&text).map_err(err)?;
                    if let Some(id)=v["id"].as_str(){if v.get("code").is_some(){return Err("SUBSCRIBE_FAILED: 交易所拒绝订阅".into());}if ack.as_ref().is_some_and(|(pending,_)|pending==id){ack=None;}continue;}
                    let mono=feed.origin.elapsed().as_millis() as u64;let source_now=source+anchor.elapsed().as_millis() as u64;
                    match parse_frame(&text,&catalog,&keys)? {
                        Some(Parsed::Session{event,mut value})=>{value["generation"]=json!(generation);feed.state.lock().map_err(err)?.sessions.insert(event,value);},
                        Some(Parsed::ContractUpdate{symbol,status})=>{if catalog.iter().find(|c|c.symbol==symbol).is_none_or(|c|c.status!=status){return Ok(());}},
                        Some(Parsed::Ticker{symbol,value})=>{feed.state.lock().map_err(err)?.tickers.insert(symbol,value);},
                        Some(Parsed::Price{key,price,time,trade_id})=>{
                            if time>source_now.saturating_add(5000)||source_now.saturating_sub(time)>10000 {continue;}
                            if last.get(&key).is_some_and(|(t,p,id,_)|time<*t||(time==*t&&price==*p)||trade_id.zip(*id).is_some_and(|(a,b)|a<=b)){continue;}
                            let recovered=last.get(&key).is_none_or(|(_,_,_,at)|mono.saturating_sub(*at)>10_000);
                            last.insert(key.clone(),(time,price.clone(),trade_id,mono));ordinal+=1;
                            let input=Input{observation:Observation{key:key.clone(),price:price.clone(),source_utc_ms:time,received_mono_ms:mono,generation,ordinal,recovered,valid:true},context:Context{clock:Clock{utc_ms:now_ms(),mono_ms:mono},generation,source_now_utc_ms:source_now}};
                            if tx.try_send(input).is_err(){feed.state.lock().map_err(err)?.queue_overflows+=1;return Err("QUEUE_OVERFLOW: 警报输入队列已满，停止该代次并恢复".into());}
                            let mut s=feed.state.lock().map_err(err)?;s.received+=1;s.source_now=Some(source_now);s.quotes.retain(|q|q.key!=key&&keys.contains(&q.key));s.quotes.push(Quote{key,price,source_utc_ms:time,generation,quality:"fresh".into(),received:mono});
                        },None=>{}
                    }
                },_=>{}
            }}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queue_depth_tracks_enqueued_and_consumed_observations_without_keeping_sender_alive() {
        let (feed, _, tx, mut rx) = Feed::new(NetworkConfig::Direct);
        let quote = Observation {
            key: MarketKey::binance("BTCUSDT", PriceKind::Mark),
            price: "100".into(),
            source_utc_ms: 1,
            received_mono_ms: 1,
            generation: 1,
            ordinal: 1,
            recovered: false,
            valid: true,
        };
        tx.try_send(Input {
            observation: quote,
            context: Context {
                clock: Clock {
                    utc_ms: 1,
                    mono_ms: 1,
                },
                generation: 1,
                source_now_utc_ms: 1,
            },
        })
        .unwrap();
        assert_eq!(feed.snapshot().unwrap()["queueDepth"], 1);
        rx.recv().await.unwrap();
        assert_eq!(feed.snapshot().unwrap()["queueDepth"], 0);
        drop(tx);
        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .is_none());
    }
    async fn local_endpoints(burst: bool) -> (Endpoints, Vec<tokio::task::JoinHandle<()>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = http.local_addr().unwrap().port();
        let http_task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = http.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0; 2048];
                    let n = stream.read(&mut buf).await.unwrap();
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let rows=["BTCUSDT","ETHUSDT"].into_iter().map(|s|json!({"symbol":s,"baseAsset":s,"quoteAsset":"USDT","contractType":"PERPETUAL","status":"TRADING","filters":[{"filterType":"PRICE_FILTER","tickSize":"0.1"}]})).collect::<Vec<_>>();
                    let body = if request.contains("/time") {
                        json!({"serverTime":now_ms()})
                    } else {
                        json!({"symbols":rows})
                    }
                    .to_string();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                });
            }
        });
        let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_port = ws.local_addr().unwrap().port();
        let ws_task = tokio::spawn(async move {
            loop {
                let (stream, _) = ws.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let base = now_ms();
                    for index in 0..if burst { 12 } else { 2 } {
                        if !burst && index == 1 {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                        let symbol = if !burst && index == 1 {
                            "ETHUSDT"
                        } else {
                            "BTCUSDT"
                        };
                        let v = json!({"e":"markPriceUpdate","s":symbol,"p":format!("{}",100+index),"E":base+index,"st":1});
                        if ws.send(Message::Text(v.to_string().into())).await.is_err() {
                            return;
                        }
                    }
                    while let Some(Ok(frame)) = ws.next().await {
                        match frame {
                            Message::Ping(_) => {
                                let _ = ws.flush().await;
                            }
                            Message::Text(text) => {
                                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                    let _ = ws
                                        .send(Message::Text(
                                            json!({"result":null,"id":v["id"]}).to_string().into(),
                                        ))
                                        .await;
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        (
            Endpoints {
                time: format!("http://127.0.0.1:{port}/time"),
                catalog: format!("http://127.0.0.1:{port}/catalog"),
                ws: format!("ws://127.0.0.1:{ws_port}/stream"),
            },
            vec![http_task, ws_task],
        )
    }
    #[tokio::test]
    async fn reconnect_recovers_each_stream_independently_and_cancels_the_live_reader() {
        let (endpoints, servers) = local_endpoints(false).await;
        let (feed, rx, tx, mut input) = Feed::new(NetworkConfig::Direct);
        feed.desired.send_modify(|d| {
            d.keys = HashSet::from([
                MarketKey::binance("BTCUSDT", PriceKind::Mark),
                MarketKey::binance("ETHUSDT", PriceKind::Mark),
            ])
        });
        let run = tokio::spawn(run_endpoints(feed.clone(), rx, tx, endpoints));
        let first = tokio::time::timeout(Duration::from_secs(3), input.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(first.observation.recovered);
        let second = tokio::time::timeout(Duration::from_secs(3), input.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.observation.key.symbol, "ETHUSDT");
        assert!(second.observation.recovered);
        feed.desired.send_modify(|d| d.reconnect += 1);
        let third = tokio::time::timeout(Duration::from_secs(3), input.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(third.observation.generation > first.observation.generation);
        assert!(third.observation.recovered);
        let snapshot = feed.snapshot().unwrap();
        assert!(snapshot["quotes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["key"]["symbol"] == "ETHUSDT" && q["quality"] == "unavailable"));
        let fourth = tokio::time::timeout(Duration::from_secs(3), input.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fourth.observation.key.symbol, "ETHUSDT");
        assert!(fourth.observation.recovered);
        feed.cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), run)
            .await
            .unwrap()
            .unwrap();
        for task in servers {
            task.abort();
        }
    }
    #[tokio::test]
    async fn queue_overflow_invalidates_generation_instead_of_silently_dropping_a_price() {
        let (endpoints, servers) = local_endpoints(true).await;
        let (feed, rx, _, _) = Feed::new(NetworkConfig::Direct);
        let (tx, mut pending) = mpsc::channel(4);
        feed.desired.send_modify(|d| {
            d.keys = HashSet::from([MarketKey::binance("BTCUSDT", PriceKind::Mark)])
        });
        let run = tokio::spawn(run_endpoints(feed.clone(), rx, tx, endpoints));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if feed.state.lock().unwrap().queue_overflows > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let old = pending.recv().await.unwrap();
        assert_ne!(
            old.observation.generation,
            feed.generation.load(Ordering::SeqCst)
        );
        assert_eq!(feed.snapshot().unwrap()["status"], "reconnecting");
        feed.cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), run)
            .await
            .unwrap()
            .unwrap();
        for task in servers {
            task.abort();
        }
    }
    #[test]
    fn last_price_uses_trade_time_and_session_metadata_does_not_pretend_to_be_a_quote() {
        let keys = HashSet::from([MarketKey::binance("BTCUSDT", PriceKind::Last)]);
        let trade = json!({"e":"aggTrade","s":"BTCUSDT","p":"100","E":2000,"T":1000,"a":2,"st":1});
        assert!(matches!(
            parse_frame(&trade.to_string(), &catalog(), &keys).unwrap(),
            Some(Parsed::Price { time: 1000, .. })
        ));
        let session = json!({"e":"EquityUpdate","E":2000,"t":1000,"T":3000,"S":"NO_TRADING"});
        assert!(matches!(
            parse_frame(&session.to_string(), &catalog(), &keys).unwrap(),
            Some(Parsed::Session { .. })
        ));
    }
    #[test]
    fn source_clock_expires_and_snapshot_never_exposes_an_old_generation_as_fresh() {
        let (feed, _, _, _) = Feed::new(NetworkConfig::Direct);
        assert!(feed.source_now().is_none());
        *feed.source_clock.lock().unwrap() = Some((1_700_000_000_000, Instant::now()));
        assert!(feed.source_now().unwrap() >= 1_700_000_000_000);
        // 向未来推进观测点，避免新启动的Windows虚拟机不能回拨15分钟。
        let at = Instant::now();
        *feed.source_clock.lock().unwrap() = Some((1, at));
        assert!(feed.source_now_at(at + Duration::from_secs(899)).is_some());
        assert!(feed.source_now_at(at + Duration::from_secs(900)).is_none());
        assert!(feed.source_now_at(at + Duration::from_secs(901)).is_none());
        {
            let mut s = feed.state.lock().unwrap();
            s.status = "connected".into();
            s.generation = 2;
            s.quotes.push(Quote {
                key: MarketKey::binance("BTCUSDT", PriceKind::Last),
                price: "100".into(),
                source_utc_ms: 1,
                generation: 1,
                quality: "fresh".into(),
                received: 0,
            });
        }
        assert_eq!(
            feed.snapshot().unwrap()["quotes"][0]["quality"],
            "unavailable"
        );
    }
    fn catalog() -> Vec<Contract> {
        vec![Contract {
            symbol: "BTCUSDT".into(),
            base_asset: "BTC".into(),
            contract_type: "PERPETUAL".into(),
            status: "TRADING".into(),
            tick_size: "0.1".into(),
        }]
    }
    #[test]
    fn directory_rejects_unknown_precision_and_excludes_spot_and_delivery() {
        let row = json!({"symbol":"BTCUSDT","baseAsset":"BTC","quoteAsset":"USDT","contractType":"PERPETUAL","status":"TRADING","filters":[{"filterType":"PRICE_FILTER","tickSize":"0.1"}]});
        let mut tradfi = row.clone();
        tradfi["symbol"] = json!("XAUUSDT");
        tradfi["contractType"] = json!("TRADIFI_PERPETUAL");
        let mut excluded = row.clone();
        excluded["contractType"] = json!("CURRENT_QUARTER");
        assert_eq!(
            parse_catalog(&json!({"symbols":[row.clone(),tradfi,excluded]}).to_string())
                .unwrap()
                .len(),
            2
        );
        let mut bad = row;
        bad["filters"] = json!([]);
        assert!(parse_catalog(&json!({"symbols":[bad]}).to_string()).is_err());
    }
    #[test]
    fn identity_price_kind_and_source_id_are_not_interchangeable() {
        let keys = HashSet::from([
            MarketKey::binance("BTCUSDT", PriceKind::Last),
            MarketKey::binance("BTCUSDT", PriceKind::Mark),
        ]);
        let mut frame = json!({"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","s":"BTCUSDT","p":"0.1000000000000000001","E":1,"T":1,"a":2,"st":1}});
        assert!(
            matches!(parse_frame(&frame.to_string(),&catalog(),&keys).unwrap(),Some(Parsed::Price{key,trade_id:Some(2),..}) if key.price_kind==PriceKind::Last)
        );
        frame["data"]["st"] = json!(2);
        assert!(parse_frame(&frame.to_string(), &catalog(), &keys).is_err());
        frame["data"]["st"] = json!(1);
        frame["stream"] = json!("btcusdt@markPrice@1s");
        assert!(parse_frame(&frame.to_string(), &catalog(), &keys).is_err());
    }
    #[test]
    fn shared_streams_deduplicate_and_unrelated_consumers_keep_their_keys() {
        let a = MarketKey::binance("BTCUSDT", PriceKind::Last);
        let b = MarketKey::binance("BTCUSDT", PriceKind::Mark);
        assert_eq!(streams(&HashSet::from([a.clone(), a, b])).len(), 5);
    }
}
