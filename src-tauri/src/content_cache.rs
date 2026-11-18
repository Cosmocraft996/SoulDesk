//! 仅用于固定公开内容源的进程内短缓存；行情、警报和通用HTTP接口不经过这里。
use crate::{
    network::NetworkConfig,
    store::{err, AppResult},
};
use futures_util::FutureExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_FLIGHTS: usize = 32;

pub(crate) fn lifetime(source: &str) -> Option<Duration> {
    Some(Duration::from_secs(match source {
        "bilibili" | "douyin" | "tou-tiao" | "weibo" | "zhihu" => 30,
        "cls-telegraph" => 10,
        "bangumi" => 60,
        "itime-calendar" => 3600,
        "bilibili-rcmd" => 0, // 只合并在途请求，不缓存已完成推荐。
        _ => return None,
    }))
}

fn key(
    source: &str,
    args: &Value,
    config: &NetworkConfig,
    calendar_day: &str,
) -> AppResult<String> {
    let scope = if source == "itime-calendar" {
        calendar_day
    } else {
        ""
    };
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let mut names: Vec<_> = fields.keys().collect();
                names.sort();
                Value::Object(
                    names
                        .into_iter()
                        .map(|name| (name.clone(), canonical(&fields[name])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    let bytes = serde_json::to_vec(&(source, canonical(args), config, scope)).map_err(err)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

struct Entry {
    value: Value,
    at: Instant,
    ttl: Duration,
    bytes: usize,
}
type ResultChannel = watch::Receiver<Option<AppResult<Value>>>;
#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    flights: HashMap<String, ResultChannel>,
}
#[derive(Clone, Default)]
pub(crate) struct Service(Arc<Mutex<State>>);

impl Service {
    pub(crate) async fn read<F>(
        &self,
        source: &str,
        args: &Value,
        config: &NetworkConfig,
        force: bool,
        fetch: F,
    ) -> AppResult<Value>
    where
        F: Future<Output = AppResult<Value>> + Send + 'static,
    {
        let ttl = lifetime(source).ok_or("CAPABILITY_DENIED: 此来源不使用公开内容缓存")?;
        // 日期及UTC偏移进入黄历口径，跨日/时区变化不会命中之前的结果。
        let day = chrono::Local::now().format("%Y-%m-%d%:z").to_string();
        self.read_key(key(source, args, config, &day)?, ttl, force, fetch)
            .await
    }

    async fn read_key<F>(
        &self,
        key: String,
        ttl: Duration,
        force: bool,
        fetch: F,
    ) -> AppResult<Value>
    where
        F: Future<Output = AppResult<Value>> + Send + 'static,
    {
        let mut fetch = Some(fetch);
        let receiver = {
            let mut state = self.0.lock().map_err(err)?;
            state
                .entries
                .retain(|_, entry| entry.at.elapsed() < entry.ttl);
            state
                .flights
                .retain(|_, receiver| receiver.has_changed().is_ok());
            if force {
                state.entries.remove(&key);
            }
            if let Some(receiver) = state.flights.get(&key) {
                Some(receiver.clone())
            } else if let Some(entry) = state.entries.get(&key) {
                return Ok(entry.value.clone());
            } else if state.flights.len() >= MAX_FLIGHTS {
                None // 合并记录已满时沿用直接读取，不给原有业务新增限流。
            } else {
                let (sender, receiver) = watch::channel(None);
                state.flights.insert(key.clone(), receiver.clone());
                let cache = self.clone();
                let fetch = fetch.take().unwrap();
                tokio::spawn(async move {
                    // 请求有网络层的8秒总超时；页面关闭不让其他等待者丢失同一公开读取结果。
                    let mut result =
                        AssertUnwindSafe(fetch)
                            .catch_unwind()
                            .await
                            .unwrap_or_else(|_| {
                                Err("DATA_UNAVAILABLE: 公开读取意外中断，请重试".into())
                            });
                    if let Ok(mut state) = cache.0.lock() {
                        state.flights.remove(&key);
                        if let Ok(value) = &result {
                            let bytes = serde_json::to_vec(value)
                                .map(|v| v.len())
                                .unwrap_or(MAX_BYTES + 1);
                            if !ttl.is_zero() && bytes <= MAX_BYTES {
                                while !state.entries.is_empty()
                                    && (state.entries.len() >= MAX_ENTRIES
                                        || state
                                            .entries
                                            .values()
                                            .map(|entry| entry.bytes)
                                            .sum::<usize>()
                                            + bytes
                                            > MAX_BYTES)
                                {
                                    let oldest = state
                                        .entries
                                        .iter()
                                        .min_by_key(|(_, entry)| entry.at)
                                        .map(|(key, _)| key.clone())
                                        .unwrap();
                                    state.entries.remove(&oldest);
                                }
                                state.entries.insert(
                                    key,
                                    Entry {
                                        value: value.clone(),
                                        at: Instant::now(),
                                        ttl,
                                        bytes,
                                    },
                                );
                            }
                        }
                    } else {
                        result = Err("STORAGE_ERROR: 公开内容缓存锁不可用".into());
                    }
                    // 沿用第一次真实取数的完整响应，包括source和fetchedAt；失败不保存。
                    sender.send_replace(Some(result));
                });
                Some(receiver)
            }
        };
        let Some(mut receiver) = receiver else {
            return fetch.unwrap().await;
        };
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| "DATA_UNAVAILABLE: 公开请求已中断，请重新刷新")?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn only_confirmed_public_sources_have_a_policy_and_market_paths_are_excluded() {
        for (name, seconds) in [
            ("bilibili", 30),
            ("douyin", 30),
            ("tou-tiao", 30),
            ("weibo", 30),
            ("zhihu", 30),
            ("cls-telegraph", 10),
            ("bangumi", 60),
            ("itime-calendar", 3600),
            ("bilibili-rcmd", 0),
        ] {
            assert_eq!(lifetime(name), Some(Duration::from_secs(seconds)));
        }
        for name in [
            "forex",
            "metal",
            "gold",
            "gold-trends",
            "a-index",
            "stock-search",
            "stock-search-eastmoney",
            "stock-quote",
            "stock-minute",
            "coin-index",
            "coin-currencies",
            "market.subscribe",
            "alerts.save",
            "weather",
            "ai.begin",
            "unknown",
        ] {
            assert_eq!(lifetime(name), None, "{name}误接入缓存");
        }
    }

    #[test]
    fn source_parameters_network_and_calendar_day_are_isolated() {
        let direct = NetworkConfig::Direct;
        let proxy = NetworkConfig::Http {
            host: "proxy.test".into(),
            port: 8080,
        };
        let a = key(
            "bilibili-rcmd",
            &json!({"lastShown":"av_1"}),
            &direct,
            "day1",
        )
        .unwrap();
        assert_ne!(
            a,
            key(
                "bilibili-rcmd",
                &json!({"lastShown":"av_2"}),
                &direct,
                "day1"
            )
            .unwrap()
        );
        assert_ne!(
            a,
            key(
                "bilibili-rcmd",
                &json!({"lastShown":"av_1"}),
                &proxy,
                "day1"
            )
            .unwrap()
        );
        assert_ne!(
            a,
            key("weibo", &json!({"lastShown":"av_1"}), &direct, "day1").unwrap()
        );
        assert_eq!(
            a,
            key(
                "bilibili-rcmd",
                &json!({"lastShown":"av_1"}),
                &direct,
                "day2"
            )
            .unwrap()
        );
        assert_ne!(
            key("itime-calendar", &json!({}), &direct, "day1").unwrap(),
            key("itime-calendar", &json!({}), &direct, "day2").unwrap()
        );
    }

    #[tokio::test]
    async fn hits_keep_actual_timestamp_expiry_and_manual_failure_cannot_restore_old_success() {
        let cache = Service::default();
        let ttl = Duration::from_secs(30);
        let original = json!({"data":{"list":[]},"source":"https://fixed.test/","fetchedAt":123});
        assert_eq!(
            cache
                .read_key(
                    "a".into(),
                    ttl,
                    false,
                    std::future::ready(Ok(original.clone()))
                )
                .await
                .unwrap(),
            original
        );
        assert_eq!(
            cache
                .read_key("a".into(), ttl, false, async {
                    panic!("缓存命中不请求来源")
                })
                .await
                .unwrap(),
            original
        );
        let error = cache
            .read_key("a".into(), ttl, true, async {
                Err("DATA_UNAVAILABLE: HTTP 503".into())
            })
            .await
            .unwrap_err();
        assert!(error.contains("503"));
        assert!(cache
            .read_key("a".into(), ttl, false, async { Err("仍然离线".into()) })
            .await
            .is_err());
        cache
            .read_key(
                "a".into(),
                ttl,
                false,
                std::future::ready(Ok(original.clone())),
            )
            .await
            .unwrap();
        cache.0.lock().unwrap().entries.get_mut("a").unwrap().at =
            Instant::now() - Duration::from_secs(31);
        let next = json!({"fetchedAt":456});
        assert_eq!(
            cache
                .read_key("a".into(), ttl, false, std::future::ready(Ok(next.clone())))
                .await
                .unwrap(),
            next
        );
    }

    #[tokio::test]
    async fn concurrent_consumers_share_a_request_even_when_original_consumer_leaves() {
        let cache = Service::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let (done, wait) = tokio::sync::oneshot::channel();
        let first_cache = cache.clone();
        let count = calls.clone();
        let first = tokio::spawn(async move {
            first_cache
                .read_key("a".into(), Duration::from_secs(30), false, async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    wait.await.unwrap();
                    Ok(json!({"fetchedAt":1}))
                })
                .await
        });
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        first.abort();
        let second = cache.read_key("a".into(), Duration::from_secs(30), true, async {
            panic!("在途请求应共用")
        });
        let result = async {
            tokio::task::yield_now().await;
            done.send(()).unwrap();
        };
        let (value, _) = tokio::join!(second, result);
        assert_eq!(value.unwrap()["fetchedAt"], 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn recommendations_only_share_inflight_and_cache_does_not_survive_a_new_service() {
        let cache = Service::default();
        for n in 1..=2 {
            assert_eq!(
                cache
                    .read_key(
                        "r".into(),
                        Duration::ZERO,
                        false,
                        std::future::ready(Ok(json!(n)))
                    )
                    .await
                    .unwrap(),
                n
            );
        }
        assert!(cache.0.lock().unwrap().entries.is_empty());
        cache
            .read_key("a".into(), Duration::from_secs(30), false, async {
                Ok(json!(1))
            })
            .await
            .unwrap();
        assert_eq!(
            Service::default()
                .read_key("a".into(), Duration::from_secs(30), false, async {
                    Ok(json!(2))
                })
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn bounded_entries_and_interrupted_reads_leave_room_for_retries() {
        let cache = Service::default();
        for n in 0..MAX_ENTRIES + 2 {
            cache
                .read_key(n.to_string(), Duration::from_secs(30), false, async {
                    Ok(json!(1))
                })
                .await
                .unwrap();
        }
        assert_eq!(cache.0.lock().unwrap().entries.len(), MAX_ENTRIES);
        assert!(cache
            .read_key("panic".into(), Duration::from_secs(30), false, async {
                panic!("模拟读取中断")
            })
            .await
            .is_err());
        assert_eq!(
            cache
                .read_key("panic".into(), Duration::from_secs(30), false, async {
                    Ok(json!(2))
                })
                .await
                .unwrap(),
            2
        );
        for n in 0..MAX_ENTRIES {
            cache
                .read_key(
                    format!("large-{n}"),
                    Duration::from_secs(30),
                    false,
                    async { Ok(Value::String("x".repeat(350000))) },
                )
                .await
                .unwrap();
        }
        assert!(
            cache
                .0
                .lock()
                .unwrap()
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                <= MAX_BYTES
        );
    }

    #[tokio::test]
    async fn quote_sources_cannot_poll_a_cache_fetch_and_a_new_calendar_day_fetches_again() {
        let cache = Service::default();
        let config = NetworkConfig::Direct;
        for source in [
            "gold",
            "metal",
            "forex",
            "stock-quote",
            "stock-minute",
            "a-index",
            "coin-index",
            "coin-currencies",
            "market.subscribe",
            "alerts.save",
        ] {
            assert!(cache
                .read(source, &Value::Null, &config, false, async {
                    panic!("行情与警报不应访问此缓存")
                })
                .await
                .is_err());
        }
        for (day, value) in [("2026-11-18+08:00", 1), ("2026-11-19+08:00", 2)] {
            let key = key("itime-calendar", &Value::Null, &config, day).unwrap();
            assert_eq!(
                cache
                    .read_key(
                        key,
                        Duration::from_secs(3600),
                        false,
                        std::future::ready(Ok(Value::from(value)))
                    )
                    .await
                    .unwrap(),
                value
            );
        }
    }

    #[tokio::test]
    async fn a_full_coalescing_budget_keeps_reading_instead_of_rejecting_existing_business() {
        let cache = Service::default();
        let mut senders = Vec::new();
        for n in 0..MAX_FLIGHTS {
            let (sender, receiver) = watch::channel(None);
            senders.push(sender);
            cache
                .0
                .lock()
                .unwrap()
                .flights
                .insert(n.to_string(), receiver);
        }
        assert_eq!(
            cache
                .read_key("overflow".into(), Duration::from_secs(30), false, async {
                    Ok(Value::from(42))
                })
                .await
                .unwrap(),
            42
        );
        assert_eq!(cache.0.lock().unwrap().flights.len(), MAX_FLIGHTS);
        assert!(!cache.0.lock().unwrap().entries.contains_key("overflow"));
    }
}
