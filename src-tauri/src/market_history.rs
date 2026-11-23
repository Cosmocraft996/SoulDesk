//! 币安公开历史行情。请求只接受已登记的标的身份和受限时间参数。
use crate::{
    alerts::{parse_price, MarketKey, PriceKind},
    network::{self, NetworkConfig},
    store::{err, AppResult},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub key: MarketKey,
    interval: String,
    limit: u16,
    end_time: Option<u64>,
}

impl Request {
    fn url(&self, interest: bool) -> AppResult<url::Url> {
        self.key.validate()?;
        let valid = if interest {
            self.key.market == "usd_m"
                && self.key.price_kind == PriceKind::Last
                && matches!(
                    self.interval.as_str(),
                    "5m" | "15m" | "30m" | "1h" | "2h" | "4h" | "6h" | "12h" | "1d"
                )
        } else {
            matches!(
                self.interval.as_str(),
                "1m" | "3m"
                    | "5m"
                    | "15m"
                    | "30m"
                    | "1h"
                    | "2h"
                    | "4h"
                    | "6h"
                    | "8h"
                    | "12h"
                    | "1d"
                    | "3d"
                    | "1w"
            )
        };
        if !valid
            || !(1..=500).contains(&self.limit)
            || self.end_time.is_some_and(|time| {
                time == 0 || time > crate::store::now_ms().saturating_add(60_000)
            })
        {
            return Err("INVALID_ARGUMENT: 图表周期、数量或时间无效".into());
        }
        let address = match (interest, self.key.market.as_str(), self.key.price_kind) {
            (true, _, _) => "https://fapi.binance.com/futures/data/openInterestHist",
            (false, "spot", _) => "https://data-api.binance.vision/api/v3/klines",
            (false, _, PriceKind::Mark) => "https://fapi.binance.com/fapi/v1/markPriceKlines",
            (false, _, _) => "https://fapi.binance.com/fapi/v1/klines",
        };
        let mut url = url::Url::parse(address).map_err(err)?;
        let mut query = url.query_pairs_mut();
        query
            .append_pair("symbol", &self.key.symbol)
            .append_pair(if interest { "period" } else { "interval" }, &self.interval)
            .append_pair("limit", &self.limit.to_string());
        if let Some(time) = self.end_time {
            query.append_pair("endTime", &time.to_string());
        }
        drop(query);
        Ok(url)
    }
}

type Sample = Arc<Mutex<Option<(Instant, Value)>>>;
pub struct History {
    samples: Mutex<HashMap<(String, url::Url), Sample>>,
    slots: Semaphore,
}
impl Default for History {
    fn default() -> Self {
        Self {
            samples: Default::default(),
            slots: Semaphore::new(8),
        }
    }
}

impl History {
    pub async fn get(
        &self,
        args: Request,
        config: &NetworkConfig,
        interest: bool,
    ) -> AppResult<Value> {
        let url = args.url(interest)?;
        let key = (serde_json::to_string(config).map_err(err)?, url.clone());
        let sample = {
            let mut cache = self.samples.lock().await;
            if cache.len() >= 256 && !cache.contains_key(&key) {
                cache.retain(|_, item| {
                    item.try_lock().map_or(true, |entry| {
                        entry
                            .as_ref()
                            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(60))
                    }) || Arc::strong_count(item) > 1
                });
            }
            if cache.len() >= 256 && !cache.contains_key(&key) {
                return Err("BUSY: 图表请求较多，请稍后重试".into());
            }
            cache.entry(key).or_default().clone()
        };
        tokio::time::timeout(Duration::from_secs(9), async {
            let mut entry = sample.lock().await;
            let ttl = if interest { 30 } else { 8 };
            if let Some((_, value)) = entry
                .as_ref()
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(ttl))
            {
                return Ok(value.clone());
            }
            let _permit = self.slots.acquire().await.map_err(err)?;
            let bytes = network::http_get_bounded(config, url.as_str(), 512 * 1024).await?;
            let value = parse(
                &args,
                interest,
                serde_json::from_slice(&bytes).map_err(err)?,
            )?;
            *entry = Some((Instant::now(), value.clone()));
            Ok(value)
        })
        .await
        .map_err(|_| "DATA_UNAVAILABLE: 图表请求超时".to_owned())?
    }
}

fn decimal(field: &Value, zero_ok: bool) -> AppResult<&str> {
    let text = field.as_str().ok_or("PROTOCOL_CHANGED: 历史数值缺失")?;
    if zero_ok
        && text.len() <= 64
        && text.split('.').count() <= 2
        && text
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|c| c == b'0'))
    {
        return Ok(text);
    }
    parse_price(text)?;
    Ok(text)
}

fn parse(args: &Request, interest: bool, response: Value) -> AppResult<Value> {
    let rows = response
        .as_array()
        .filter(|rows| rows.len() <= args.limit as usize)
        .ok_or("PROTOCOL_CHANGED: 历史返回格式异常")?;
    let mut points = Vec::with_capacity(rows.len());
    let mut previous = 0;
    for row in rows {
        let time = (if interest {
            row["timestamp"].as_u64()
        } else {
            row[0].as_u64()
        })
        .filter(|time| *time > previous)
        .ok_or("PROTOCOL_CHANGED: 历史时间无效或重复")?;
        previous = time;
        if interest {
            if row["symbol"] != args.key.symbol {
                return Err("WRONG_MARKET: 未平仓量标的身份不匹配".into());
            }
            points.push(json!({"time":time,"value":decimal(&row["sumOpenInterest"],true)?}));
        } else {
            let open = decimal(&row[1], false)?;
            let high = decimal(&row[2], false)?;
            let low = decimal(&row[3], false)?;
            let close = decimal(&row[4], false)?;
            let close_time = row[6]
                .as_u64()
                .filter(|close| *close >= time)
                .ok_or("PROTOCOL_CHANGED: K线收盘时间无效")?;
            let volume = if args.key.price_kind == PriceKind::Last {
                Some(decimal(&row[5], true)?)
            } else {
                None
            };
            if parse_price(high)? < parse_price(open)?.max(parse_price(close)?)
                || parse_price(low)? > parse_price(open)?.min(parse_price(close)?)
            {
                return Err("PROTOCOL_CHANGED: K线高低价无效".into());
            }
            points.push(json!({"time":time,"closeTime":close_time,"open":open,"high":high,"low":low,"close":close,"volume":volume}));
        }
    }
    Ok(json!({"key":args.key,"interval":args.interval,"rows":points}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_and_payloads_keep_price_and_market_identity() {
        let mut request = Request {
            key: MarketKey::spot("BTCUSDT"),
            interval: "5m".into(),
            limit: 2,
            end_time: None,
        };
        assert_eq!(request.url(false).unwrap().path(), "/api/v3/klines");
        assert!(request.url(true).is_err());
        let rows = json!([[100, "1", "3", "1", "2", "0.000", 199]]);
        assert_eq!(
            parse(&request, false, rows.clone()).unwrap()["rows"][0]["volume"],
            "0.000"
        );
        request.key = MarketKey::binance("BTCUSDT", PriceKind::Mark);
        assert_eq!(
            request.url(false).unwrap().path(),
            "/fapi/v1/markPriceKlines"
        );
        assert!(parse(&request, false, rows).unwrap()["rows"][0]["volume"].is_null());
        assert!(parse(
            &request,
            true,
            json!([{"symbol":"ETHUSDT","timestamp":100,"sumOpenInterest":"10"}])
        )
        .is_err());
    }
}
