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
use serde::{Deserialize, Serialize};
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
    pub spot: Arc<market::Feed>,
    pub charts: crate::market_history::History,
    pub subscriptions: Mutex<HashMap<String, Subscription>>,
    pub network_ready: std::sync::atomic::AtomicBool,
}
impl Service {
    pub fn feed_for(&self, market: &str) -> &Arc<market::Feed> {
        if market == "spot" {
            &self.spot
        } else {
            &self.feed
        }
    }
    fn feeds(&self) -> [&Arc<market::Feed>; 2] {
        [&self.feed, &self.spot]
    }
    pub fn reconnect(&self) {
        for feed in self.feeds() {
            feed.reconnect();
        }
    }
    fn snapshot(&self, keys: &HashSet<MarketKey>) -> AppResult<Value> {
        let mut result = self.feed.snapshot_for(keys)?;
        let spot = self.spot.snapshot_for(keys)?;
        let quotes = result["quotes"].as_array_mut().unwrap();
        quotes.extend(spot["quotes"].as_array().unwrap().iter().cloned());
        let mut tickers = serde_json::Map::new();
        for (market, snapshot) in [("usd_m", &result), ("spot", &spot)] {
            for (symbol, ticker) in snapshot["tickers"].as_object().unwrap() {
                if keys
                    .iter()
                    .any(|key| key.market == market && key.symbol == *symbol)
                {
                    let mut value = ticker.clone();
                    value["fresh"] = json!(
                        snapshot["status"] == "connected"
                            && snapshot["sourceNow"]
                                .as_u64()
                                .zip(ticker["time"].as_u64())
                                .is_some_and(|(now, time)| now.saturating_sub(time) <= 10_000)
                    );
                    tickers.insert(format!("{market}:{symbol}"), value);
                }
            }
        }
        result["marketStatus"] = json!({"usd_m":{"status":result["status"],"error":result["error"],"sourceNow":result["sourceNow"]},"spot":{"status":spot["status"],"error":spot["error"],"sourceNow":spot["sourceNow"]}});
        result["marketTickers"] = json!(tickers);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_watchlist_retains_legacy_items_and_same_symbol_market_identity() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-market-mixed-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        {
            let db = store.db.lock().unwrap();
            db.execute(
                "INSERT INTO widget_packages VALUES('test','{}','test','test')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO widget_instances(id,package_id,settings) VALUES('market','test','{}')",
                [],
            )
            .unwrap();
            db.execute("INSERT INTO watchlist VALUES('market','BTCUSDT',0)", [])
                .unwrap();
        }
        let mut items = watch_items(&store, "market").unwrap();
        assert_eq!(items[0].market, "usd_m");
        items.insert(
            0,
            WatchItem {
                market: "spot".into(),
                symbol: "BTCUSDT".into(),
                alias: "现货观察".into(),
            },
        );
        save_watch_items(&store, "market", &items).unwrap();
        let read = watch_items(&store, "market").unwrap();
        assert_eq!((&*read[0].market, &*read[0].alias), ("spot", "现货观察"));
        assert_eq!(watchlist(&store, "market").unwrap(), vec!["BTCUSDT"]);
        assert_eq!(
            persistent_keys(&store).unwrap(),
            HashSet::from([
                MarketKey::spot("BTCUSDT"),
                MarketKey::binance("BTCUSDT", PriceKind::Last),
                MarketKey::binance("BTCUSDT", PriceKind::Mark)
            ])
        );
        save_watchlist(&store, "market", &[]).unwrap();
        assert_eq!(
            persistent_keys(&store).unwrap(),
            HashSet::from([MarketKey::spot("BTCUSDT")])
        );
    }
    #[test]
    fn more_than_200_rules_share_spot_feed_and_keep_recovery_and_cooldown() {
        use crate::alerts::{Clock, Context, Direction, Mode, Observation};
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-market-alerts-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let service = Service {
            feed: market::Feed::new(NetworkConfig::Direct).0,
            spot: market::Feed::for_market(NetworkConfig::Direct, "spot").0,
            charts: Default::default(),
            subscriptions: Default::default(),
            network_ready: Default::default(),
        };
        service
            .spot
            .state
            .lock()
            .unwrap()
            .catalog
            .push(market::Contract {
                symbol: "BTCUSDT".into(),
                base_asset: "BTC".into(),
                quote_asset: "USDT".into(),
                market: "spot".into(),
                contract_type: "SPOT".into(),
                status: "TRADING".into(),
                tick_size: "0.01".into(),
            });
        let key = MarketKey::spot("BTCUSDT");
        for i in 0..205 {
            let rule = Rule {
                condition: Default::default(),
                notification_channel: Default::default(),
                id: uuid::Uuid::new_v4().to_string(),
                revision: 1,
                name: format!("现货 {i}"),
                key: key.clone(),
                direction: Direction::Up,
                threshold: "100".into(),
                mode: Mode::Repeat,
                cooldown_ms: 1000,
                enabled: true,
                notification: false,
                sound: false,
            };
            save_market_rule(&store, &service, rule, None).unwrap();
        }
        assert_eq!(alert_store::list_rules(&store).unwrap().len(), 205);
        assert_eq!(
            persistent_keys(&store).unwrap(),
            HashSet::from([key.clone()])
        );
        let mut engine = alert_store::Engine::default();
        let mut observe = |market_key: &MarketKey, price: &str, t: u64, generation, recovered| {
            let now = 1_760_000_000_000 + t;
            engine
                .process(
                    &store,
                    &Observation {
                        key: market_key.clone(),
                        price: price.into(),
                        source_utc_ms: now,
                        received_mono_ms: t,
                        generation,
                        ordinal: t,
                        recovered,
                        valid: true,
                    },
                    Context {
                        clock: Clock {
                            utc_ms: now,
                            mono_ms: t,
                        },
                        generation,
                        source_now_utc_ms: now,
                    },
                )
                .unwrap()
        };
        assert!(observe(
            &MarketKey::binance("BTCUSDT", PriceKind::Last),
            "100",
            1,
            1,
            false
        )
        .is_empty());
        assert!(observe(&key, "99", 2, 1, false).is_empty());
        assert_eq!(observe(&key, "100", 3, 2, true).len(), 205);
        assert!(observe(&key, "99", 4, 2, false).is_empty());
        assert!(observe(&key, "100", 5, 2, false).is_empty());
        assert_eq!(observe(&key, "100", 1200, 2, false).len(), 205);
    }
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
            spot: market::Feed::for_market(NetworkConfig::Direct, "spot").0,
            charts: Default::default(),
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
            condition: Default::default(),
            notification_channel: Default::default(),
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
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WatchItem {
    market: String,
    symbol: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    alias: String,
}
impl WatchItem {
    fn legacy(symbol: &str) -> Self {
        Self {
            market: "usd_m".into(),
            symbol: symbol.into(),
            alias: String::new(),
        }
    }
    fn decode(value: String) -> AppResult<Self> {
        if value.starts_with('{') {
            serde_json::from_str(&value).map_err(err)
        } else {
            Ok(Self::legacy(&value))
        }
    }
    fn key(&self, price_kind: PriceKind) -> MarketKey {
        MarketKey {
            market: self.market.clone(),
            ..MarketKey::binance(&self.symbol, price_kind)
        }
    }
    fn keys(&self) -> impl Iterator<Item = MarketKey> + '_ {
        [PriceKind::Last, PriceKind::Mark]
            .into_iter()
            .filter(|kind| self.market == "usd_m" || *kind == PriceKind::Last)
            .map(|kind| self.key(kind))
    }
}
fn watch_items(store: &Store, instance: &str) -> AppResult<Vec<WatchItem>> {
    let db = store.db.lock().map_err(err)?;
    let mut query = db
        .prepare("SELECT symbol FROM watchlist WHERE instance_id=?1 ORDER BY position")
        .map_err(err)?;
    let rows = query
        .query_map([instance], |r| r.get::<_, String>(0))
        .map_err(err)?;
    rows.map(|row| WatchItem::decode(row.map_err(err)?))
        .collect()
}
pub fn watchlist(store: &Store, instance: &str) -> AppResult<Vec<String>> {
    Ok(watch_items(store, instance)?
        .into_iter()
        .filter(|item| item.market == "usd_m")
        .map(|item| item.symbol)
        .collect())
}
pub fn save_watchlist(store: &Store, instance: &str, symbols: &[String]) -> AppResult<()> {
    let previous = watch_items(store, instance)?;
    let mut items = symbols
        .iter()
        .map(|symbol| {
            previous
                .iter()
                .find(|item| item.market == "usd_m" && item.symbol == *symbol)
                .cloned()
                .unwrap_or_else(|| WatchItem::legacy(symbol))
        })
        .collect::<Vec<_>>();
    items.extend(previous.into_iter().filter(|item| item.market == "spot"));
    save_watch_items(store, instance, &items)
}
fn save_watch_items(store: &Store, instance: &str, items: &[WatchItem]) -> AppResult<()> {
    let distinct = items
        .iter()
        .map(|item| (&item.market, &item.symbol))
        .collect::<HashSet<_>>();
    if items.len() > 20 || distinct.len() != items.len() {
        return Err("INVALID_ARGUMENT: 自选最多20个不同标的".into());
    }
    for item in items {
        item.key(PriceKind::Last).validate()?;
        if item.alias.chars().count() > 32 || item.alias.chars().any(char::is_control) {
            return Err("INVALID_ARGUMENT: 显示别名无效".into());
        }
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
    for (p, item) in items.iter().enumerate() {
        // 沿用位置表及旧版纯合约值，扩展市场或别名的条目使用结构化数据。
        let value = if item.market == "usd_m" && item.alias.is_empty() {
            item.symbol.clone()
        } else {
            serde_json::to_string(item).map_err(err)?
        };
        tx.execute(
            "INSERT INTO watchlist(instance_id,symbol,position) VALUES(?1,?2,?3)",
            params![instance, value, p],
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
    let count = |keys: &HashSet<MarketKey>, market_name| {
        market::streams(
            &keys
                .iter()
                .filter(|key| key.market == market_name)
                .cloned()
                .collect(),
        )
        .len()
    };
    let before = [count(&keys, "usd_m"), count(&keys, "spot")];
    keys.extend(extra);
    if [count(&keys, "usd_m"), count(&keys, "spot")]
        .into_iter()
        .zip(before)
        .any(|(next, current)| next > 900 && next > current)
    {
        return Err("LIMIT: 新增消费者会超过对应市场的行情流上限".into());
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
            let item = WatchItem::decode(row.map_err(err)?)?;
            keys.extend(item.keys());
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
        .feed_for(&key.market)
        .state
        .lock()
        .map_err(err)?
        .catalog
        .iter()
        .any(|c| c.symbol == key.symbol && c.status == "TRADING")
    {
        return Err("DATA_UNAVAILABLE: 标的不在当前可交易目录，请等待目录刷新".into());
    }
    Ok(())
}

fn save_market_rule(
    store: &Store,
    service: &Service,
    rule: Rule,
    expected: Option<u64>,
) -> AppResult<Value> {
    let subs = service.subscriptions.lock().map_err(err)?;
    if rule.enabled {
        active_contract(service, &rule.key)?;
        ensure_capacity(store, &subs, [rule.key.clone()])?;
    }
    alert_store::save_rule(store, &rule, expected)?;
    Ok(json!(rule))
}

pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    if matches!(method, "market.candles" | "market.openInterest") {
        let require = || {
            widgets::require_capability(&core.store, &core.sessions, window, token, "market.chart")
        };
        require()?;
        let request: crate::market_history::Request = serde_json::from_value(args).map_err(err)?;
        request.key.validate()?;
        let feed = core.business.feed_for(&request.key.market);
        if !feed
            .state
            .lock()
            .map_err(err)?
            .catalog
            .iter()
            .any(|entry| entry.symbol == request.key.symbol)
        {
            return Err("DATA_UNAVAILABLE: 目录尚未确认该标的".into());
        }
        let config = feed.desired.borrow().network.clone();
        let value = core
            .business
            .charts
            .get(request, &config, method == "market.openInterest")
            .await?;
        require()?;
        if feed.desired.borrow().network != config {
            return Err("DATA_UNAVAILABLE: 网络配置已变化，请重试".into());
        }
        return Ok(value);
    }
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
        "market.preferences" => {
            let patch = args
                .as_object()
                .filter(|value| {
                    !value.is_empty()
                        && value.keys().all(|key| {
                            matches!(key.as_str(), "layout" | "rangeHours" | "maPeriods")
                        })
                })
                .ok_or("INVALID_ARGUMENT: 行情设置字段无效")?;
            let db = core.store.db.lock().map_err(err)?;
            let saved: String = db
                .query_row(
                    "SELECT settings FROM widget_instances WHERE id=?1",
                    [&session.instance],
                    |row| row.get(0),
                )
                .map_err(err)?;
            let mut settings: Value = serde_json::from_str(&saved).map_err(err)?;
            settings
                .as_object_mut()
                .ok_or("STATE_INVALID: 组件设置无效")?
                .extend(patch.clone());
            widgets::validate_settings(&manifest.settings_schema, &settings)?;
            if settings
                .get("layout")
                .is_some_and(|value| !matches!(value.as_str(), Some("list" | "grid")))
                || settings.get("rangeHours").is_some_and(|value| {
                    !matches!(value.as_i64(), Some(1 | 4 | 6 | 8 | 12 | 24 | 72 | 168))
                })
            {
                return Err("INVALID_ARGUMENT: 行情布局或区间无效".into());
            }
            db.execute(
                "UPDATE widget_instances SET settings=?1 WHERE id=?2",
                params![settings.to_string(), session.instance],
            )
            .map_err(err)?;
            Ok(settings)
        }
        "market.catalog" => Ok(json!(core.business.feed.state.lock().map_err(err)?.catalog)),
        "market.instruments" => {
            let mut result = core
                .business
                .feed
                .state
                .lock()
                .map_err(err)?
                .catalog
                .clone();
            result.extend(
                core.business
                    .spot
                    .state
                    .lock()
                    .map_err(err)?
                    .catalog
                    .iter()
                    .cloned(),
            );
            Ok(json!(result))
        }
        "watchlist.get" => Ok(json!(watchlist(&core.store, &session.instance)?)),
        "watchlist.getItems" => Ok(json!(watch_items(&core.store, &session.instance)?)),
        "watchlist.setItems" => {
            let items: Vec<WatchItem> = serde_json::from_value(args).map_err(err)?;
            let previous = watch_items(&core.store, &session.instance)?;
            for item in &items {
                if !previous
                    .iter()
                    .any(|old| old.market == item.market && old.symbol == item.symbol)
                {
                    active_contract(&core.business, &item.key(PriceKind::Last))?;
                }
            }
            let subs = core.business.subscriptions.lock().map_err(err)?;
            ensure_capacity(&core.store, &subs, items.iter().flat_map(WatchItem::keys))?;
            save_watch_items(&core.store, &session.instance, &items)?;
            Ok(json!(items))
        }
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
            let previous = watch_items(&core.store, &session.instance)?;
            for key in &keys {
                key.validate()?;
                if !previous
                    .iter()
                    .any(|item| item.market == key.market && item.symbol == key.symbol)
                {
                    active_contract(&core.business, key)?;
                }
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
            let snapshot = core.business.snapshot(&sub.keys)?;
            Ok(json!({"subscriptionId":id,"sequence":sub.sequence,"data":snapshot}))
        }
        "alerts.list" => Ok(json!(alert_store::list_rules(&core.store)?
            .into_iter()
            .map(|(rule, state)| json!({"rule":rule,"state":state}))
            .collect::<Vec<_>>())),
        "alerts.save" => {
            let rule: Rule = serde_json::from_value(args["rule"].clone()).map_err(err)?;
            save_market_rule(
                &core.store,
                &core.business,
                rule,
                args["expectedRevision"].as_u64(),
            )
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
        "alerts.historySettings" => alert_store::history_settings(&core.store, args.get("days")),
        "alerts.historyClear" => Ok(json!({"cleared":alert_store::clear_history(&core.store)?})),
        "alerts.retry" => {
            let channel = match args["channel"].as_str() {
                Some("notification") => Channel::Notification,
                Some("sound") => return Err("INVALID_ARGUMENT: 行情铃声由灵动通知设置管理".into()),
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

fn market_log_status(feed: &market::Feed) -> AppResult<String> {
    #[derive(serde::Serialize)]
    struct Status<'a> {
        error: Option<&'a str>,
        generation: u64,
        status: &'a str,
        subscriptions: usize,
    }

    let state = feed.state.lock().map_err(err)?;
    serde_json::to_string(&Status {
        error: state.error.as_deref(),
        generation: state.generation,
        status: &state.status,
        subscriptions: state.subscriptions,
    })
    .map_err(err)
}

pub fn start_feed(
    app: &AppHandle,
    feed: Arc<market::Feed>,
    rx: tokio::sync::watch::Receiver<market::Desired>,
    tx: tokio::sync::mpsc::Sender<market::Input>,
    mut inputs: tokio::sync::mpsc::Receiver<market::Input>,
) {
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
                                "事件 {} {} {} {:?} {:?}",
                                event.id,
                                event.key.market,
                                event.key.symbol,
                                event.key.price_kind,
                                event.reason
                            ),
                        ) {
                            core.report(e);
                        }
                    }
                }
                Err(e) => {
                    core.report(format!("警报状态写入失败，停止行情代次：{e}"));
                    feed.desired.send_modify(|d| d.reconnect += 1);
                    feed.generation.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    });
}
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        // 每次只检查当前订阅和连接状态，跳过休眠期间错过的刷新。
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous = String::new();
        let mut last_history_cleanup: Option<std::time::Instant> = None;
        let mut prune_alert_backlog = false;
        loop {
            timer.tick().await;
            let core = handle.state::<Core>();
            if core.quitting.load(Ordering::Relaxed) {
                for feed in core.business.feeds() {
                    feed.cancel.cancel();
                }
                break;
            }
            let cleanup_due = last_history_cleanup.is_none_or(|at| at.elapsed().as_secs() >= 60);
            let now = crate::store::now_ms();
            if cleanup_due {
                last_history_cleanup = Some(std::time::Instant::now());
                if let Err(error) = crate::reminder_store::prune_island_history(&core.store, now) {
                    core.report(error);
                }
            }
            if cleanup_due || prune_alert_backlog {
                let source_now = core
                    .business
                    .feed
                    .source_now()
                    .or_else(|| core.business.spot.source_now());
                prune_alert_backlog =
                    match core
                        .reminders
                        .prune_alert_history(&core.store, now, source_now)
                    {
                        Ok(more) => more,
                        Err(error) => {
                            core.report(error);
                            false
                        }
                    };
            }
            if cleanup_due {
                if let Err(error) = core.store.reclaim_idle_wal() {
                    core.report(error);
                }
            }
            let refresh = || -> AppResult<()> {
                let sessions = core.sessions.0.lock().map_err(err)?;
                let mut subs = core.business.subscriptions.lock().map_err(err)?;
                subs.retain(|_, s| sessions.contains_key(&s.owner));
                let mut keys = persistent_keys(&core.store)?;
                for sub in subs.values() {
                    keys.extend(sub.keys.iter().cloned());
                }
                for feed in core.business.feeds() {
                    let market_keys = keys
                        .iter()
                        .filter(|key| key.market == feed.market)
                        .cloned()
                        .collect::<HashSet<_>>();
                    feed.desired.send_if_modified(|d| {
                        if d.keys == market_keys {
                            return false;
                        }
                        d.keys = market_keys;
                        true
                    });
                }
                Ok(())
            };
            if let Err(e) = refresh() {
                core.report(e);
            }
            let status = market_log_status(&core.business.feed).unwrap_or_else(|_| {
                r#"{"error":null,"generation":null,"status":null,"subscriptions":null}"#.into()
            });
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
            let core = handle.state::<Core>();
            let Some(source_now) = core
                .business
                .feed
                .source_now()
                .or_else(|| core.business.spot.source_now())
            else {
                continue;
            };
            deliver_pending(&handle, source_now).await;
        }
    });
}

pub(crate) async fn deliver_pending(handle: &AppHandle, source_now: u64) {
    let jobs = match alert_store::pending_deliveries(&handle.state::<Core>().store, source_now) {
        Ok(jobs) => jobs,
        Err(error) => {
            handle.state::<Core>().report(error);
            return;
        }
    };
    for event in jobs {
        let channel = Channel::Notification;
        match alert_store::claim_delivery(&handle.state::<Core>().store, &event.id, channel) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                handle.state::<Core>().report(e);
                continue;
            }
        }
        let app = handle.clone();
        let id = event.id.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            crate::alert_notice::deliver(&app, &event)
        })
        .await;
        let detail = format!("事件 {id} 渠道 {channel:?} 结果 {outcome:?}");
        let saved = match outcome {
            Ok(result) => {
                alert_store::finish_delivery(&handle.state::<Core>().store, &id, channel, result)
            }
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
