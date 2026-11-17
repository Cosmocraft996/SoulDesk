//! 原OKX指数组件的公开数据与按实例代理设置。
use crate::{
    network::NetworkConfig,
    store::{err, AppResult},
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
pub struct IndexCache(Mutex<HashMap<String, CachedIndex>>);
struct CachedIndex {
    key: String,
    response: Value,
    samples: Vec<Value>,
}
impl IndexCache {
    pub fn peek(&self, instance: &str, key: &str) -> AppResult<Value> {
        let entries = self.0.lock().map_err(err)?;
        let Some(entry) = entries.get(instance).filter(|entry| entry.key == key) else {
            return Ok(Value::Null);
        };
        let mut response = entry.response.clone();
        response["samples"] = json!(entry.samples);
        response["cached"] = json!(true);
        Ok(response)
    }
    pub fn remember(&self, instance: &str, key: String, response: Value) -> AppResult<()> {
        let timestamp = response["data"]["timestamp"]
            .as_u64()
            .ok_or("DATA_UNAVAILABLE: 缓存指数时间缺失")?;
        let mut entries = self.0.lock().map_err(err)?;
        // 每实例只保留当前口径，最多128实例；临时展示状态不写入SQLite。
        if !entries.contains_key(instance) && entries.len() >= 128 {
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.response["fetchedAt"].as_u64().unwrap_or(0))
                .map(|(id, _)| id.clone())
            {
                entries.remove(&oldest);
            }
        }
        let entry = entries
            .entry(instance.into())
            .or_insert_with(|| CachedIndex {
                key: key.clone(),
                response: Value::Null,
                samples: Vec::new(),
            });
        if entry.key != key {
            entry.key = key;
            entry.samples.clear();
            entry.response = Value::Null;
        }
        if entry.response["data"]["timestamp"]
            .as_u64()
            .is_some_and(|old| old > timestamp)
        {
            return Ok(());
        }
        let sample = json!({"timestamp":timestamp,"price":response["data"]["price"]});
        if entry
            .samples
            .last()
            .is_some_and(|last| last["timestamp"] == timestamp)
        {
            *entry.samples.last_mut().unwrap() = sample;
        } else {
            entry.samples.push(sample);
            if entry.samples.len() > 48 {
                entry.samples.remove(0);
            }
        }
        entry.response = response;
        Ok(())
    }
    pub fn forget(&self, instance: &str) -> AppResult<()> {
        self.0.lock().map_err(err)?.remove(instance);
        Ok(())
    }
}
pub fn index_url(instrument: &str) -> AppResult<String> {
    let coins: Vec<Value> = serde_json::from_str(include_str!(
        "../../components/widgetjs/coin-common/coins.json"
    ))
    .map_err(err)?;
    if !coins.iter().any(|value| value["type"] == instrument) {
        return Err("INVALID_ARGUMENT: 不在原币种目录中".into());
    }
    let mut url =
        url::Url::parse("https://www.okx.com/api/v5/market/index-tickers").map_err(err)?;
    url.query_pairs_mut().append_pair("instId", instrument);
    Ok(url.to_string())
}
pub(crate) fn positive(value: &Value) -> AppResult<Option<String>> {
    if value.is_null() || value == "" {
        return Ok(None);
    }
    let value = value
        .as_str()
        .filter(|value| value.len() <= 96)
        .ok_or("DATA_UNAVAILABLE: 指数价格必须为十进制字符串")?;
    let price = rust_decimal::Decimal::from_str_exact(value)
        .map_err(|_| "DATA_UNAVAILABLE: 指数价格无效或超出精度")?;
    if price <= rust_decimal::Decimal::ZERO {
        return Ok(None);
    }
    Ok(Some(value.into()))
}
pub fn parse_index(bytes: &[u8], instrument: &str) -> AppResult<Value> {
    let response: Value =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: OKX指数格式无效")?;
    if response["code"] != "0" {
        return Err("DATA_UNAVAILABLE: OKX指数源返回失败".into());
    }
    let data = response["data"]
        .as_array()
        .filter(|values| values.len() <= 100)
        .and_then(|values| values.iter().find(|value| value["instId"] == instrument))
        .ok_or("DATA_UNAVAILABLE: OKX未提供指定指数")?;
    let price = positive(&data["idxPx"])?.ok_or("DATA_UNAVAILABLE: OKX指数价格缺失")?;
    let (reference, basis) = if let Some(price) = positive(&data["open24h"])? {
        (Some(price), Some("open24h"))
    } else if let Some(price) = positive(&data["sodUtc8"])? {
        (Some(price), Some("sodUtc8"))
    } else {
        (None, None)
    };
    let timestamp = data["ts"]
        .as_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= 16 && value.bytes().all(|c| c.is_ascii_digit())
        })
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0 && *value <= 8_640_000_000_000_000)
        .ok_or("DATA_UNAVAILABLE: OKX指数时间无效")?;
    Ok(
        json!({"instrument":instrument,"price":price,"reference":reference,"basis":basis,"timestamp":timestamp}),
    )
}
pub fn instance_network(settings: &Value, inherited: NetworkConfig) -> AppResult<NetworkConfig> {
    let mode = settings["proxyMode"].as_str().unwrap_or("inherit");
    if mode == "inherit" {
        return Ok(inherited);
    }
    if mode == "direct" {
        return Ok(NetworkConfig::Direct);
    }
    let host = settings["proxyHost"]
        .as_str()
        .ok_or("INVALID_ARGUMENT: 代理主机缺失")?
        .to_owned();
    let port = settings["proxyPort"]
        .as_u64()
        .filter(|value| *value > 0 && *value <= 65535)
        .ok_or("INVALID_ARGUMENT: 代理端口无效")? as u16;
    let config = match mode {
        "http" => NetworkConfig::Http { host, port },
        "socks5" => NetworkConfig::Socks5 { host, port },
        _ => return Err("INVALID_ARGUMENT: 代理模式无效".into()),
    };
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cached_response(timestamp: u64) -> Value {
        json!({"data":{"instrument":"BTC-USD","price":"100.25","timestamp":timestamp},"source":"https://www.okx.com/api/v5/market/index-tickers?instId=BTC-USD","fetchedAt":timestamp+10})
    }
    #[test]
    fn index_view_cache_keeps_real_samples_and_original_times() {
        let cache = IndexCache::default();
        for time in [100, 200, 200, 150] {
            cache
                .remember("first", "BTC-direct".into(), cached_response(time))
                .unwrap();
        }
        let value = cache.peek("first", "BTC-direct").unwrap();
        assert_eq!(value["data"]["timestamp"], 200);
        assert_eq!(value["fetchedAt"], 210);
        assert_eq!(value["samples"].as_array().unwrap().len(), 2);
        assert_eq!(value["samples"][0]["timestamp"], 100);
        assert_eq!(cache.peek("first", "BTC-direct").unwrap(), value);
    }
    #[test]
    fn index_view_cache_is_bound_to_instance_and_network_instrument_context() {
        let cache = IndexCache::default();
        cache
            .remember("first", "BTC-direct".into(), cached_response(100))
            .unwrap();
        assert!(cache.peek("second", "BTC-direct").unwrap().is_null());
        assert!(cache.peek("first", "BTC-http").unwrap().is_null());
        assert!(cache.peek("first", "ETH-direct").unwrap().is_null());
        cache
            .remember("first", "ETH-direct".into(), cached_response(200))
            .unwrap();
        assert_eq!(
            cache.peek("first", "ETH-direct").unwrap()["samples"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        cache.forget("first").unwrap();
        assert!(cache.peek("first", "ETH-direct").unwrap().is_null());
    }
    #[test]
    fn index_view_cache_has_bounded_instances_and_sample_count() {
        let cache = IndexCache::default();
        for id in 0..129 {
            cache
                .remember(&id.to_string(), "key".into(), cached_response(id + 1))
                .unwrap();
        }
        assert_eq!(cache.0.lock().unwrap().len(), 128);
        assert!(cache.peek("0", "key").unwrap().is_null());
        for time in 200..260 {
            cache
                .remember("128", "key".into(), cached_response(time))
                .unwrap();
        }
        let value = cache.peek("128", "key").unwrap();
        assert_eq!(value["samples"].as_array().unwrap().len(), 48);
        assert_eq!(value["samples"][0]["timestamp"], 212);
    }

    #[test]
    fn only_original_index_identifiers_are_allowed_and_quotes_keep_source_time() {
        assert!(index_url("BTC-USD").unwrap().ends_with("instId=BTC-USD"));
        assert!(index_url("BTC-USD&url=x").is_err());
        assert!(index_url("BTC-USDT").is_err());
        let body=br#"{"code":"0","data":[{"instId":"BTC-USD","idxPx":"12345678901234567890.125","open24h":"12345678901234567890.12","sodUtc8":"1","ts":"1794657600000"}]}"#;
        let value = parse_index(body, "BTC-USD").unwrap();
        assert_eq!(value["price"], "12345678901234567890.125");
        assert_eq!(value["basis"], "open24h");
        assert_eq!(value["timestamp"], 1794657600000_u64);
        assert!(parse_index(body, "ETH-USD").is_err());
        let fallback = String::from_utf8(body.to_vec()).unwrap().replace(
            "\"open24h\":\"12345678901234567890.12\"",
            "\"open24h\":\"\"",
        );
        assert_eq!(
            parse_index(fallback.as_bytes(), "BTC-USD").unwrap()["basis"],
            "sodUtc8"
        );
        assert!(parse_index(br#"{"code":"1","data":[]}"#, "BTC-USD").is_err());
    }

    #[test]
    fn component_proxy_can_inherit_or_override_without_changing_global_config() {
        let global = crate::network::NetworkConfig::Http {
            host: "127.0.0.1".into(),
            port: 9000,
        };
        assert_eq!(
            instance_network(&json!({"proxyMode":"inherit"}), global.clone()).unwrap(),
            global
        );
        assert_eq!(
            instance_network(&json!({"proxyMode":"direct"}), global.clone()).unwrap(),
            crate::network::NetworkConfig::Direct
        );
        assert_eq!(
            instance_network(
                &json!({"proxyMode":"socks5","proxyHost":"127.0.0.1","proxyPort":1080}),
                global.clone()
            )
            .unwrap(),
            crate::network::NetworkConfig::Socks5 {
                host: "127.0.0.1".into(),
                port: 1080
            }
        );
        for value in [
            json!({"proxyMode":"http","proxyHost":"x@y","proxyPort":1080}),
            json!({"proxyMode":"http","proxyHost":"localhost","proxyPort":0}),
            json!({"proxyMode":"other"}),
        ] {
            assert!(instance_network(&value, global.clone()).is_err());
        }
    }

    /// 显式网络验收，不纳入日常离线检查，也不触碰正常用户数据库。
    #[tokio::test]
    #[ignore = "需要显式执行真实OKX公开接口检查"]
    async fn real_okx_index_uses_native_bounded_reader_and_checks_fresh_source_time() {
        let config = crate::network::system_proxy().unwrap();
        let source = index_url("BTC-USD").unwrap();
        let bytes = crate::network::http_get_bounded(&config, &source, 2 * 1024 * 1024)
            .await
            .unwrap();
        let value = parse_index(&bytes, "BTC-USD").unwrap();
        let timestamp = value["timestamp"].as_u64().unwrap();
        let now = crate::store::now_ms();
        assert!(
            timestamp <= now + 5000 && now.saturating_sub(timestamp) < 120000,
            "原生读取收到旧时间，未判实时成功"
        );
        let report = json!({"source":source,"scope":"原生有限HTTP读取和精确解析；不等于WebView/代理设置/全部币种验收","systemProxy":true,"timestamp":timestamp,"checkedAt":now,"instrument":value["instrument"],"price":value["price"],"fresh":true,"normalDataOpened":false});
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts/widgetjs-okx-native-probe.json");
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
