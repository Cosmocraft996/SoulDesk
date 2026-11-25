//! 用原聚合成交序列补查异常重启区间；无法证明连续覆盖时记录缺口。
use crate::{
    alerts::{self, MarketKey, Observation, PriceKind},
    market::Input,
    network::{self, NetworkConfig},
    reminder_store,
    store::{err, AppResult, Store},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

const BOOT_KEY: &str = "market.recovery.boot";
const MAX_CATCHUP_MS: u64 = 90_000;
const MAX_AGG_TRADES: u64 = 1_000;
const FETCH_BUDGET: Duration = Duration::from_secs(2);
const MAX_FETCHES: u8 = 4;

pub(crate) type RuleVersions = Vec<(String, u64)>;

pub(crate) fn install(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS market_checkpoints(key TEXT PRIMARY KEY, checkpoint TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS market_monitor_gaps(
           epoch INTEGER NOT NULL, key TEXT NOT NULL, started INTEGER, ended INTEGER NOT NULL,
           reason TEXT NOT NULL, PRIMARY KEY(epoch,key)
         );",
    ).map_err(err)
}

#[derive(Default, Deserialize, Serialize)]
struct Boot {
    epoch: u64,
    clean_through: u64,
}

pub(crate) struct Session {
    boot: Boot,
    budget: Mutex<Option<(Instant, u8)>>,
    eligible: HashSet<MarketKey>,
    fixture: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Checkpoint {
    epoch: u64,
    pub(crate) time: u64,
    pub(crate) price: String,
    id: Option<u64>,
    pub(crate) rules: RuleVersions,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Trade {
    #[serde(rename = "a")]
    id: u64,
    #[serde(rename = "T")]
    pub(crate) time: u64,
    #[serde(rename = "p")]
    pub(crate) price: String,
}

pub(crate) enum Coverage {
    Verified {
        baseline: Checkpoint,
        trades: Vec<Trade>,
    },
    Gap {
        from: Option<u64>,
        reason: &'static str,
    },
}

pub(crate) struct Progress {
    pub(crate) checkpoint: Checkpoint,
    pub(crate) save_checkpoint: bool,
    pub(crate) coverage: Option<Coverage>,
}

impl Session {
    pub(crate) fn begin(store: &Store, isolated_check: bool) -> AppResult<Self> {
        let eligible = crate::business::rule_keys(store)?;
        let mut db = store.db.lock().map_err(err)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut boot: Boot = reminder_store::preference(&tx, BOOT_KEY)?
            .map(|raw| serde_json::from_str(&raw).map_err(err))
            .transpose()?
            .unwrap_or_default();
        boot.epoch = boot.epoch.checked_add(1).ok_or("行情运行代次溢出")?;
        save_boot(&tx, &boot)?;
        tx.commit().map_err(err)?;
        let fixture =
            isolated_check.then(|| store.root.join("market-runtime/recovery-trades.json"));
        Ok(Self {
            boot,
            budget: Mutex::new(None),
            eligible,
            fixture,
        })
    }

    pub(crate) fn finish(&self, store: &Store) -> AppResult<()> {
        let db = store.db.lock().map_err(err)?;
        save_boot(
            &db,
            &Boot {
                epoch: self.boot.epoch,
                clean_through: self.boot.epoch,
            },
        )
    }

    pub(crate) async fn prepare(
        &self,
        store: &Store,
        input: &Input,
        first: bool,
        save: bool,
        network: &NetworkConfig,
    ) -> AppResult<Progress> {
        let quote = &input.observation;
        let mut progress = Progress {
            checkpoint: Checkpoint {
                epoch: self.boot.epoch,
                time: quote.source_utc_ms,
                price: quote.price.clone(),
                id: input.trade_id,
                rules: Vec::new(),
            },
            save_checkpoint: first || save,
            coverage: None,
        };
        if !first
            || !self.eligible.contains(&quote.key)
            || self.boot.epoch <= self.boot.clean_through.saturating_add(1)
        {
            return Ok(progress);
        }
        let baseline = read_checkpoint(store, &quote.key)?
            .filter(|prior| prior.epoch > self.boot.clean_through && prior.epoch < self.boot.epoch);
        progress.coverage = Some(match baseline {
            None => Coverage::Gap {
                from: None,
                reason: "缺少可验证的重启前行情断点",
            },
            Some(baseline) => match self.trades(&baseline, input, network).await {
                Ok(trades) => Coverage::Verified { baseline, trades },
                Err(reason) => Coverage::Gap {
                    from: Some(baseline.time.min(quote.source_utc_ms)),
                    reason,
                },
            },
        });
        Ok(progress)
    }

    async fn trades(
        &self,
        baseline: &Checkpoint,
        input: &Input,
        network_config: &NetworkConfig,
    ) -> Result<Vec<Trade>, &'static str> {
        let quote = &input.observation;
        if quote.key.price_kind != PriceKind::Last {
            return Err("原标记价源没有可验证触发时刻的逐笔历史");
        }
        let (Some(start), Some(end)) = (baseline.id, input.trade_id) else {
            return Err("原行情缺少聚合成交编号");
        };
        let Some(count) = end.checked_sub(start) else {
            return Err("实时聚合成交编号早于断点");
        };
        if count == 0
            || count > MAX_AGG_TRADES
            || quote
                .source_utc_ms
                .checked_sub(baseline.time)
                .is_none_or(|age| age > MAX_CATCHUP_MS)
        {
            return Err("原行情超出短时补查范围");
        }
        if count == 1 {
            return Ok(Vec::new());
        }
        let url = trade_url(&quote.key, start, count).map_err(|_| "原行情地址无效")?;
        let bytes = if let Some(path) = &self.fixture {
            std::fs::read(path).map_err(|_| "隔离验收缺少原行情响应夹具")?
        } else {
            let deadline = self.fetch_deadline()?;
            tokio::time::timeout_at(
                deadline.into(),
                network::http_get_bounded(network_config, url.as_str(), 512 * 1024),
            )
            .await
            .map_err(|_| "原行情补查超时")?
            .map_err(|_| "本次原行情补查不可用")?
        };
        verify_trades(&bytes, baseline, quote, end, count)
            .map_err(|_| "原聚合成交编号、价格或时间无法连续核实")
    }

    fn fetch_deadline(&self) -> Result<Instant, &'static str> {
        let mut budget = self.budget.lock().map_err(|_| "无法安排原行情补查")?;
        let (deadline, count) = budget.get_or_insert_with(|| (Instant::now() + FETCH_BUDGET, 0));
        if *count >= MAX_FETCHES || Instant::now() >= *deadline {
            return Err("本轮原行情补查额度已用完");
        }
        *count += 1;
        Ok(*deadline)
    }
}

fn trade_url(key: &MarketKey, from: u64, count: u64) -> AppResult<url::Url> {
    key.validate()?;
    let base = if key.market == "spot" {
        "https://data-api.binance.vision/api/v3/aggTrades"
    } else {
        "https://fapi.binance.com/fapi/v1/aggTrades"
    };
    let mut url = url::Url::parse(base).map_err(err)?;
    url.query_pairs_mut()
        .append_pair("symbol", &key.symbol)
        .append_pair("fromId", &from.to_string())
        .append_pair("limit", &count.to_string());
    Ok(url)
}

fn verify_trades(
    bytes: &[u8],
    baseline: &Checkpoint,
    quote: &Observation,
    end: u64,
    count: u64,
) -> AppResult<Vec<Trade>> {
    let mut trades: Vec<Trade> = serde_json::from_slice(bytes).map_err(err)?;
    if trades.len() as u64 != count {
        return Err("返回笔数与断点编号范围不同".into());
    }
    let mut expected = baseline.id.ok_or("缺少聚合成交编号")?;
    let mut last = baseline.time;
    for trade in &trades {
        if trade.id != expected || trade.time < last || trade.time > quote.source_utc_ms {
            return Err("聚合成交编号或时间不连续".into());
        }
        alerts::parse_price(&trade.price)?;
        last = trade.time;
        expected += 1;
    }
    let first = trades.first().ok_or("缺少已知断点")?;
    if first.time != baseline.time
        || alerts::parse_price(&first.price)? != alerts::parse_price(&baseline.price)?
        || expected != end
    {
        return Err("原行情未能与已确认的断点及新实时报价衔接".into());
    }
    trades.remove(0);
    Ok(trades)
}

fn save_boot(db: &Connection, boot: &Boot) -> AppResult<()> {
    db.execute(
        "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![BOOT_KEY, serde_json::to_string(boot).map_err(err)?],
    ).map_err(err)?;
    Ok(())
}

pub(crate) fn read_checkpoint(store: &Store, key: &MarketKey) -> AppResult<Option<Checkpoint>> {
    let db = store.db.lock().map_err(err)?;
    let raw: Option<String> = db
        .query_row(
            "SELECT checkpoint FROM market_checkpoints WHERE key=?1",
            [serde_json::to_string(key).map_err(err)?],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(err))
        .transpose()
}

pub(crate) fn save_checkpoint(
    db: &Connection,
    key: &str,
    checkpoint: &Checkpoint,
) -> AppResult<()> {
    db.execute(
        "INSERT INTO market_checkpoints(key,checkpoint) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET checkpoint=excluded.checkpoint",
        params![key, serde_json::to_string(checkpoint).map_err(err)?],
    ).map_err(err)?;
    Ok(())
}

pub(crate) fn save_gap(
    db: &Connection,
    key: &str,
    checkpoint: &Checkpoint,
    from: Option<u64>,
    reason: &str,
) -> AppResult<()> {
    db.execute(
        "INSERT OR IGNORE INTO market_monitor_gaps(epoch,key,started,ended,reason) VALUES(?1,?2,?3,?4,?5)",
        params![checkpoint.epoch,key,from,checkpoint.time,reason],
    ).map_err(err)?;
    db.execute(
        "DELETE FROM market_monitor_gaps WHERE rowid IN (SELECT rowid FROM market_monitor_gaps ORDER BY ended DESC,rowid DESC LIMIT -1 OFFSET 200)",
        [],
    ).map_err(err)?;
    Ok(())
}

pub(crate) fn history(store: &Store) -> AppResult<Vec<Value>> {
    let db = store.db.lock().map_err(err)?;
    let mut query = db.prepare(
        "SELECT key,started,ended,reason FROM market_monitor_gaps ORDER BY ended DESC,rowid DESC LIMIT 30",
    ).map_err(err)?;
    let values = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<u64>>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(err)?;
    values
        .map(|value| {
            let (key, from, to, reason) = value.map_err(err)?;
            let key: MarketKey = serde_json::from_str(&key).map_err(err)?;
            Ok(json!({"key":key,"fromUtcMs":from,"toUtcMs":to,"reason":reason}))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_exact_source_boundary_and_complete_trade_sequence() {
        let baseline = Checkpoint {
            epoch: 1,
            time: 1000,
            price: "99.0".into(),
            id: Some(10),
            rules: Vec::new(),
        };
        let next = Observation {
            key: MarketKey::spot("BTCUSDT"),
            price: "99".into(),
            source_utc_ms: 2000,
            received_mono_ms: 100,
            generation: 1,
            ordinal: 1,
            recovered: true,
            valid: true,
        };
        let valid = br#"[{"a":10,"p":"99","T":1000},{"a":11,"p":"100.001","T":1500},{"a":12,"p":"99","T":1600}]"#;
        let trades = verify_trades(valid, &baseline, &next, 13, 3).unwrap();
        assert_eq!(trades[0].price, "100.001");
        assert_eq!(trades[0].time, 1500);
        assert_eq!(trades.len(), 2);
        for invalid in [
            br#"[{"a":10,"p":"99","T":1000},{"a":12,"p":"100.001","T":1500},{"a":13,"p":"99","T":1600}]"#.as_slice(),
            br#"[{"a":10,"p":"98","T":1000},{"a":11,"p":"100.001","T":1500},{"a":12,"p":"99","T":1600}]"#,
            br#"[{"a":10,"p":"99","T":1000},{"a":11,"p":"100.001","T":999},{"a":12,"p":"99","T":1600}]"#,
            br#"[{"a":10,"p":"99","T":1000},{"a":11,"p":"100.001","T":1500}]"#,
        ] {
            assert!(verify_trades(invalid, &baseline, &next, 13, 3).is_err());
        }
    }
}
