//! 和风天气固定协议与按实例保护的凭据；不提供通用HTTP。
use crate::{
    business, network,
    store::{err, now_ms, AppResult, Store},
    widgets, Core,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use tauri::{AppHandle, Manager};

const LIMIT: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Now,
    Daily,
    Indices,
    Lookup,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    kind: Kind,
    location: Option<String>,
    query: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    host: String,
    key: String,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Configure {
    api_host: String,
    api_key: Option<String>,
    enabled: bool,
}

fn api_host(input: &str) -> AppResult<String> {
    let input = input.trim();
    let url = url::Url::parse(&if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    })
    .map_err(|_| "INVALID_ARGUMENT: API Host无效")?;
    let host = url.host_str().ok_or("INVALID_ARGUMENT: API Host缺失")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !host.ends_with(".qweatherapi.com")
        || host.len() > 253
        || host.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return Err(
            "INVALID_ARGUMENT: 请使用自己的和风天气专属HTTPS API Host，不含路径、端口或凭据".into(),
        );
    }
    Ok(host.into())
}
fn location(value: &str) -> AppResult<()> {
    if (2..=32).contains(&value.len()) && value.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Ok(());
    }
    if let Some((lon, lat)) = value.split_once(',') {
        let valid = |value: &str, limit: f64| {
            value.len() <= 16
                && value
                    .parse::<f64>()
                    .is_ok_and(|n| n.is_finite() && n.abs() <= limit)
                && value
                    .bytes()
                    .all(|c| c.is_ascii_digit() || c == b'.' || c == b'-')
        };
        if valid(lon, 180.) && valid(lat, 90.) {
            return Ok(());
        }
    }
    Err("INVALID_ARGUMENT: 城市标识或经纬度无效".into())
}
fn request_url(host: &str, args: &Request) -> AppResult<url::Url> {
    let host = api_host(host)?;
    let path = match args.kind {
        Kind::Now => "/v7/weather/now",
        Kind::Daily => "/v7/weather/3d",
        Kind::Indices => "/v7/indices/1d",
        Kind::Lookup => "/geo/v2/city/lookup",
    };
    let mut url = url::Url::parse(&format!("https://{host}{path}")).map_err(err)?;
    let value = if matches!(args.kind, Kind::Lookup) {
        args.query
            .as_deref()
            .filter(|s| {
                !s.trim().is_empty() && s.chars().count() <= 80 && !s.chars().any(char::is_control)
            })
            .ok_or("INVALID_ARGUMENT: 城市搜索需要1至80个字符")?
    } else {
        let value = args
            .location
            .as_deref()
            .ok_or("INVALID_ARGUMENT: 缺少城市标识")?;
        location(value)?;
        value
    };
    url.query_pairs_mut()
        .append_pair("location", value)
        .append_pair("lang", "zh");
    if matches!(args.kind, Kind::Indices) {
        url.query_pairs_mut().append_pair("type", "0");
    }
    Ok(url)
}
fn decoded(bytes: &[u8]) -> AppResult<Vec<u8>> {
    if bytes.len() > LIMIT {
        return Err("LIMIT: 天气响应过大".into());
    }
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes.to_vec());
    }
    let mut output = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|_| "DATA_UNAVAILABLE: 天气压缩数据损坏")?;
    if output.len() > LIMIT {
        return Err("LIMIT: 天气解压响应过大".into());
    }
    Ok(output)
}
fn parse_response(bytes: &[u8], kind: &Kind) -> AppResult<Value> {
    let value: Value = serde_json::from_slice(&decoded(bytes)?)
        .map_err(|_| "DATA_UNAVAILABLE: 天气响应不是有效JSON")?;
    if value["code"] != "200" && value["code"] != 200 {
        let code = value["code"]
            .as_str()
            .filter(|code| code.len() <= 3 && code.bytes().all(|c| c.is_ascii_digit()))
            .unwrap_or("未知");
        return Err(format!(
            "DATA_UNAVAILABLE: 和风天气返回{code}，请检查凭据、服务权限或额度"
        ));
    }
    let number = |value: &Value| {
        value
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 32)
            .and_then(|s| rust_decimal::Decimal::from_str_exact(s).ok())
            .is_some()
    };
    let icon = |value: &Value| {
        value
            .as_str()
            .is_some_and(|s| (3..=4).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit()))
    };
    match kind {
        Kind::Now => {
            let now = &value["now"];
            let time = now["obsTime"]
                .as_str()
                .filter(|s| s.len() <= 40)
                .ok_or("DATA_UNAVAILABLE: 缺少天气观测时间")?;
            if !number(&now["temp"])
                || !icon(&now["icon"])
                || !now["text"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 160)
                || (chrono::DateTime::parse_from_rfc3339(time).is_err()
                    && chrono::DateTime::parse_from_str(time, "%Y-%m-%dT%H:%M%:z").is_err())
            {
                return Err("DATA_UNAVAILABLE: 实时天气温度、图标或来源时间无效".into());
            }
        }
        Kind::Daily => {
            let days = value["daily"]
                .as_array()
                .filter(|items| !items.is_empty() && items.len() <= 3)
                .ok_or("DATA_UNAVAILABLE: 三日预报无效")?;
            for day in days {
                if !number(&day["tempMin"])
                    || !number(&day["tempMax"])
                    || !icon(&day["iconDay"])
                    || day["fxDate"]
                        .as_str()
                        .is_none_or(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err())
                {
                    return Err("DATA_UNAVAILABLE: 预报温度或日期无效".into());
                }
            }
        }
        Kind::Indices => {
            if value["daily"]
                .as_array()
                .is_none_or(|items| items.len() > 32 || items.iter().any(|item| !item.is_object()))
            {
                return Err("DATA_UNAVAILABLE: 生活指数无效".into());
            }
        }
        Kind::Lookup => {
            let cities = value["location"]
                .as_array()
                .filter(|items| items.len() <= 20)
                .ok_or("DATA_UNAVAILABLE: 城市列表无效")?;
            for city in cities {
                let id = city["id"]
                    .as_str()
                    .ok_or("DATA_UNAVAILABLE: 城市标识缺失")?;
                location(id)?;
                if !city["name"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 160)
                {
                    return Err("DATA_UNAVAILABLE: 城市名称无效".into());
                }
            }
        }
    }
    Ok(value)
}

#[cfg(windows)]
fn crypt(data: &[u8], instance: &str, decrypt: bool) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if data.len() > 65536 {
        return Err("LIMIT: 天气凭据过大".into());
    }
    let entropy = format!("SoulDesk.weather.v1/{instance}").into_bytes();
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let additional = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &additional,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &additional,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err("CREDENTIAL_ERROR: 天气凭据保护失败，可能不属于当前Windows用户或实例".into());
    }
    let result = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    unsafe {
        LocalFree(output.pbData as _);
    }
    Ok(result)
}
fn protect(data: &[u8], instance: &str) -> AppResult<Vec<u8>> {
    #[cfg(windows)]
    {
        crypt(data, instance, false)
    }
    #[cfg(not(windows))]
    {
        let _ = (data, instance);
        Err("UNSUPPORTED: 当前仅支持Windows凭据保护".into())
    }
}
fn unprotect(data: &[u8], instance: &str) -> AppResult<Vec<u8>> {
    #[cfg(windows)]
    {
        crypt(data, instance, true)
    }
    #[cfg(not(windows))]
    {
        let _ = (data, instance);
        Err("UNSUPPORTED: 当前仅支持Windows凭据保护".into())
    }
}
fn credential_key(instance: &str) -> String {
    format!("weather-credential:{instance}")
}
fn load(store: &Store, instance: &str) -> AppResult<Option<Credential>> {
    let value: Option<String> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [credential_key(instance)],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    value
        .map(|value| {
            let encrypted = STANDARD
                .decode(value)
                .map_err(|_| "CREDENTIAL_ERROR: 天气密文无效")?;
            serde_json::from_slice(&unprotect(&encrypted, instance)?)
                .map_err(|_| "CREDENTIAL_ERROR: 天气凭据格式无效".into())
        })
        .transpose()
}
fn status(value: Option<&Credential>) -> Value {
    json!({"configured":value.is_some(),"enabled":value.is_some_and(|v|v.enabled),"apiHost":value.map(|v|v.host.as_str()).unwrap_or("")})
}
fn configure(store: &Store, instance: &str, args: Configure) -> AppResult<Value> {
    let previous = load(store, instance)?;
    let host = api_host(&args.api_host)?;
    let key = args
        .api_key
        .filter(|s| !s.is_empty())
        .or_else(|| {
            previous
                .as_ref()
                .filter(|old| old.host == host)
                .map(|old| old.key.clone())
        })
        .ok_or("INVALID_ARGUMENT: 首次配置或更换API Host时必须提供自己的API Key")?;
    if key.len() > 512
        || !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("INVALID_ARGUMENT: API Key格式无效".into());
    }
    let value = Credential {
        host,
        key,
        enabled: args.enabled,
    };
    let plain = serde_json::to_vec(&value).map_err(err)?;
    store.save_preference(
        &credential_key(instance),
        &STANDARD.encode(protect(&plain, instance)?),
    )?;
    Ok(status(Some(&value)))
}
pub(crate) fn forget(store: &Store, instance: &str) -> AppResult<()> {
    store
        .db
        .lock()
        .map_err(err)?
        .execute(
            "DELETE FROM app_settings WHERE key=?1",
            [credential_key(instance)],
        )
        .map_err(err)?;
    Ok(())
}
pub(crate) fn remove_data(
    transaction: &rusqlite::Transaction<'_>,
    instance: &str,
) -> AppResult<()> {
    transaction
        .execute(
            "DELETE FROM app_settings WHERE key=?1",
            [credential_key(instance)],
        )
        .map_err(err)?;
    Ok(())
}
async fn read_weather_bytes(
    config: &network::NetworkConfig,
    url: &str,
    key: &str,
) -> AppResult<Vec<u8>> {
    network::http_get_bounded_headers(
        config,
        url,
        LIMIT,
        &[("X-QW-Api-Key", key), ("Accept-Encoding", "gzip")],
    )
    .await
    .map_err(|error| {
        if error.starts_with("DATA_UNAVAILABLE: 内容源HTTP ") || error.starts_with("LIMIT:") {
            error
        } else {
            "NETWORK_ERROR: 和风天气请求失败，请检查网络及服务状态".into()
        }
    })
}
pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let instance =
        widgets::require_capability(&core.store, &core.sessions, window, token, "weather.read")?;
    match method {
        "weather.status" => return Ok(status(load(&core.store, &instance)?.as_ref())),
        "weather.configure" => {
            return configure(
                &core.store,
                &instance,
                serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 天气配置参数无效")?,
            )
        }
        "weather.clear" => {
            forget(&core.store, &instance)?;
            return Ok(status(None));
        }
        "weather.get" => {}
        _ => return Err("CAPABILITY_DENIED: 未开放的天气方法".into()),
    }
    let credential = load(&core.store, &instance)?
        .filter(|v| v.enabled)
        .ok_or("CONFIG_REQUIRED: 请先设置自己的和风API Host/API Key并启用联网")?;
    let request: Request =
        serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENT: 天气请求参数无效")?;
    let url = request_url(&credential.host, &request)?;
    let config: Value = serde_json::from_str(
        &core
            .store
            .preference("network")
            .unwrap_or("{\"mode\":\"system\"}".into()),
    )
    .map_err(err)?;
    let config = business::resolve_network(&config)?;
    let bytes = read_weather_bytes(&config, url.as_str(), &credential.key).await?;
    widgets::require_capability(&core.store, &core.sessions, window, token, "weather.read")?;
    let current = load(&core.store, &instance)?.ok_or("SESSION_EXPIRED: 天气配置已清除")?;
    if !current.enabled || current.host != credential.host || current.key != credential.key {
        return Err("SESSION_EXPIRED: 天气配置已变更，丢弃旧响应".into());
    }
    Ok(
        json!({"source":"QWeather","fetchedAt":now_ms(),"data":parse_response(&bytes, &request.kind)?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn weather_http_uses_header_credentials_gzip_and_never_follows_redirects() {
        use std::io::Write;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for redirect in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    let mut chunk = [0u8; 2048];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0 && bytes.len() < 16384);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let request = String::from_utf8(bytes).unwrap().to_lowercase();
                assert!(!request
                    .lines()
                    .next()
                    .unwrap()
                    .contains("placeholder-secret"));
                assert!(request.contains("x-qw-api-key: placeholder-secret"));
                assert!(request.contains("accept-encoding: gzip"));
                if redirect {
                    stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/must-not-follow\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    let mut encoder =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(br#"{"code":"200","now":{"temp":"0","text":"sun","icon":"100","obsTime":"2026-11-16T08:00+08:00"}}"#).unwrap();
                    let body = encoder.finish().unwrap();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    stream.write_all(&body).await.unwrap();
                }
            });
            let result = read_weather_bytes(
                &network::NetworkConfig::Direct,
                &format!("http://{address}/v7/weather/now?location=101010100"),
                "placeholder-secret",
            )
            .await;
            if redirect {
                let error = result.unwrap_err();
                assert!(error.contains("302"));
                assert!(!error.contains("placeholder-secret"));
            } else {
                assert_eq!(
                    parse_response(&result.unwrap(), &Kind::Now).unwrap()["now"]["temp"],
                    "0"
                );
            }
            peer.await.unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn weather_formal_capability_preview_and_instance_deletion_are_isolated() {
        use std::sync::{Arc, Mutex};
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-weather-cap-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.weather-small")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let first =
            widgets::create_instance(&store, "local.souldesk.widgetjs.weather-small").unwrap();
        let second =
            widgets::create_instance(&store, "local.souldesk.widgetjs.weather-small").unwrap();
        let sessions = widgets::Sessions::default();
        let window = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &window, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        assert_eq!(
            widgets::require_capability(&store, &sessions, &window, token, "weather.read").unwrap(),
            first
        );
        assert!(
            widgets::require_capability(&store, &sessions, "other", token, "weather.read").is_err()
        );
        for id in [&first, &second] {
            configure(
                &store,
                id,
                Configure {
                    api_host: "placeholder.qweatherapi.com".into(),
                    api_key: Some("placeholder_secret".into()),
                    enabled: false,
                },
            )
            .unwrap();
        }
        assert!(!serde_json::to_string(&store.snapshot().unwrap())
            .unwrap()
            .contains("placeholder_secret"));
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
                instance: first.clone(),
                window: "trial-weather".into(),
                shared_cursor: None,
                preview: Some(preview),
            },
        );
        assert!(widgets::require_capability(
            &store,
            &sessions,
            "trial-weather",
            "preview",
            "weather.read"
        )
        .is_err());
        widgets::remove_instance(&store, &sessions, &first).unwrap();
        assert!(load(&store, &first).unwrap().is_none());
        assert!(load(&store, &second).unwrap().is_some());
    }

    #[test]
    fn weather_gzip_is_decoded_with_a_separate_expansion_limit() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(br#"{"code":"401"}"#).unwrap();
        let packed = encoder.finish().unwrap();
        assert_eq!(decoded(&packed).unwrap(), br#"{"code":"401"}"#);
        assert!(decoded(&[0x1f, 0x8b, 0]).is_err());
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&vec![b' '; LIMIT + 1]).unwrap();
        assert!(decoded(&encoder.finish().unwrap())
            .unwrap_err()
            .starts_with("LIMIT:"));
        assert!(decoded(&vec![b' '; LIMIT + 1])
            .unwrap_err()
            .starts_with("LIMIT:"));
    }

    #[test]
    fn weather_partial_data_and_search_requests_do_not_silently_change_identity() {
        let overseas = json!({"code":"200","location":[{"id":"TEST123","name":"夹具城市","tz":"America/Los_Angeles"}]});
        assert_eq!(
            parse_response(&serde_json::to_vec(&overseas).unwrap(), &Kind::Lookup).unwrap(),
            overseas
        );
        let request: Request =
            serde_json::from_value(json!({"kind":"now","location":"TEST123"})).unwrap();
        assert!(request_url("abc.xy.qweatherapi.com", &request).is_ok());
        assert!(parse_response(br#"{"code":"200","daily":[null]}"#, &Kind::Indices).is_err());
        let daily = json!({"code":"200","daily":[{"fxDate":"2026-11-16","tempMin":"-2","tempMax":"5","iconDay":"100"}]});
        assert!(parse_response(&serde_json::to_vec(&daily).unwrap(), &Kind::Daily).is_ok());
        let mut bad = daily.clone();
        bad["daily"][0]["tempMin"] = Value::Null;
        assert!(parse_response(&serde_json::to_vec(&bad).unwrap(), &Kind::Daily).is_err());
        let cities = json!({"code":"200","location":[{"id":"101010100","name":"北京","lat":"39.90","lon":"116.40","tz":"Asia/Shanghai"}]});
        assert_eq!(
            parse_response(&serde_json::to_vec(&cities).unwrap(), &Kind::Lookup).unwrap(),
            cities
        );
        for value in [
            "",
            "../now",
            "101010100&key=oops",
            "181,20",
            "116,91",
            "NaN,20",
        ] {
            let request: Request =
                serde_json::from_value(json!({"kind":"now","location":value})).unwrap();
            assert!(request_url("abc.xy.qweatherapi.com", &request).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn weather_settings_store_only_ciphertext_and_failed_updates_preserve_credentials() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-weather-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        assert_eq!(
            status(load(&store, "first").unwrap().as_ref())["enabled"],
            false
        );
        let result = configure(
            &store,
            "first",
            Configure {
                api_host: "abc.xy.qweatherapi.com".into(),
                api_key: Some("placeholder_key_123".into()),
                enabled: false,
            },
        )
        .unwrap();
        assert_eq!(result["configured"], true);
        assert_eq!(result["enabled"], false);
        assert!(result.get("apiKey").is_none());
        let encrypted = store.preference(&credential_key("first")).unwrap();
        assert!(!encrypted.contains("placeholder_key_123"));
        assert!(load(&store, "second").unwrap().is_none());
        assert!(configure(
            &store,
            "first",
            Configure {
                api_host: "evil.test".into(),
                api_key: Some("bad".into()),
                enabled: true
            }
        )
        .is_err());
        assert_eq!(
            store.preference(&credential_key("first")).unwrap(),
            encrypted
        );
        configure(
            &store,
            "first",
            Configure {
                api_host: "abc.xy.qweatherapi.com".into(),
                api_key: None,
                enabled: true,
            },
        )
        .unwrap();
        assert!(load(&store, "first").unwrap().unwrap().enabled);
        assert_eq!(
            load(&store, "first").unwrap().unwrap().key,
            "placeholder_key_123"
        );
        {
            let mut db = store.db.lock().unwrap();
            let transaction = db.transaction().unwrap();
            remove_data(&transaction, "first").unwrap();
            transaction.rollback().unwrap();
        }
        assert!(load(&store, "first").unwrap().is_some());
        forget(&store, "first").unwrap();
        assert!(load(&store, "first").unwrap().is_none());
    }

    #[test]
    fn weather_host_and_paths_cannot_redirect_credentials_to_other_services() {
        assert_eq!(
            api_host("https://abc.xy.qweatherapi.com").unwrap(),
            "abc.xy.qweatherapi.com"
        );
        for host in [
            "http://abc.qweatherapi.com",
            "qweatherapi.com",
            "evilqweatherapi.com",
            "abc.qweatherapi.com.evil.test",
            "https://user:pass@abc.qweatherapi.com",
            "https://abc.qweatherapi.com:444",
            "abc.qweatherapi.com/path",
            "127.0.0.1",
        ] {
            assert!(api_host(host).is_err(), "{host}");
        }
        let args: Request =
            serde_json::from_value(json!({"kind":"lookup","query":"厦门&key=do-not-forward"}))
                .unwrap();
        let url = request_url("abc.xy.qweatherapi.com", &args).unwrap();
        assert_eq!(url.path(), "/geo/v2/city/lookup");
        assert_eq!(url.query_pairs().count(), 2);
        assert!(!url.query_pairs().any(|(name, _)| name == "key"));
        assert!(
            serde_json::from_value::<Request>(json!({"kind":"now","url":"https://evil.test"}))
                .is_err()
        );
    }

    #[test]
    fn weather_values_keep_observation_time_and_do_not_turn_missing_temperature_into_zero() {
        let value = json!({"code":"200","updateTime":"2026-11-16T08:00+08:00","now":{"obsTime":"2026-11-16T07:40+08:00","temp":"-2.5","icon":"100","text":"晴","humidity":"70"}});
        let parsed = parse_response(&serde_json::to_vec(&value).unwrap(), &Kind::Now).unwrap();
        assert_eq!(parsed["now"]["temp"], "-2.5");
        assert_eq!(parsed["now"]["obsTime"], "2026-11-16T07:40+08:00");
        for replacement in [json!(null), json!(""), json!("NaN")] {
            let mut bad = value.clone();
            bad["now"]["temp"] = replacement;
            assert!(parse_response(&serde_json::to_vec(&bad).unwrap(), &Kind::Now).is_err());
        }
        let failure = parse_response(
            br#"{"code":"401","message":"secret-placeholder"}"#,
            &Kind::Now,
        )
        .unwrap_err();
        assert!(failure.contains("401"));
        assert!(!failure.contains("secret-placeholder"));
    }

    #[cfg(windows)]
    #[test]
    fn weather_secret_roundtrip_is_encrypted_and_instance_bound() {
        let input = b"placeholder-weather-secret";
        let encrypted = protect(input, "first").unwrap();
        assert!(!encrypted.windows(input.len()).any(|window| window == input));
        assert_eq!(unprotect(&encrypted, "first").unwrap(), input);
        assert!(unprotect(&encrypted, "second").is_err());
    }
}
