//! G2 宿主业务接口；持久消费者与视图会话分离。
use crate::{
    alert_store::{self, Channel},
    alerts::{MarketKey, PriceKind, Rule},
    market,
    network::{self, NetworkConfig},
    store::{err, AppResult, Store},
    widgets, Core,
};
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager};

pub struct Subscription {
    pub owner: String,
    pub keys: HashSet<MarketKey>,
    pub sequence: u64,
}
pub struct Service {
    pub feed: Arc<market::Feed>,
    pub subscriptions: Mutex<HashMap<String, Subscription>>,
    pub network_ready: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_new_consumer_over_global_capacity_is_refused_before_persisting() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-capacity-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        {
            let db = store.db.lock().unwrap();
            db.execute(
                "INSERT INTO widget_packages VALUES('test','{}','test','test')",
                [],
            )
            .unwrap();
            for i in 0..16 {
                db.execute(
                    "INSERT INTO widget_instances(id,package_id,settings) VALUES(?1,'test','{}')",
                    [format!("instance-{i}")],
                )
                .unwrap();
            }
            for i in 0..299 {
                db.execute(
                    "INSERT INTO watchlist VALUES(?1,?2,?3)",
                    params![format!("instance-{}", i / 20), format!("S{i}USDT"), i % 20],
                )
                .unwrap();
            }
        }
        let (feed, _, _, _) = market::Feed::new(NetworkConfig::Direct);
        let service = Service {
            feed,
            subscriptions: Default::default(),
            network_ready: std::sync::atomic::AtomicBool::new(true),
        };
        assert_eq!(
            market::streams(&persistent_keys(&store).unwrap()).len(),
            899
        );
        assert!(
            save_watchlist_checked(&store, &service, "instance-15", &["NEWUSDT".into()]).is_err()
        );
        assert!(watchlist(&store, "instance-15").unwrap().is_empty());
        assert_eq!(
            market::streams(&persistent_keys(&store).unwrap()).len(),
            899
        );
    }
    #[test]
    fn removing_one_watchlist_keeps_other_instance_and_rule_consumers() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-consumers-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        {
            let db = store.db.lock().unwrap();
            db.execute(
                "INSERT INTO widget_packages VALUES('test','{}','test','test')",
                [],
            )
            .unwrap();
            for id in ["a", "b"] {
                db.execute(
                    "INSERT INTO widget_instances(id,package_id,settings) VALUES(?1,'test','{}')",
                    [id],
                )
                .unwrap();
            }
        }
        save_watchlist(&store, "a", &["BTCUSDT".into()]).unwrap();
        save_watchlist(&store, "b", &["BTCUSDT".into(), "ETHUSDT".into()]).unwrap();
        let r = Rule {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            name: "独立规则".into(),
            key: MarketKey::binance("SOLUSDT", PriceKind::Mark),
            direction: crate::alerts::Direction::Up,
            threshold: "100".into(),
            mode: crate::alerts::Mode::Once,
            cooldown_ms: 1000,
            enabled: true,
            notification: false,
            sound: false,
        };
        alert_store::save_rule(&store, &r, None).unwrap();
        save_watchlist(&store, "a", &[]).unwrap();
        assert_eq!(watchlist(&store, "b").unwrap(), vec!["BTCUSDT", "ETHUSDT"]);
        let keys = persistent_keys(&store).unwrap();
        assert_eq!(keys.len(), 5);
        assert!(keys.contains(&r.key));
        assert!(save_watchlist(&store, "b", &["BTCUSDT".into(), "BTCUSDT".into()]).is_err());
        assert_eq!(watchlist(&store, "b").unwrap().len(), 2);
        store
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM widget_instances", [])
            .unwrap();
        assert_eq!(persistent_keys(&store).unwrap(), HashSet::from([r.key]));
    }
}

pub fn resolve_network(value: &Value) -> AppResult<NetworkConfig> {
    if value["mode"] == "system" {
        network::system_proxy()
    } else {
        let c: NetworkConfig = serde_json::from_value(value.clone()).map_err(err)?;
        c.validate()?;
        Ok(c)
    }
}
pub fn watchlist(store: &Store, instance: &str) -> AppResult<Vec<String>> {
    let db = store.db.lock().map_err(err)?;
    let mut query = db
        .prepare("SELECT symbol FROM watchlist WHERE instance_id=?1 ORDER BY position")
        .map_err(err)?;
    let rows = query.query_map([instance], |r| r.get(0)).map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}
pub fn save_watchlist(store: &Store, instance: &str, symbols: &[String]) -> AppResult<()> {
    if symbols.len() > 20 || symbols.iter().collect::<HashSet<_>>().len() != symbols.len() {
        return Err("INVALID_ARGUMENT: 自选最多20个不同合约".into());
    }
    for s in symbols {
        MarketKey::binance(s, PriceKind::Last).validate()?;
    }
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM widget_instances WHERE id=?1)",
            [instance],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !exists {
        return Err("SESSION_EXPIRED: 实例已移除".into());
    }
    tx.execute("DELETE FROM watchlist WHERE instance_id=?1", [instance])
        .map_err(err)?;
    for (p, s) in symbols.iter().enumerate() {
        tx.execute(
            "INSERT INTO watchlist(instance_id,symbol,position) VALUES(?1,?2,?3)",
            params![instance, s, p],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)
}
fn ensure_capacity(
    store: &Store,
    subscriptions: &HashMap<String, Subscription>,
    extra: impl IntoIterator<Item = MarketKey>,
) -> AppResult<()> {
    let mut keys = persistent_keys(store)?;
    for sub in subscriptions.values() {
        keys.extend(sub.keys.iter().cloned());
    }
    let current = market::streams(&keys).len();
    keys.extend(extra);
    let next = market::streams(&keys).len();
    if next > 900 && next > current {
        return Err("LIMIT: 新增消费者会超过全局900行情流上限；未修改原自选、规则或连接，请先移除不需要的消费者".into());
    }
    Ok(())
}
fn save_watchlist_checked(
    store: &Store,
    service: &Service,
    instance: &str,
    symbols: &[String],
) -> AppResult<()> {
    let subs = service.subscriptions.lock().map_err(err)?;
    ensure_capacity(
        store,
        &subs,
        symbols.iter().flat_map(|s| {
            [
                MarketKey::binance(s, PriceKind::Last),
                MarketKey::binance(s, PriceKind::Mark),
            ]
        }),
    )?;
    save_watchlist(store, instance, symbols)
}
pub fn persistent_keys(store: &Store) -> AppResult<HashSet<MarketKey>> {
    let mut keys = HashSet::new();
    {
        let db = store.db.lock().map_err(err)?;
        let mut q = db
            .prepare("SELECT DISTINCT symbol FROM watchlist")
            .map_err(err)?;
        for row in q.query_map([], |r| r.get::<_, String>(0)).map_err(err)? {
            let s = row.map_err(err)?;
            keys.insert(MarketKey::binance(&s, PriceKind::Last));
            keys.insert(MarketKey::binance(&s, PriceKind::Mark));
        }
    }
    for (r, state) in alert_store::list_rules(store)? {
        if r.enabled && !state.completed {
            keys.insert(r.key);
        }
    }
    Ok(keys)
}
fn active_contract(service: &Service, key: &MarketKey) -> AppResult<()> {
    key.validate()?;
    if !service
        .feed
        .state
        .lock()
        .map_err(err)?
        .catalog
        .iter()
        .any(|c| c.symbol == key.symbol && c.status == "TRADING")
    {
        return Err("DATA_UNAVAILABLE: 合约不在当前可交易目录，请等待目录刷新".into());
    }
    Ok(())
}

pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let sessions = core.sessions.0.lock().map_err(err)?;
    let session = sessions
        .get(token)
        .filter(|s| s.window == window)
        .ok_or("SESSION_EXPIRED: 请重新加载组件")?;
    if session.preview.is_some() {
        return Err("CAPABILITY_DENIED: 试加载不启用真实行情及警报".into());
    }
    let definition:String=core.store.db.lock().map_err(err)?.query_row("SELECT p.manifest FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",[&session.instance],|r|r.get(0)).map_err(err)?;
    let manifest: widgets::Manifest = serde_json::from_str(&definition).map_err(err)?;
    let capability = if method.starts_with("alerts.") {
        "alerts.manage"
    } else {
        "market.read"
    };
    if !manifest.capabilities.iter().any(|c| c == capability) {
        return Err(format!("CAPABILITY_DENIED: 未声明{capability}"));
    }
    let result = match method {
        "market.catalog" => Ok(json!(core.business.feed.state.lock().map_err(err)?.catalog)),
        "watchlist.get" => Ok(json!(watchlist(&core.store, &session.instance)?)),
        "watchlist.set" => {
            let symbols: Vec<String> = serde_json::from_value(args).map_err(err)?;
            for symbol in &symbols {
                active_contract(&core.business, &MarketKey::binance(symbol, PriceKind::Last))?;
            }
            save_watchlist_checked(&core.store, &core.business, &session.instance, &symbols)?;
            Ok(json!(symbols))
        }
        "market.subscribe" => {
            let keys: Vec<MarketKey> = serde_json::from_value(args).map_err(err)?;
            if keys.len() > 40 {
                return Err("LIMIT: 单个订阅最多40个行情键".into());
            }
            for key in &keys {
                active_contract(&core.business, key)?;
            }
            let mut subs = core.business.subscriptions.lock().map_err(err)?;
            if subs.values().filter(|s| s.owner == token).count() >= 4 {
                return Err("LIMIT: 每会话最多4个订阅".into());
            }
            ensure_capacity(&core.store, &subs, keys.iter().cloned())?;
            let id = uuid::Uuid::new_v4().to_string();
            subs.insert(
                id.clone(),
                Subscription {
                    owner: token.into(),
                    keys: keys.into_iter().collect(),
                    sequence: 0,
                },
            );
            Ok(json!({"subscriptionId":id}))
        }
        "market.unsubscribe" => {
            let id = args["subscriptionId"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少订阅标识")?;
            let mut subs = core.business.subscriptions.lock().map_err(err)?;
            if subs.get(id).is_some_and(|s| s.owner != token) {
                return Err("CAPABILITY_DENIED: 订阅属于其他会话".into());
            }
            subs.remove(id);
            Ok(Value::Null)
        }
        "market.poll" => {
            let id = args["subscriptionId"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少订阅标识")?;
            let mut subs = core.business.subscriptions.lock().map_err(err)?;
            let sub = subs
                .get_mut(id)
                .filter(|s| s.owner == token)
                .ok_or("SESSION_EXPIRED: 订阅已失效")?;
            sub.sequence += 1;
            let mut snapshot = core.business.feed.snapshot()?;
            snapshot["quotes"].as_array_mut().unwrap().retain(|q| {
                serde_json::from_value::<MarketKey>(q["key"].clone())
                    .is_ok_and(|k| sub.keys.contains(&k))
            });
            snapshot["tickers"]
                .as_object_mut()
                .unwrap()
                .retain(|s, _| sub.keys.iter().any(|k| k.symbol == *s));
            snapshot.as_object_mut().unwrap().remove("catalog");
            Ok(json!({"subscriptionId":id,"sequence":sub.sequence,"data":snapshot}))
        }
        "alerts.list" => Ok(json!(alert_store::list_rules(&core.store)?
            .into_iter()
            .map(|(rule, state)| json!({"rule":rule,"state":state}))
            .collect::<Vec<_>>())),
        "alerts.save" => {
            let rule: Rule = serde_json::from_value(args["rule"].clone()).map_err(err)?;
            if rule.enabled {
                active_contract(&core.business, &rule.key)?;
                ensure_capacity(
                    &core.store,
                    &*core.business.subscriptions.lock().map_err(err)?,
                    [rule.key.clone()],
                )?;
            }
            if alert_store::list_rules(&core.store)?.len() >= 200
                && args["expectedRevision"].is_null()
            {
                return Err("LIMIT: 规则最多200条".into());
            }
            alert_store::save_rule(&core.store, &rule, args["expectedRevision"].as_u64())?;
            Ok(json!(rule))
        }
        "alerts.delete" => {
            alert_store::remove_rule(
                &core.store,
                args["id"].as_str().ok_or("缺少规则标识")?,
                args["revision"].as_u64().ok_or("缺少规则版本")?,
            )?;
            Ok(Value::Null)
        }
        "alerts.history" => Ok(json!(alert_store::list_events(
            &core.store,
            args["offset"].as_u64().unwrap_or(0),
            args["limit"].as_u64().unwrap_or(50).min(100) as u32
        )?)),
        "alerts.retry" => {
            let channel = match args["channel"].as_str() {
                Some("notification") => Channel::Notification,
                Some("sound") => Channel::Sound,
                _ => return Err("INVALID_ARGUMENT: 递送渠道无效".into()),
            };
            alert_store::retry_delivery(
                &core.store,
                args["id"].as_str().ok_or("缺少事件标识")?,
                channel,
            )?;
            Ok(Value::Null)
        }
        _ => Err("METHOD_NOT_FOUND: 未知业务接口".into()),
    };
    drop(sessions);
    result
}

pub fn start(
    app: &AppHandle,
    rx: tokio::sync::watch::Receiver<market::Desired>,
    tx: tokio::sync::mpsc::Sender<market::Input>,
    mut inputs: tokio::sync::mpsc::Receiver<market::Input>,
) {
    let feed = app.state::<Core>().business.feed.clone();
    let market_app = app.clone();
    let runner = feed.clone();
    tauri::async_runtime::spawn(async move {
        while !market_app
            .state::<Core>()
            .business
            .network_ready
            .load(Ordering::Acquire)
        {
            if runner.cancel.is_cancelled() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        market::run(runner, rx, tx).await;
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut engine = alert_store::Engine::default();
        let mut maintenance = std::time::Instant::now();
        while let Some(mut input) = inputs.recv().await {
            if handle.state::<Core>().quitting.load(Ordering::Relaxed) {
                break;
            }
            let _guard = feed.processing.lock().unwrap_or_else(|e| e.into_inner());
            if input.observation.generation != feed.generation.load(Ordering::SeqCst) {
                continue;
            }
            let core = handle.state::<Core>();
            let Some(source_now) = feed.source_now() else {
                continue;
            };
            input.context.clock = crate::alerts::Clock {
                utc_ms: crate::store::now_ms(),
                mono_ms: feed.origin.elapsed().as_millis() as u64,
            };
            input.context.source_now_utc_ms = source_now;
            if maintenance.elapsed() > std::time::Duration::from_secs(30) {
                if let Err(e) = engine.retain_rules(&core.store) {
                    core.report(e);
                }
                maintenance = std::time::Instant::now();
            }
            match engine.process(&core.store, &input.observation, input.context) {
                Ok(events) => {
                    for event in events {
                        if let Err(e) = core.store.log(
                            "alert",
                            &format!(
                                "事件 {} {} {:?} {:?}",
                                event.id, event.key.symbol, event.key.price_kind, event.reason
                            ),
                        ) {
                            core.report(e);
                        }
                    }
                }
                Err(e) => {
                    core.report(format!("警报状态写入失败，停止行情代次：{e}"));
                    core.business.feed.desired.send_modify(|d| d.reconnect += 1);
                    feed.generation.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut previous = String::new();
        loop {
            timer.tick().await;
            let core = handle.state::<Core>();
            if core.quitting.load(Ordering::Relaxed) {
                core.business.feed.cancel.cancel();
                break;
            }
            let refresh = || -> AppResult<()> {
                let sessions = core.sessions.0.lock().map_err(err)?;
                let mut subs = core.business.subscriptions.lock().map_err(err)?;
                subs.retain(|_, s| sessions.contains_key(&s.owner));
                let mut keys = persistent_keys(&core.store)?;
                for sub in subs.values() {
                    keys.extend(sub.keys.iter().cloned());
                }
                core.business.feed.desired.send_if_modified(|d| {
                    if d.keys != keys {
                        d.keys = keys;
                        true
                    } else {
                        false
                    }
                });
                Ok(())
            };
            if let Err(e) = refresh() {
                core.report(e);
            }
            let value = core.business.feed.snapshot().unwrap_or(Value::Null);
            let status=json!({"status":value["status"],"error":value["error"],"generation":value["generation"],"subscriptions":value["subscriptions"]}).to_string();
            if previous != status {
                if let Err(e) = core.store.log("market", &status) {
                    core.report(e);
                }
                previous = status;
            }
            let _ = handle.emit("business:changed", ());
        }
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if handle.state::<Core>().quitting.load(Ordering::Relaxed) {
                break;
            }
            let Some(source_now) = handle.state::<Core>().business.feed.source_now() else {
                continue;
            };
            let jobs =
                match alert_store::pending_deliveries(&handle.state::<Core>().store, source_now) {
                    Ok(j) => j,
                    Err(e) => {
                        handle.state::<Core>().report(e);
                        continue;
                    }
                };
            for event in jobs {
                for channel in [Channel::Notification, Channel::Sound] {
                    match alert_store::claim_delivery(
                        &handle.state::<Core>().store,
                        &event.id,
                        channel,
                    ) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(e) => {
                            handle.state::<Core>().report(e);
                            continue;
                        }
                    }
                    let app = handle.clone();
                    let event = event.clone();
                    let id = event.id.clone();
                    let outcome = tauri::async_runtime::spawn_blocking(move || match channel {
                        Channel::Notification => crate::alert_notification(&app, &event),
                        Channel::Sound => crate::independent_sound(),
                    })
                    .await;
                    let detail = format!("事件 {id} 渠道 {channel:?} 结果 {outcome:?}");
                    let saved = match outcome {
                        Ok(result) => alert_store::finish_delivery(
                            &handle.state::<Core>().store,
                            &id,
                            channel,
                            result,
                        ),
                        Err(e) => alert_store::finish_uncertain(
                            &handle.state::<Core>().store,
                            &id,
                            channel,
                            &format!("发送任务中断，系统结果不确定：{e}"),
                        ),
                    };
                    if let Err(e) = saved {
                        handle.state::<Core>().report(e);
                    }
                    if let Err(e) = handle.state::<Core>().store.log("delivery", &detail) {
                        handle.state::<Core>().report(e);
                    }
                }
            }
        }
    });
}
