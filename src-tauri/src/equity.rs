//! 原股票小组件的固定公开来源与精确分时解析。
use crate::store::{err, AppResult};
use serde_json::{json, Value};
const MARKETS: &[u64] = &[0, 1, 116, 105, 106, 107, 113, 114, 155, 156, 157];
fn valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 32
        && !code.contains("..")
        && code.bytes().any(|c| c.is_ascii_alphanumeric())
        && code
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'^' | b'-'))
}
pub fn source_url(source: &str, args: &Value) -> AppResult<Option<String>> {
    if source == "stock-search-eastmoney" {
        let query = args["query"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 160 && !s.chars().any(char::is_control))
            .ok_or("INVALID_ARGUMENT: 搜索词不能为空或超长")?;
        let mut url =
            url::Url::parse("https://searchapi.eastmoney.com/api/suggest/get").map_err(err)?;
        url.query_pairs_mut()
            .append_pair("input", query)
            .append_pair("type", "14")
            .append_pair("token", "D43BF722C8E33BDC906FB84D85E326E8")
            .append_pair("count", "20");
        return Ok(Some(url.into()));
    }
    if source == "stock-search" {
        let query = args["query"]
            .as_str()
            .map(str::trim)
            .filter(|value| {
                !value.is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
            })
            .ok_or("INVALID_ARGUMENT: 搜索词不能为空或超长")?;
        let mut url =
            url::Url::parse("https://finance.pae.baidu.com/selfselect/sug").map_err(err)?;
        url.query_pairs_mut()
            .append_pair("wd", query)
            .append_pair("skip_login", "1")
            .append_pair("finClientType", "pc");
        return Ok(Some(url.to_string()));
    }
    if source != "stock-minute" && source != "stock-quote" {
        return Ok(None);
    }
    let code = args["code"]
        .as_str()
        .filter(|value| valid_code(value))
        .ok_or("INVALID_ARGUMENT: 证券代码无效")?;
    let market = args["market"]
        .as_u64()
        .filter(|value| MARKETS.contains(value))
        .ok_or("INVALID_ARGUMENT: 证券市场无效")?;
    if source == "stock-quote" {
        return Ok(Some(format!("https://push2.eastmoney.com/api/qt/stock/get?secid={market}.{code}&fields=f43,f57,f58,f59,f60,f86&ut=fa5fd1943c7b386f172d6893dbfba10b&invt=2&fltt=2")));
    }
    let fields = (1..=50)
        .map(|n| format!("f{n}"))
        .collect::<Vec<_>>()
        .join(",");
    Ok(Some(format!("https://push2his.eastmoney.com/api/qt/stock/trends2/get?fields1={fields}&fields2=f51,f52,f53,f54,f55,f56,f57,f58&secid={market}.{code}&klt=1&ndays=1&iscr=0&ut=fa5fd1943c7b386f172d6893dbfba10b")))
}
/// fltt=2返回已带小数的价格；不复制上游再次按位数缩放的错误。
pub fn parse_quote(bytes: &[u8], code: &str, market: u64) -> AppResult<Value> {
    #[derive(serde::Deserialize)]
    struct Data {
        f43: Box<serde_json::value::RawValue>,
        f57: String,
        f58: String,
        f59: u32,
        f60: Option<Box<serde_json::value::RawValue>>,
        f86: Option<u64>,
    }
    #[derive(serde::Deserialize)]
    struct Response {
        rc: i32,
        data: Option<Data>,
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 股票报价格式无效")?;
    if response.rc != 0 {
        return Err("DATA_UNAVAILABLE: 股票报价源返回失败".into());
    }
    let value = response.data.ok_or("DATA_UNAVAILABLE: 股票报价缺失")?;
    if !valid_code(code)
        || value.f57 != code
        || !MARKETS.contains(&market)
        || value.f58.is_empty()
        || value.f58.len() > 128
        || value.f59 > 12
    {
        return Err("DATA_UNAVAILABLE: 股票报价身份或精度无效".into());
    }
    let price = crate::content::source_decimal(&value.f43)?;
    if price <= rust_decimal::Decimal::ZERO {
        return Err("DATA_UNAVAILABLE: 股票价格缺失或无效".into());
    }
    let previous = value
        .f60
        .as_deref()
        .and_then(|raw| crate::content::source_decimal(raw).ok())
        .filter(|v| *v > rust_decimal::Decimal::ZERO);
    if price.scale() > value.f59 || previous.is_some_and(|v| v.scale() > value.f59) {
        return Err("DATA_UNAVAILABLE: 股票数值超出来源声明精度".into());
    }
    let previous = previous.map(|v| v.to_string());
    let time = value
        .f86
        .filter(|value| *value > 0 && *value <= 253402300799);
    // 此公开响应无市场字段，market身份由固定请求secid绑定，不伪称源字段验证。
    Ok(
        json!({"code":code,"marketId":market,"name":value.f58,"decimal":value.f59,"price":price.to_string(),"previousClose":previous,"sourceTime":time,"identityBasis":"request-secid"}),
    )
}
pub fn parse_eastmoney_search(bytes: &[u8]) -> AppResult<Value> {
    let response: Value =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 股票搜索格式无效")?;
    let table = &response["QuotationCodeTable"];
    if table["Status"] != 0 {
        return Err("DATA_UNAVAILABLE: 股票搜索源返回失败".into());
    }
    let rows = table["Data"]
        .as_array()
        .filter(|rows| rows.len() <= 1000)
        .ok_or("DATA_UNAVAILABLE: 股票搜索列表无效")?;
    let mut result = Vec::new();
    for row in rows {
        let (Some(code), Some(name)) = (row["Code"].as_str(), row["Name"].as_str()) else {
            continue;
        };
        let market = row["MktNum"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| row["MktNum"].as_u64());
        let Some(market) = market.filter(|v| MARKETS.contains(v)) else {
            continue;
        };
        if !valid_code(code)
            || name.is_empty()
            || name.len() > 128
            || row["QuoteID"]
                .as_str()
                .is_some_and(|id| id != format!("{market}.{code}"))
        {
            continue;
        }
        if result
            .iter()
            .any(|item: &Value| item["code"] == code && item["marketId"] == market)
        {
            continue;
        }
        let kind = match row["Classify"].as_str() {
            Some("Index") => "index",
            Some("Fund") => "fund",
            Some("Block") => "block",
            _ => "stock",
        };
        result.push(json!({"code":code,"marketId":market,"name":name,"type":kind,"price":null}));
        if result.len() == 50 {
            break;
        }
    }
    Ok(json!(result))
}
fn decimal_text(text: &str) -> AppResult<String> {
    if text.is_empty() || text.len() > 96 {
        return Err("DATA_UNAVAILABLE: 证券数值无效".into());
    }
    let text = text.strip_prefix('+').unwrap_or(text);
    rust_decimal::Decimal::from_str_exact(text)
        .map_err(|_| "DATA_UNAVAILABLE: 证券数值无效或超出范围")?;
    Ok(text.to_owned())
}
fn search_market(exchange: &str) -> Option<u64> {
    match exchange.to_ascii_uppercase().as_str() {
        "SH" => Some(1),
        "SZ" | "BJ" => Some(0),
        "HK" => Some(116),
        "NASDAQ" | "NSD" => Some(105),
        "NYSE" => Some(113),
        "US" => Some(105),
        _ => None,
    }
}
pub fn parse_search(bytes: &[u8]) -> AppResult<Value> {
    let response: Value =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 搜索数据格式无效")?;
    if response["ResultCode"] != json!("0") && response["ResultCode"] != json!(0) {
        return Err("DATA_UNAVAILABLE: 证券搜索返回失败".into());
    }
    let list = response["Result"]["stock"]
        .as_array()
        .filter(|value| value.len() <= 1000)
        .ok_or("DATA_UNAVAILABLE: 搜索列表无效")?;
    let mut result = Vec::new();
    for item in list {
        let (Some(code), Some(name), Some(exchange), Some(kind)) = (
            item["code"].as_str(),
            item["name"].as_str(),
            item["exchange"].as_str(),
            item["type"].as_str(),
        ) else {
            continue;
        };
        let Some(market) = search_market(exchange) else {
            continue;
        };
        if !valid_code(code)
            || name.is_empty()
            || name.len() > 128
            || !["index", "stock", "fund", "block"].contains(&kind)
        {
            continue;
        }
        let price = item["price"]
            .as_str()
            .and_then(|value| decimal_text(value).ok())
            .filter(|value| {
                value
                    .parse::<rust_decimal::Decimal>()
                    .is_ok_and(|value| value > rust_decimal::Decimal::ZERO)
            });
        let ratio = item["ratio"]
            .as_str()
            .and_then(|value| decimal_text(value.strip_suffix('%').unwrap_or(value)).ok());
        result.push(json!({"code":code,"marketId":market,"name":name,"exchange":exchange,"type":kind,"price":price,"ratio":ratio}));
        if result.len() == 50 {
            break;
        }
    }
    Ok(json!(result))
}
pub fn parse_minute(bytes: &[u8], code: &str, market: u64) -> AppResult<Value> {
    #[derive(serde::Deserialize)]
    struct Data {
        code: String,
        market: u64,
        name: String,
        decimal: u32,
        #[serde(rename = "preClose")]
        previous: Option<Box<serde_json::value::RawValue>>,
        trends: Vec<String>,
        status: Option<i64>,
        time: Option<u64>,
    }
    #[derive(serde::Deserialize)]
    struct Response {
        rc: i32,
        data: Option<Data>,
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| "DATA_UNAVAILABLE: 证券分时格式无效")?;
    if response.rc != 0 {
        return Err("DATA_UNAVAILABLE: 证券分时源返回失败".into());
    }
    let value = response.data.ok_or("DATA_UNAVAILABLE: 证券分时缺失")?;
    if value.code != code
        || value.market != market
        || !MARKETS.contains(&market)
        || value.name.is_empty()
        || value.name.len() > 128
        || value.decimal > 12
        || value.trends.len() > 3000
    {
        return Err("DATA_UNAVAILABLE: 证券分时身份、精度或容量无效".into());
    }
    let previous = value
        .previous
        .as_deref()
        .map(crate::content::source_decimal)
        .transpose()?
        .filter(|value| *value > rust_decimal::Decimal::ZERO)
        .map(|value| value.normalize().to_string());
    let mut times = Vec::new();
    let mut values = Vec::new();
    for row in value.trends {
        if row.len() > 512 {
            return Err("DATA_UNAVAILABLE: 证券分时条目超长".into());
        }
        let parts = row.split(',').collect::<Vec<_>>();
        if parts.len() < 3 || parts[0].len() > 40 {
            return Err("DATA_UNAVAILABLE: 证券分时条目无效".into());
        }
        let price = decimal_text(parts[2])?;
        if price.parse::<rust_decimal::Decimal>().map_err(err)? <= rust_decimal::Decimal::ZERO {
            return Err("DATA_UNAVAILABLE: 证券分时价格无效".into());
        }
        times.push(parts[0].to_owned());
        values.push(price);
    }
    Ok(
        json!({"code":code,"marketId":market,"name":value.name,"decimal":value.decimal,"previousClose":previous,"times":times,"values":values,"sourceStatus":value.status,"sourceTime":value.time}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_stock_quote_keeps_fltt2_decimal_price_and_request_identity() {
        let request = source_url("stock-quote", &json!({"code":"600519","market":1}))
            .unwrap()
            .unwrap();
        let url = url::Url::parse(&request).unwrap();
        assert_eq!(url.domain(), Some("push2.eastmoney.com"));
        assert!(url
            .query_pairs()
            .any(|(k, v)| k == "secid" && v == "1.600519"));
        assert!(url.query_pairs().any(|(k, v)| k == "fltt" && v == "2"));
        let body = br#"{"rc":0,"data":{"f43":12345678901234567890.125,"f57":"600519","f58":"fixture","f59":3,"f60":12345678901234567890.115,"f86":1794557495}}"#;
        let result = parse_quote(body, "600519", 1).unwrap();
        assert_eq!(result["price"], "12345678901234567890.125");
        assert_eq!(result["previousClose"], "12345678901234567890.115");
        assert_eq!(result["sourceTime"], 1794557495u64);
        assert_eq!(result["marketId"], 1);
        assert_eq!(result["identityBasis"], "request-secid");
        assert!(parse_quote(body, "000001", 1).is_err());
        assert!(parse_quote(body, "600519", 999).is_err());
        let drift = String::from_utf8(body.to_vec())
            .unwrap()
            .replace("12345678901234567890.125", "0.1234567890123456789012345678");
        assert!(parse_quote(drift.as_bytes(), "600519", 1).is_err());
        for replacement in ["null", "0", "-1", "\"-\""] {
            let invalid = String::from_utf8(body.to_vec())
                .unwrap()
                .replace("12345678901234567890.125", replacement);
            assert!(parse_quote(invalid.as_bytes(), "600519", 1).is_err());
        }
        let no_previous = String::from_utf8(body.to_vec())
            .unwrap()
            .replace("12345678901234567890.115", "0")
            .replace("1794557495", "0");
        let missing = parse_quote(no_previous.as_bytes(), "600519", 1).unwrap();
        assert!(missing["previousClose"].is_null());
        assert!(missing["sourceTime"].is_null());
    }

    #[test]
    fn full_stock_eastmoney_search_preserves_same_code_in_different_markets() {
        let request = source_url("stock-search-eastmoney", &json!({"query":"000001&url=bad"}))
            .unwrap()
            .unwrap();
        let url = url::Url::parse(&request).unwrap();
        assert_eq!(url.domain(), Some("searchapi.eastmoney.com"));
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "input").unwrap().1,
            "000001&url=bad"
        );
        assert!(!url.query_pairs().any(|(k, _)| k == "url"));
        let data=br#"{"QuotationCodeTable":{"Status":0,"Data":[{"Code":"000001","Name":"index","MktNum":"1","QuoteID":"1.000001","Classify":"Index"},{"Code":"000001","Name":"stock","MktNum":"0","QuoteID":"0.000001","Classify":"AStock"},{"Code":"000001","Name":"wrong","MktNum":"1","QuoteID":"0.000001"}]}}"#;
        let result = parse_eastmoney_search(data).unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
        assert_eq!(result[0]["marketId"], 1);
        assert_eq!(result[1]["marketId"], 0);
        assert_eq!(result[0]["name"], "index");
        assert!(result[1]["price"].is_null());
        for source in ["stock-quote", "stock-minute"] {
            assert!(source_url(source, &json!({"code":"600519&other=1","market":1})).is_err());
        }
    }

    #[test]
    fn stock_sources_encode_search_and_preserve_code_market_identity() {
        let search = source_url("stock-search", &json!({"query":"小米&x=1"}))
            .unwrap()
            .unwrap();
        let url = url::Url::parse(&search).unwrap();
        assert_eq!(url.domain(), Some("finance.pae.baidu.com"));
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "wd").unwrap().1,
            "小米&x=1"
        );
        assert!(
            source_url("stock-minute", &json!({"code":"01810","market":116}))
                .unwrap()
                .unwrap()
                .contains("secid=116.01810")
        );
        for args in [
            json!({"code":"01810&url=x","market":116}),
            json!({"code":"AAPL","market":999}),
            json!({"code":"../x","market":1}),
        ] {
            assert!(source_url("stock-minute", &args).is_err());
        }
        assert!(source_url("other", &json!({})).unwrap().is_none());
    }

    #[test]
    fn search_results_do_not_lose_exchange_and_never_invent_prices() {
        let value=parse_search(br#"{"ResultCode":"0","Result":{"stock":[{"code":"000001","exchange":"SH","market":"ab","type":"index","name":"index","price":"3000.1","ratio":"+0.2%"},{"code":"000001","exchange":"SZ","market":"ab","type":"stock","name":"stock","price":"--","ratio":"--"}]}}"#).unwrap();
        assert_eq!(value[0]["marketId"], 1);
        assert_eq!(value[1]["marketId"], 0);
        assert_eq!(value[1]["price"], serde_json::Value::Null);
        assert!(parse_search(br#"{"ResultCode":"1","Result":{"stock":[]}}"#).is_err());
    }

    #[test]
    fn minute_prices_keep_decimal_digits_and_reject_wrong_market_or_invalid_rows() {
        let body=br#"{"rc":0,"data":{"code":"01810","market":116,"name":"Xiaomi","decimal":3,"preClose":12345678901234567890.125,"status":1,"time":1794657600,"trends":["2026-11-14 10:00,1,12345678901234567890.135,1,1,1,1,1"]}}"#;
        let value = parse_minute(body, "01810", 116).unwrap();
        assert_eq!(value["previousClose"], "12345678901234567890.125");
        assert_eq!(value["values"][0], "12345678901234567890.135");
        assert_eq!(value["marketId"], 116);
        assert!(parse_minute(body, "01810", 1).is_err());
        assert!(parse_minute(body, "00700", 116).is_err());
        assert!(parse_minute(
            &String::from_utf8(body.to_vec())
                .unwrap()
                .replace(".135", ".135x")
                .into_bytes(),
            "01810",
            116
        )
        .is_err());
    }
}
