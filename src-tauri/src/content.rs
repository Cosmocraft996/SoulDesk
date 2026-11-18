//! 公开内容组件的有限读取接口，不接受任意请求地址、Cookie或脚本。
use crate::{
    business, network,
    store::{err, now_ms, AppResult},
    widgets, Core,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

fn source_url(source: &str, args: &Value) -> AppResult<String> {
    if let Some(url) = crate::equity::source_url(source, args)? {
        return Ok(url);
    }
    let value = match source {
        "forex" => {
            const CURRENCIES: &[&str] = &["CNY", "USD", "JPY", "EUR", "KRW", "AUD", "HKD", "PHP", "MYR", "SGD", "THB", "VND", "IDR", "RUB"];
            let from = args["from"].as_str().filter(|v| CURRENCIES.contains(v)).ok_or("INVALID_ARGUMENT: 来源币种无效")?;
            let to = args["to"].as_str().filter(|v| CURRENCIES.contains(v)).ok_or("INVALID_ARGUMENT: 目标币种无效")?;
            return Ok(format!("https://finance.pae.baidu.com/api/getrevforeigndata?query={to}{from}&finClientType=pc"));
        }
        "a-index" => {
            let code=args["code"].as_str().filter(|code|["000001","399001","399006"].contains(code)).ok_or("INVALID_ARGUMENT: 仅支持原三大指数代码")?;
            return Ok(format!("https://finance.pae.baidu.com/selfselect/sug?wd={code}&skip_login=1&finClientType=pc"));
        }
        "coin-index"=>return crate::coin::index_url(args["instrument"].as_str().ok_or("INVALID_ARGUMENT: 缺少指数标识")?),
        "itime-calendar" => {
            if args.as_object().is_none_or(|fields| fields.len() != 1 || !fields.contains_key("source")) {
                return Err("INVALID_ARGUMENT: 黄历来源不接受额外参数".into());
            }
            "https://itime.fun/api/v2/event/public/calendar"
        }
        "bilibili" => "https://s.search.bilibili.com/main/hotword",
        "bangumi" => "https://bangumi.bilibili.com/web_api/timeline_global",
        "douyin" => "https://aweme.snssdk.com/aweme/v1/hot/search/list/",
        "weibo" => "https://weibo.com/ajax/side/hotSearch",
        "zhihu" => "https://www.zhihu.com/api/v3/feed/topstory/hot-lists/total?limit=30&desktop=true",
        "metal" => "https://biquote.io/api/latest?symbols=XAUUSD&symbols=XAGUSD&symbols=XCUUSD&symbols=XPTUSD",
        "gold" => "https://push2.eastmoney.com/api/qt/stock/get?secid=118.AU9999&fields=f43,f44,f45,f46,f57,f58,f60,f86",
        "gold-trends" => "https://push2his.eastmoney.com/api/qt/stock/trends2/get?secid=118.AU9999&fields1=f1,f2,f3,f4,f5,f6,f7,f8,f9,f10,f11,f12,f13&fields2=f51,f52,f53,f54,f55,f56,f57,f58&ndays=1&iscr=0",
        "tou-tiao" => "https://www.toutiao.com/hot-event/hot-board/?origin=toutiao_pc",
        "cls-telegraph" => return Ok(format!("https://www.cls.cn/api/cache?app=CailianpressWeb&lastTime={}&name=refreshTenTelegraph&os=web&sv=8.7.9", now_ms() / 1000)),
        "bilibili-rcmd" => {
            let shown = args.get("lastShown").map(|v| v.as_str().ok_or("INVALID_ARGUMENT: 推荐列表参数无效")).transpose()?.unwrap_or("");
            if shown.len() > 10000 || !shown.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b',') { return Err("INVALID_ARGUMENT: 推荐列表参数无效".into()); }
            let mut url = url::Url::parse("https://api.bilibili.com/x/web-interface/wbi/index/top/feed/rcmd?brush=4&homepage_ver=1&ps=10&last_y_num=5").map_err(err)?;
            url.query_pairs_mut().append_pair("last_show_list", shown).append_pair("w_ts", &(now_ms() / 1000).to_string()).append_pair("fresh_idx", &(now_ms() / 1000).to_string());
            return Ok(url.to_string());
        }
        _ => return Err("CAPABILITY_DENIED: 未开放的公开内容源".into()),
    };
    Ok(value.into())
}

pub(crate) fn source_decimal(
    value: &serde_json::value::RawValue,
) -> AppResult<rust_decimal::Decimal> {
    let text = value.get();
    if text.len() > 96 {
        return Err("DATA_UNAVAILABLE: 源数值超出支持范围".into());
    }
    (if let Some((base, _)) = text.split_once(['e', 'E']) {
        rust_decimal::Decimal::from_str_exact(base)
            .and_then(|_| rust_decimal::Decimal::from_scientific(text))
    } else {
        rust_decimal::Decimal::from_str_exact(text)
    })
    .map_err(|_| "DATA_UNAVAILABLE: 源数值无效或超出十进制范围".into())
}

fn parse_baidu_index(bytes: &[u8], code: &str) -> AppResult<Value> {
    let expected = match code {
        "000001" => "SH",
        "399001" | "399006" => "SZ",
        _ => return Err("INVALID_ARGUMENT: 指数代码无效".into()),
    };
    let response: Value =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 指数数据格式无效")?;
    if response["ResultCode"] != json!("0") && response["ResultCode"] != json!(0) {
        return Err("DATA_UNAVAILABLE: 指数源返回失败".into());
    }
    let items = response["Result"]["stock"]
        .as_array()
        .filter(|items| items.len() <= 1000)
        .ok_or("DATA_UNAVAILABLE: 指数候选列表无效")?;
    let value = items
        .iter()
        .find(|value| {
            value["code"] == code
                && value["type"] == "index"
                && value["market"] == "ab"
                && value["exchange"] == expected
        })
        .ok_or("DATA_UNAVAILABLE: 未找到匹配交易所的指数")?;
    let price = value["price"]
        .as_str()
        .ok_or("DATA_UNAVAILABLE: 指数价格缺失")?;
    if price.len() > 96
        || rust_decimal::Decimal::from_str_exact(price)
            .map_err(|_| "DATA_UNAVAILABLE: 指数价格无效")?
            <= rust_decimal::Decimal::ZERO
    {
        return Err("DATA_UNAVAILABLE: 指数价格无效".into());
    }
    let ratio = value["ratio"]
        .as_str()
        .filter(|value| value.len() <= 96)
        .ok_or("DATA_UNAVAILABLE: 指数涨跌幅缺失")?
        .strip_suffix('%')
        .unwrap_or(value["ratio"].as_str().unwrap());
    let ratio = ratio.strip_prefix('+').unwrap_or(ratio);
    rust_decimal::Decimal::from_str_exact(ratio).map_err(|_| "DATA_UNAVAILABLE: 指数涨跌幅无效")?;
    let name = value["name"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or("DATA_UNAVAILABLE: 指数名称无效")?;
    let status = value["stockStatusInfo"].as_str().unwrap_or("状态未提供");
    let source_time = response["Result"]["refresh_time"].as_str().unwrap_or("");
    if source_time.len() > 128 || status.len() > 64 {
        return Err("DATA_UNAVAILABLE: 指数来源状态无效".into());
    }
    Ok(
        json!({"code":code,"type":"index","market":"ab","exchange":expected,"name":name,"price":price,"ratio":ratio,"stockStatusInfo":status,"stockStatus":value["stockStatus"],"sourceTimeText":source_time}),
    )
}

fn parse_gold_quote(bytes: &[u8]) -> AppResult<Value> {
    #[derive(serde::Deserialize)]
    struct Quote {
        f43: Box<serde_json::value::RawValue>,
        f44: Option<Box<serde_json::value::RawValue>>,
        f45: Option<Box<serde_json::value::RawValue>>,
        f57: String,
        f60: Option<Box<serde_json::value::RawValue>>,
        f86: u64,
    }
    #[derive(serde::Deserialize)]
    struct Response {
        rc: i32,
        data: Option<Quote>,
    }
    fn price(value: &serde_json::value::RawValue) -> AppResult<rust_decimal::Decimal> {
        let value = source_decimal(value)?;
        if !value.fract().is_zero() {
            return Err("DATA_UNAVAILABLE: 黄金原始价格精度不符".into());
        }
        value
            .checked_div(rust_decimal::Decimal::from(100))
            .ok_or("DATA_UNAVAILABLE: 黄金价格换算失败".into())
    }
    fn statistic(value: Option<&serde_json::value::RawValue>) -> AppResult<Option<String>> {
        value.map(price).transpose().map(|value| {
            value
                .filter(|v| *v > rust_decimal::Decimal::ZERO)
                .map(|v| v.normalize().to_string())
        })
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 黄金报价格式无效")?;
    if response.rc != 0 {
        return Err("DATA_UNAVAILABLE: 黄金内容源返回失败".into());
    }
    let value = response.data.ok_or("DATA_UNAVAILABLE: 黄金报价缺失")?;
    let current = price(&value.f43)?;
    let timestamp = value
        .f86
        .checked_mul(1000)
        .filter(|value| *value > 0 && *value <= 8_640_000_000_000_000)
        .ok_or("DATA_UNAVAILABLE: 黄金报价时间无效")?;
    if value.f57 != "AU9999" || current <= rust_decimal::Decimal::ZERO {
        return Err("DATA_UNAVAILABLE: 黄金合约或价格不匹配".into());
    }
    let max = statistic(value.f44.as_deref())?;
    let min = statistic(value.f45.as_deref())?;
    if let (Some(max), Some(min)) = (&max, &min) {
        if max.parse::<rust_decimal::Decimal>().map_err(err)?
            < min.parse::<rust_decimal::Decimal>().map_err(err)?
        {
            return Err("DATA_UNAVAILABLE: 黄金最高最低价顺序无效".into());
        }
    }
    Ok(
        json!({"instrument":"Au99.99","code":"AU9999","latestPrice":current.normalize().to_string(),"yesterdayClose":statistic(value.f60.as_deref())?,"min":min,"max":max,"timestamp":timestamp}),
    )
}

fn parse_gold_trends(bytes: &[u8]) -> AppResult<Value> {
    #[derive(serde::Deserialize)]
    struct Trends {
        code: Option<String>,
        trends: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    struct Response {
        rc: i32,
        data: Option<Trends>,
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 黄金分时格式无效")?;
    if response.rc != 0 {
        return Err("DATA_UNAVAILABLE: 黄金分时源返回失败".into());
    }
    let data = response.data.ok_or("DATA_UNAVAILABLE: 黄金分时缺失")?;
    if data.code.as_ref().is_some_and(|value| value != "AU9999") || data.trends.len() > 3000 {
        return Err("DATA_UNAVAILABLE: 黄金分时身份或容量无效".into());
    }
    let mut times = Vec::new();
    let mut values = Vec::new();
    for item in data.trends {
        if item.len() > 512 {
            return Err("DATA_UNAVAILABLE: 黄金分时条目无效".into());
        }
        let parts = item.split(',').collect::<Vec<_>>();
        if parts.len() < 3 || parts[0].len() > 40 {
            return Err("DATA_UNAVAILABLE: 黄金分时条目无效".into());
        }
        let price = rust_decimal::Decimal::from_str_exact(parts[2])
            .map_err(|_| "DATA_UNAVAILABLE: 黄金分时价格无效")?;
        if price <= rust_decimal::Decimal::ZERO {
            return Err("DATA_UNAVAILABLE: 黄金分时价格无效".into());
        }
        times.push(parts[0].to_owned());
        values.push(price.normalize().to_string());
    }
    Ok(json!({"code":"AU9999","times":times,"values":values}))
}

// 保留原JSON数字字面量，避免先转为二进制浮点再生成价格字符串。
fn parse_metals(bytes: &[u8]) -> AppResult<Value> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Tick {
        symbol: String,
        mid: Box<serde_json::value::RawValue>,
        day_diff_percent: Box<serde_json::value::RawValue>,
        timestamp: String,
        stale: bool,
        market_state: Option<String>,
        source: Option<String>,
        exchange: Option<String>,
        last_quote_at: Option<String>,
        quote_age_seconds: Option<u32>,
    }
    let quotes: std::collections::BTreeMap<String, Tick> =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 贵金属数据格式无效")?;
    let mut result = Vec::new();
    for (code, name) in [
        ("XAUUSD", "Gold"),
        ("XAGUSD", "Silver"),
        ("XCUUSD", "Copper"),
        ("XPTUSD", "Platinum"),
    ] {
        let tick = quotes
            .get(code)
            .ok_or("DATA_UNAVAILABLE: 贵金属报价不完整")?;
        let price = source_decimal(&tick.mid)?;
        let change = source_decimal(&tick.day_diff_percent)?;
        let quote_time = tick.last_quote_at.as_deref().unwrap_or(&tick.timestamp);
        if tick.symbol != code
            || price <= rust_decimal::Decimal::ZERO
            || tick.timestamp.is_empty()
            || tick.timestamp.len() > 80
            || tick
                .market_state
                .as_ref()
                .is_some_and(|value| value.len() > 64)
            || quote_time.is_empty()
            || quote_time.len() > 80
            || tick.source.as_ref().is_some_and(|value| value.len() > 128)
            || tick
                .exchange
                .as_ref()
                .is_some_and(|value| value.len() > 128)
        {
            return Err("DATA_UNAVAILABLE: 贵金属身份、价格或时间状态无效".into());
        }
        result.push(json!({"code":code,"name":name,"price":price.normalize().to_string(),"changePercent":change.normalize().to_string(),"updatedAt":quote_time,"sourceTimestamp":tick.timestamp,"stale":tick.stale,"marketState":tick.market_state.as_deref().unwrap_or("状态未提供"),"dataSource":tick.source,"exchange":tick.exchange,"quoteAgeSeconds":tick.quote_age_seconds}));
    }
    Ok(json!(result))
}

fn external_url(value: &str) -> AppResult<url::Url> {
    if value.len() > 4096
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        return Err("INVALID_ARGUMENT: 网页地址无效".into());
    }
    let parsed = url::Url::parse(value).map_err(err)?;
    let domain = parsed
        .domain()
        .ok_or("INVALID_ARGUMENT: 只允许公开网页域名")?;
    if parsed.scheme() != "https"
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !domain.contains('.')
        || [".local", ".internal", ".localhost"]
            .iter()
            .any(|suffix| domain.ends_with(suffix))
    {
        return Err("INVALID_ARGUMENT: 只允许无凭证的HTTPS公开网页".into());
    }
    Ok(parsed)
}

use widgets::require_capability as authorize;

// 在缓存、网络和快照读取前核对来源。币价快照与指数请求共用同一个来源许可。
fn authorize_content(
    store: &crate::store::Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    method: &str,
    source: Option<&str>,
) -> AppResult<String> {
    use crate::component_registry::{content_source_capability, LEGACY_CONTENT_READ};
    if method == "content.get" || (method == "content.peek" && source == Some("coin-index")) {
        if let Some(capability) = source.and_then(content_source_capability) {
            return widgets::with_any_capability(
                store,
                sessions,
                window,
                token,
                &[LEGACY_CONTENT_READ, capability],
                |session| Ok(session.instance.clone()),
            );
        }
    }
    authorize(store, sessions, window, token, LEGACY_CONTENT_READ)
}

fn index_settings(core: &Core, instance: &str) -> AppResult<Value> {
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
    serde_json::from_str(&settings).map_err(err)
}
fn content_network(core: &Core, instance: &str, source: &str) -> AppResult<network::NetworkConfig> {
    let config: Value = serde_json::from_str(
        &core
            .store
            .preference("network")
            .unwrap_or("{\"mode\":\"system\"}".into()),
    )
    .map_err(err)?;
    let inherited = business::resolve_network(&config);
    if source != "coin-index" {
        return inherited;
    }
    let settings = index_settings(core, instance)?;
    let inherited = if settings["proxyMode"].as_str().unwrap_or("inherit") == "inherit" {
        inherited?
    } else {
        network::NetworkConfig::Direct
    };
    crate::coin::instance_network(&settings, inherited)
}

pub async fn gateway(
    app: &AppHandle,
    window: &str,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    if method == "links.open" {
        authorize(&core.store, &core.sessions, window, token, "links.open")?;
        let url = external_url(
            args["url"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少网页地址")?,
        )?;
        return open_external(&url);
    }
    if !matches!(method, "content.get" | "content.peek") {
        return Err("CAPABILITY_DENIED: 未开放的方法".into());
    }
    let source = args["source"].as_str();
    let auth = || authorize_content(&core.store, &core.sessions, window, token, method, source);
    let instance = auth()?;
    let source = source.ok_or("INVALID_ARGUMENT: 缺少内容源")?;
    if method == "content.get"
        && args["cache"] == true
        && crate::content_cache::lifetime(source).is_some()
    {
        let (parameters, force) = cache_parameters(&args)?;
        let url = source_url(source, &parameters)?;
        let config = content_network(&core, &instance, source)?;
        let response = core
            .content_cache
            .read(
                source,
                &parameters,
                &config,
                force,
                fetch_public(source.to_owned(), url, config.clone()),
            )
            .await?;
        auth()?;
        if content_network(&core, &instance, source)? != config {
            return Err("DATA_UNAVAILABLE: 网络设置已变化，请重新刷新".into());
        }
        return Ok(response);
    }
    let url = source_url(source, &args)?;
    let config = content_network(&core, &instance, source)?;
    let cache_key = serde_json::to_string(&(&args["instrument"], &config)).map_err(err)?;
    if method == "content.peek" {
        if source != "coin-index" {
            return Err("INVALID_ARGUMENT: 此来源没有临时展示快照".into());
        }
        return core.coin_index.peek(&instance, &cache_key);
    }
    let headers: &[(&str, &str)] = if matches!(source, "gold" | "gold-trends") {
        &[("x-custom-referer", "push2.eastmoney.com")]
    } else {
        &[]
    };
    let bytes = if headers.is_empty() {
        network::http_get_bounded(&config, &url, 2 * 1024 * 1024).await?
    } else {
        network::http_get_bounded_headers(&config, &url, 2 * 1024 * 1024, headers).await?
    };
    let data: Value = match source {
        "metal" => parse_metals(&bytes)?,
        "gold" => parse_gold_quote(&bytes)?,
        "gold-trends" => parse_gold_trends(&bytes)?,
        "a-index" => parse_baidu_index(
            &bytes,
            args["code"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少指数代码")?,
        )?,
        "stock-search" => crate::equity::parse_search(&bytes)?,
        "stock-search-eastmoney" => crate::equity::parse_eastmoney_search(&bytes)?,
        "stock-quote" => crate::equity::parse_quote(
            &bytes,
            args["code"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少证券代码")?,
            args["market"]
                .as_u64()
                .ok_or("INVALID_ARGUMENT: 缺少证券市场")?,
        )?,
        "coin-index" => crate::coin::parse_index(
            &bytes,
            args["instrument"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少指数标识")?,
        )?,
        "stock-minute" => crate::equity::parse_minute(
            &bytes,
            args["code"]
                .as_str()
                .ok_or("INVALID_ARGUMENT: 缺少证券代码")?,
            args["market"]
                .as_u64()
                .ok_or("INVALID_ARGUMENT: 缺少证券市场")?,
        )?,
        _ => serde_json::from_slice(&bytes)
            .map_err(|_| "DATA_UNAVAILABLE: 内容源未返回有效JSON".to_owned())?,
    };
    // 等待期间若会话失效，不把结果交给新实例或旧页面。
    auth()?;
    let response = json!({"data":data,"source":url,"fetchedAt":now_ms()});
    if source == "coin-index" {
        // 旧请求返回时若已换币或更换网络，不覆盖当前展示口径的缓存。
        if let (Ok(settings), Ok(current)) = (
            index_settings(&core, &instance),
            content_network(&core, &instance, source),
        ) {
            let current_key =
                serde_json::to_string(&(&args["instrument"], current)).map_err(err)?;
            if settings["coin"] == args["instrument"] && current_key == cache_key {
                core.coin_index
                    .remember(&instance, cache_key, response.clone())?;
            }
        }
    }
    Ok(response)
}

// 刷新标志只控制已批准来源的缓存，不传给外部接口，也不放宽黄历参数检查。
fn cache_parameters(args: &Value) -> AppResult<(Value, bool)> {
    let mut parameters = args.clone();
    let fields = parameters
        .as_object_mut()
        .ok_or("INVALID_ARGUMENT: 内容参数须为对象")?;
    if fields.remove("cache") != Some(Value::Bool(true)) {
        return Err("INVALID_ARGUMENT: 新版组件须显式启用公开内容缓存".into());
    }
    let force = match fields.remove("forceRefresh") {
        None => false,
        Some(Value::Bool(value)) => value,
        _ => return Err("INVALID_ARGUMENT: forceRefresh须为布尔值".into()),
    };
    Ok((parameters, force))
}

async fn fetch_public(
    source: String,
    url: String,
    config: network::NetworkConfig,
) -> AppResult<Value> {
    let bytes = network::http_get_bounded(&config, &url, 2 * 1024 * 1024).await?;
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "DATA_UNAVAILABLE: 内容源未返回有效JSON".to_owned())?;
    validate_cached_body(&source, &data)?;
    Ok(json!({"data":data,"source":url,"fetchedAt":now_ms()}))
}

// 检查来源状态和必要数据结构；来源错误或坏数据不作为成功结果进入缓存。
fn validate_cached_body(source: &str, body: &Value) -> AppResult<()> {
    let invalid = || "DATA_UNAVAILABLE: 内容源响应格式变化，未缓存无效数据".to_owned();
    if !body.is_object() {
        return Err(invalid());
    }
    if ["code", "errno"]
        .iter()
        .any(|field| body[*field].is_number() && body[*field].as_f64() != Some(0.0))
        || body["ok"].as_f64() == Some(0.0)
    {
        let detail = body["message"]
            .as_str()
            .or_else(|| body["msg"].as_str())
            .filter(|text| text.len() <= 1000)
            .unwrap_or("来源返回错误状态");
        return Err(format!("DATA_UNAVAILABLE: 内容源拒绝请求：{detail}"));
    }
    let valid = match source {
        "bilibili" => body["list"].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item["keyword"].is_string() && item["show_name"].is_string())
        }),
        "bangumi" => body["result"].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item["date_ts"].is_number() && item["seasons"].is_array())
        }),
        "bilibili-rcmd" => body["data"]["item"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                item["bvid"].is_string()
                    && item["uri"].is_string()
                    && item["title"].is_string()
                    && item["stat"].is_object()
                    && item["owner"].is_object()
            })
        }),
        "cls-telegraph" => body["data"]["l"].is_object(),
        "douyin" => body["data"]["word_list"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["word"].is_string())),
        "tou-tiao" => body["data"].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item["Title"].is_string() && item["ClusterIdStr"].is_string())
        }),
        "weibo" => body["data"]["realtime"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["word"].is_string())),
        "zhihu" => body["data"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                item["target"]["title"].is_string() && item["target"]["url"].is_string()
            })
        }),
        "itime-calendar" => {
            let mut dates = std::collections::HashSet::new();
            body["code"] == 0
                && body["data"].as_array().is_some_and(|items| {
                    items.len() <= 5000
                        && items.iter().all(|item| {
                            let number = |key: &str| {
                                item[key].as_u64().or_else(|| {
                                    item[key]
                                        .as_str()
                                        .filter(|text| {
                                            !text.is_empty()
                                                && text.bytes().all(|byte| byte.is_ascii_digit())
                                        })
                                        .and_then(|text| text.parse().ok())
                                })
                            };
                            let Some((year, month, day)) = number("year")
                                .zip(number("month"))
                                .zip(number("dayOfMonth"))
                                .map(|((y, m), d)| (y, m, d))
                            else {
                                return false;
                            };
                            if !(1..=9999).contains(&year)
                                || !(1..=12).contains(&month)
                                || !(1..=31).contains(&day)
                            {
                                return false;
                            }
                            let Some(date) = chrono::NaiveDate::from_ymd_opt(
                                year as i32,
                                month as u32,
                                day as u32,
                            ) else {
                                return false;
                            };
                            if !dates.insert(date)
                                || item.get("date").is_some_and(|value| {
                                    value.as_str() != Some(date.to_string().as_str())
                                })
                            {
                                return false;
                            }
                            if item.get("status").is_some_and(|value| {
                                !value.is_null() && number("status").is_none_or(|value| value > 2)
                            }) {
                                return false;
                            }
                            [
                                ("term", 128),
                                ("suit", 4096),
                                ("avoid", 4096),
                                ("updateTime", 80),
                            ]
                            .iter()
                            .all(|(key, max)| {
                                item.get(*key).is_none_or(|value| {
                                    value.is_null()
                                        || value
                                            .as_str()
                                            .is_some_and(|text| text.encode_utf16().count() <= *max)
                                })
                            })
                        })
                })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[cfg(windows)]
pub(crate) fn open_external(url: &url::Url) -> AppResult<Value> {
    use std::os::windows::process::CommandExt;
    // 使用参数数组交给系统URL处理器，不经命令解释器。
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url.as_str()])
        .creation_flags(0x08000000)
        .spawn()
        .map_err(err)?;
    Ok(Value::Null)
}
#[cfg(not(windows))]
pub(crate) fn open_external(_url: &url::Url) -> AppResult<Value> {
    Err("CAPABILITY_DENIED: 当前平台尚未实现外部网页".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use serde_json::json;

    #[test]
    fn cache_metadata_does_not_reach_source_parameters_or_relax_calendar_contract() {
        let (args, force) =
            cache_parameters(&json!({"source":"itime-calendar","cache":true,"forceRefresh":true}))
                .unwrap();
        assert!(force);
        assert_eq!(args, json!({"source":"itime-calendar"}));
        assert!(source_url("itime-calendar", &args).is_ok());
        assert!(
            cache_parameters(&json!({"source":"bilibili","cache":true,"forceRefresh":"yes"}))
                .is_err()
        );
        assert!(cache_parameters(&json!({"source":"bilibili"})).is_err());
        let (bad, _) = cache_parameters(
            &json!({"source":"itime-calendar","cache":true,"url":"https://other.test"}),
        )
        .unwrap();
        assert!(source_url("itime-calendar", &bad).is_err());
    }

    #[test]
    fn cache_validation_rejects_provider_errors_and_invalid_calendar_or_hotspot_rows() {
        for (source, body) in [
            (
                "bilibili",
                json!({"list":[{"keyword":"甲","show_name":"甲"}]}),
            ),
            ("bangumi", json!({"code":0,"result":[]})),
            ("bilibili-rcmd", json!({"code":0,"data":{"item":[]}})),
            ("cls-telegraph", json!({"errno":0,"data":{"l":{}}})),
            ("douyin", json!({"data":{"word_list":[]}})),
            ("tou-tiao", json!({"data":[]})),
            ("weibo", json!({"data":{"realtime":[]}})),
            ("zhihu", json!({"data":[]})),
            (
                "itime-calendar",
                json!({"code":0,"data":[{"year":"2026","month":11,"dayOfMonth":18,"status":0}]}),
            ),
        ] {
            assert!(validate_cached_body(source, &body).is_ok(), "{source}");
            assert!(
                validate_cached_body(source, &json!({"code":-403,"message":"访问受限"}))
                    .unwrap_err()
                    .contains("访问受限")
            );
        }
        for (source, body) in [
            ("bilibili", json!({"list":[{"keyword":1}]})),
            ("cls-telegraph", json!({"errno":1,"data":{"l":{}}})),
            (
                "itime-calendar",
                json!({"code":0,"data":[{"year":2026,"month":2,"dayOfMonth":30}]}),
            ),
            (
                "itime-calendar",
                json!({"code":0,"data":[{"year":2026,"month":11,"dayOfMonth":18,"status":3}]}),
            ),
        ] {
            assert!(validate_cached_body(source, &body).is_err());
        }
    }

    #[tokio::test]
    async fn public_cache_uses_one_real_http_read_and_forced_failures_are_not_cached() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for n in 0..6 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                stream.read(&mut request).await.unwrap();
                let (status, body) = match n {
                    1 => ("503 Service Unavailable", "{}".to_owned()),
                    2 => ("200 OK", "{\"list\":[{\"keyword\":1}]}".to_owned()),
                    _ => (
                        "200 OK",
                        format!("{{\"list\":[{{\"keyword\":\"{n}\",\"show_name\":\"{n}\"}}]}}"),
                    ),
                };
                let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let cache = crate::content_cache::Service::default();
        let args = json!({"source":"bilibili"});
        let config = network::NetworkConfig::Direct;
        let read = |force| {
            cache.read(
                "bilibili",
                &args,
                &config,
                force,
                fetch_public("bilibili".into(), url.clone(), config.clone()),
            )
        };
        let start = now_ms();
        let (a, b, c) = tokio::join!(read(false), read(false), read(false));
        let a = a.unwrap();
        assert_eq!(a, b.unwrap());
        assert_eq!(a, c.unwrap());
        assert!(a["fetchedAt"].as_u64().unwrap() >= start);
        assert_eq!(read(false).await.unwrap(), a);
        assert!(read(true).await.unwrap_err().contains("503"));
        assert!(read(false).await.unwrap_err().contains("格式变化"));
        assert_eq!(
            read(false).await.unwrap()["data"]["list"][0]["keyword"],
            "3"
        );
        // 普通HTTP调用者仍逐次取数，不受上面的公开内容缓存影响；行情使用原网络层。
        for expected in ["4", "5"] {
            let bytes = network::http_get_bounded(&config, &url, 2 * 1024 * 1024)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["list"][0]["keyword"], expected);
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn itime_calendar_only_reads_original_public_endpoint_without_custom_parameters() {
        assert_eq!(
            source_url("itime-calendar", &json!({"source":"itime-calendar"})).unwrap(),
            "https://itime.fun/api/v2/event/public/calendar"
        );
        for extra in [
            json!({"source":"itime-calendar","url":"https://example.com"}),
            json!({"source":"itime-calendar","year":2026}),
            json!({"source":"itime-calendar","headers":{}}),
        ] {
            assert!(source_url("itime-calendar", &extra).is_err());
        }
    }

    #[test]
    fn baidu_index_requires_exact_index_market_and_keeps_original_decimal_text() {
        let payload=br#"{"ResultCode":"0","Result":{"stock":[{"code":"000001","type":"stock","market":"ab","exchange":"SZ","name":"other","price":"9.9","ratio":"1.1%"},{"code":"000001","type":"index","market":"ab","exchange":"SH","name":"index","price":"12345678901234567890.125","ratio":"+0.005%","stockStatusInfo":"TRADE","stockStatus":"2"}],"refresh_time":""}}"#;
        let value = parse_baidu_index(payload, "000001").unwrap();
        assert_eq!(value["price"], "12345678901234567890.125");
        assert_eq!(value["ratio"], "0.005");
        assert_eq!(value["stockStatusInfo"], "TRADE");
        assert_eq!(value["sourceTimeText"], "");
        assert!(parse_baidu_index(payload, "399001").is_err());
        assert!(parse_baidu_index(
            &String::from_utf8(payload.to_vec())
                .unwrap()
                .replace("\"exchange\":\"SH\"", "\"exchange\":\"SZ\"")
                .into_bytes(),
            "000001"
        )
        .is_err());
        assert!(
            parse_baidu_index(br#"{"ResultCode":"1","Result":{"stock":[]}}"#, "000001").is_err()
        );
        assert!(source_url("a-index", &json!({"code":"000001&url=x"})).is_err());
        assert!(source_url("a-index", &json!({"code":"HSI"})).is_err());
        assert!(source_url("a-index", &json!({"code":"399006"}))
            .unwrap()
            .contains("wd=399006&skip_login=1"));
    }

    #[test]
    fn gold_quote_keeps_integer_price_scale_and_rejects_wrong_contract() {
        let body=br#"{"rc":0,"data":{"f43":1234567890123456789,"f44":1234567890123456800,"f45":1234567890123456700,"f57":"AU9999","f58":"AU9999","f60":1234567890123456700,"f86":1794657600}}"#;
        let quote = parse_gold_quote(body).unwrap();
        assert_eq!(quote["latestPrice"], "12345678901234567.89");
        assert_eq!(quote["yesterdayClose"], "12345678901234567");
        assert_eq!(quote["timestamp"], 1794657600000_u64);
        assert!(parse_gold_quote(
            &String::from_utf8(body.to_vec())
                .unwrap()
                .replace("AU9999", "AU9995")
                .into_bytes()
        )
        .is_err());
        assert!(parse_gold_quote(br#"{"rc":-1,"data":null}"#).is_err());
        assert!(parse_gold_quote(
            &String::from_utf8(body.to_vec())
                .unwrap()
                .replace("1234567890123456789,", "0,")
                .into_bytes()
        )
        .is_err());
    }

    #[test]
    fn gold_trends_preserve_decimal_text_and_report_missing_or_mismatched_data() {
        let body=br#"{"rc":0,"data":{"code":"AU9999","trends":["2026-11-14 09:30,700,700.12,700,700,1,1,1","2026-11-14 09:31,700,700.13,700,700,1,1,1"]}}"#;
        let data = parse_gold_trends(body).unwrap();
        assert_eq!(data["values"], json!(["700.12", "700.13"]));
        assert_eq!(data["times"][0], "2026-11-14 09:30");
        assert!(parse_gold_trends(
            &String::from_utf8(body.to_vec())
                .unwrap()
                .replace("700.12", "NaN")
                .into_bytes()
        )
        .is_err());
        assert!(parse_gold_trends(
            &String::from_utf8(body.to_vec())
                .unwrap()
                .replace("AU9999", "OTHER")
                .into_bytes()
        )
        .is_err());
        assert_eq!(
            parse_gold_trends(br#"{"rc":0,"data":{"code":"AU9999","trends":[]}}"#).unwrap()
                ["values"],
            json!([])
        );
    }

    #[test]
    fn metal_quotes_keep_decimal_digits_identity_and_source_age_fields() {
        let fixture = br#"{
          "XAUUSD":{"symbol":"XAUUSD","mid":12345678901234567890.1234567,"dayDiffPercent":-0.005,"timestamp":"2026-11-14T12:00:00Z","stale":false,"marketState":"open"},
          "XAGUSD":{"symbol":"XAGUSD","mid":30.2,"dayDiffPercent":1.2,"timestamp":"2026-11-14T12:00:00Z","stale":true,"marketState":"closed"},
          "XCUUSD":{"symbol":"XCUUSD","mid":1.25e1,"dayDiffPercent":0,"timestamp":"2026-11-14T12:00:00Z","stale":false,"marketState":"open"},
          "XPTUSD":{"symbol":"XPTUSD","mid":900,"dayDiffPercent":-1.1,"timestamp":"2026-11-14T12:00:00Z","stale":false,"marketState":"open"}
        }"#;
        let value = parse_metals(fixture).unwrap();
        assert_eq!(value[0]["price"], "12345678901234567890.1234567");
        assert_eq!(value[0]["changePercent"], "-0.005");
        assert_eq!(value[1]["stale"], true);
        assert_eq!(value[1]["marketState"], "closed");
        assert_eq!(value[2]["price"], "12.5");
        let metadata = String::from_utf8(fixture.to_vec()).unwrap().replace("\"marketState\":\"open\"", "\"marketState\":\"open\",\"source\":\"provider\",\"exchange\":\"venue\",\"lastQuoteAt\":\"2026-11-14T11:58:00Z\",\"quoteAgeSeconds\":120");
        let detailed = parse_metals(metadata.as_bytes()).unwrap();
        assert_eq!(detailed[0]["dataSource"], "provider");
        assert_eq!(detailed[0]["exchange"], "venue");
        assert_eq!(detailed[0]["updatedAt"], "2026-11-14T11:58:00Z");
        assert_eq!(detailed[0]["quoteAgeSeconds"], 120);
        let no_state = String::from_utf8(fixture.to_vec())
            .unwrap()
            .replace("\"marketState\":\"open\"", "\"marketState\":null");
        assert_eq!(
            parse_metals(no_state.as_bytes()).unwrap()[0]["marketState"],
            "状态未提供"
        );
        let replaced = String::from_utf8(fixture.to_vec())
            .unwrap()
            .replace("\"symbol\":\"XAUUSD\"", "\"symbol\":\"BTCUSD\"");
        assert!(parse_metals(replaced.as_bytes()).is_err());
        let zero = String::from_utf8(fixture.to_vec())
            .unwrap()
            .replace("\"mid\":900", "\"mid\":0");
        assert!(parse_metals(zero.as_bytes()).is_err());
        let unrepresentable = String::from_utf8(fixture.to_vec())
            .unwrap()
            .replace("\"mid\":900", "\"mid\":0.12345678901234567890123456789");
        assert!(
            parse_metals(unrepresentable.as_bytes()).is_err(),
            "不能静默舍入源报价后冒充精确原值"
        );
        let scientific = String::from_utf8(fixture.to_vec())
            .unwrap()
            .replace("\"mid\":900", "\"mid\":1.12345678901234567890123456789e1");
        assert!(parse_metals(scientific.as_bytes()).is_err());
        assert!(parse_metals(br#"{}"#).is_err());
        assert!(parse_metals(b"not json").is_err());
    }

    #[test]
    fn public_content_requires_declared_capability_current_window_and_live_session() {
        use std::sync::{Arc, Mutex};
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-content-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let preset = crate::presets::find("local.souldesk.widgetjs.note")
            .unwrap()
            .unwrap();
        let staged = widgets::stage_bytes(&store, preset.archive().unwrap()).unwrap();
        widgets::activate(&store, &staged).unwrap();
        let id = widgets::create_instance(&store, "local.souldesk.widgetjs.note").unwrap();
        let sessions = widgets::Sessions::default();
        let session = widgets::open_session(&store, &sessions, "main", &id).unwrap();
        let token = session["token"].as_str().unwrap();
        assert!(authorize(&store, &sessions, "main", token, "content.read").is_err());
        let mut manifest = serde_json::to_value(&staged.manifest).unwrap();
        manifest["capabilities"] = json!(["storage.instance", "content.read", "links.open"]);
        store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_packages SET manifest=?1",
                [manifest.to_string()],
            )
            .unwrap();
        assert!(authorize(&store, &sessions, "main", token, "content.read").is_ok());
        assert!(authorize(&store, &sessions, "other-window", token, "content.read").is_err());
        let preview = Arc::new(Mutex::new(widgets::Preview {
            package: staged,
            settings: json!({}),
            storage: json!({}),
            ready_since: None,
            failure: None,
        }));
        let trial =
            widgets::open_preview(&sessions, "trial-content", "trial-content", preview).unwrap();
        assert!(authorize(
            &store,
            &sessions,
            "trial-content",
            trial["token"].as_str().unwrap(),
            "content.read"
        )
        .is_err());
        sessions.0.lock().unwrap().clear();
        assert!(authorize(&store, &sessions, "main", token, "links.open").is_err());
    }

    #[test]
    fn scoped_public_content_checks_each_preset_source_without_inheriting_other_sources() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-content-scope-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let sessions = widgets::Sessions::default();
        let cases: &[(&str, &[&str], bool)] = &[
            ("hotspot-bangumi", &["bangumi"], true),
            ("hotspot-bilibili", &["bilibili"], true),
            ("hotspot-bilibili-rcmd", &["bilibili-rcmd"], true),
            ("hotspot-cls-telegraph", &["cls-telegraph"], false),
            ("hotspot-douyin", &["douyin"], false),
            ("hotspot-tou-tiao", &["tou-tiao"], true),
            ("hotspot-weibo", &["weibo"], false),
            ("hotspot-zhihu", &["zhihu"], false),
            ("calendar", &["itime-calendar"], false),
            ("calendar-large", &["itime-calendar"], false),
            ("sticker", &[], true),
            ("forex", &["forex"], true),
            ("metal", &["metal"], false),
            ("gold", &["gold", "gold-trends"], false),
            ("stock-medium", &["a-index"], false),
            ("stock-small", &["stock-search", "stock-minute"], false),
            ("stock", &["stock-search-eastmoney", "stock-quote"], false),
            ("coin-small", &["coin-index"], false),
            ("coin-medium", &[], false),
        ];
        for &(name, own_sources, images) in cases {
            let package = format!("local.souldesk.widgetjs.{name}");
            let preset = crate::presets::find(&package).unwrap().unwrap();
            let staged = widgets::stage_bytes(&store, preset.archive().unwrap()).unwrap();
            widgets::activate(&store, &staged).unwrap();
            let id = widgets::create_instance(&store, &package).unwrap();
            let session = widgets::open_session(&store, &sessions, "main", &id).unwrap();
            let token = session["token"].as_str().unwrap();
            for source in cases
                .iter()
                .flat_map(|(_, sources, _)| sources.iter().copied())
            {
                let authorized = authorize_content(
                    &store,
                    &sessions,
                    "main",
                    token,
                    "content.get",
                    Some(source),
                );
                assert_eq!(
                    authorized.is_ok(),
                    own_sources.contains(&source),
                    "{name}，{source}"
                );
            }
            assert_eq!(
                authorize_content(
                    &store,
                    &sessions,
                    "main",
                    token,
                    "content.peek",
                    Some("coin-index")
                )
                .is_ok(),
                name == "coin-small"
            );
            assert!(authorize_content(
                &store,
                &sessions,
                "other",
                token,
                "content.get",
                own_sources.first().copied()
            )
            .is_err());
            let csp = widgets::resource_csp(
                &store,
                &format!("{package}/{}/dist/index.html", staged.digest),
            );
            assert_eq!(
                csp.contains("img-src 'self' data: https:"),
                images,
                "{name}"
            );
            assert!(csp.contains("connect-src 'none'"), "{name}");
        }
    }

    #[test]
    fn content_requests_only_select_fixed_public_endpoints() {
        let fx = source_url("forex", &json!({"from":"USD","to":"CNY"})).unwrap();
        assert_eq!(
            fx,
            "https://finance.pae.baidu.com/api/getrevforeigndata?query=CNYUSD&finClientType=pc"
        );
        assert!(source_url("forex", &json!({"from":"USD&extra=1","to":"CNY"})).is_err());
        assert!(source_url("forex", &json!({"from":"BTC","to":"CNY"})).is_err());
        let bilibili = source_url("bilibili", &json!({"url":"http://127.0.0.1/private"})).unwrap();
        assert_eq!(bilibili, "https://s.search.bilibili.com/main/hotword");
        for source in [
            "bangumi",
            "bilibili-rcmd",
            "cls-telegraph",
            "douyin",
            "tou-tiao",
            "weibo",
            "zhihu",
            "metal",
            "gold",
            "gold-trends",
        ] {
            let url = url::Url::parse(&source_url(source, &json!({})).unwrap()).unwrap();
            assert_eq!(url.scheme(), "https");
        }
        assert!(source_url("https://attacker.example", &json!({})).is_err());
        assert!(source_url("bilibili-rcmd", &json!({"lastShown":"bad&inject=1"})).is_err());
        assert!(source_url("bilibili-rcmd", &json!({"lastShown":"a".repeat(10001)})).is_err());
    }

    #[test]
    fn browser_links_reject_commands_credentials_and_local_addresses() {
        assert!(external_url("https://www.bilibili.com/video/BV123?a=1&b=2").is_ok());
        for value in [
            "file:///C:/test",
            "javascript:alert(1)",
            "https://name:pass@example.com",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://localhost/",
            "https://example.com:444/",
            "http://example.com",
            "https://example.com/\nfile",
        ] {
            assert!(external_url(value).is_err(), "{value}");
        }
    }
}
