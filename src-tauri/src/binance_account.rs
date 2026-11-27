//! 币安私有接口只在宿主签名，组件不能指定地址、凭据或任意请求。
mod stream;
mod trade;

use crate::{
    ai_credentials::local_dpapi,
    business, network,
    store::{err, now_ms, AppResult, Store},
    widgets, windows, Core,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{
    header::{HeaderMap, RETRY_AFTER},
    Client, Method, RequestBuilder, StatusCode,
};
use ring::hmac;
use rusqlite::OptionalExtension;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

pub(crate) const READ: &str = "binance.account.read";
pub(crate) const TRADE: &str = "binance.account.trade";
pub(crate) const PACKAGE: &str = "local.souldesk.binance-account";
const APPROVED_PACKAGE: &str = "binance.account.approvedPackage";
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
type Params = Vec<(&'static str, String)>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub(super) enum Market {
    Usd,
    Coin,
    Spot,
    Portfolio,
}

impl Market {
    const ALL: [Self; 4] = [Self::Usd, Self::Coin, Self::Spot, Self::Portfolio];
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Usd => "usd",
            Self::Coin => "coin",
            Self::Spot => "spot",
            Self::Portfolio => "portfolio",
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Usd => "U 本位合约",
            Self::Coin => "币本位合约",
            Self::Spot => "现货",
            Self::Portfolio => "统一账户 · U 本位",
        }
    }
    fn base(self) -> &'static str {
        match self {
            Self::Usd => "https://fapi.binance.com",
            Self::Coin => "https://dapi.binance.com",
            Self::Spot => "https://api.binance.com",
            Self::Portfolio => "https://papi.binance.com",
        }
    }
    pub(super) fn prefix(self) -> &'static str {
        match self {
            Self::Usd => "/fapi/v1",
            Self::Coin => "/dapi/v1",
            Self::Spot => "/api/v3",
            Self::Portfolio => "/papi/v1/um",
        }
    }
    fn account(self) -> &'static str {
        match self {
            Self::Usd => "/fapi/v3/account",
            Self::Coin => "/dapi/v1/account",
            Self::Spot => "/api/v3/account",
            Self::Portfolio => "/papi/v1/account",
        }
    }
    pub(super) fn positions(self) -> &'static str {
        match self {
            Self::Usd => "/fapi/v3/positionRisk",
            Self::Coin => "/dapi/v1/positionRisk",
            Self::Portfolio => "/papi/v1/um/positionRisk",
            Self::Spot => unreachable!(),
        }
    }
    fn balances(self) -> &'static str {
        match self {
            Self::Usd => "/fapi/v3/balance",
            Self::Coin => "/dapi/v1/balance",
            Self::Spot => "/api/v3/account",
            Self::Portfolio => "/papi/v1/balance",
        }
    }
    pub(super) fn public_book(self) -> (Self, &'static str) {
        match self {
            Self::Usd | Self::Portfolio => (Self::Usd, "/fapi/v1/ticker/bookTicker"),
            Self::Coin => (Self::Coin, "/dapi/v1/ticker/bookTicker"),
            Self::Spot => (Self::Spot, "/api/v3/ticker/bookTicker"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    id: String,
    #[serde(default)]
    authorization: String,
    api_key: String,
    secret: String,
    trading: bool,
}

#[derive(Default)]
pub(crate) struct Service {
    writes: tokio::sync::Mutex<()>,
    cache: tokio::sync::Mutex<BTreeMap<Market, (String, u64, Value)>>,
    jobs: std::sync::Mutex<BTreeMap<String, trade::Job>>,
    streams: std::sync::Mutex<BTreeMap<Market, stream::Lease>>,
    refresh: [tokio::sync::Mutex<()>; 4],
    order_revisions: [AtomicU64; 4],
}

impl Service {
    fn orders_changed(&self, market: Market) {
        self.order_revisions[market as usize].fetch_add(1, Ordering::Relaxed);
    }
}

fn profile_key(market: Market) -> String {
    format!("binance.account.profile/{}", market.key())
}
fn encrypted_get<T: DeserializeOwned>(store: &Store, key: &str) -> AppResult<Option<T>> {
    let value: Option<String> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    value
        .map(|value| {
            let bytes = STANDARD
                .decode(value)
                .map_err(|_| "CREDENTIAL_ERROR: 币安本地密文无效")?;
            serde_json::from_slice(&local_dpapi(&bytes, key.as_bytes(), true)?)
                .map_err(|_| "CREDENTIAL_ERROR: 币安本地记录格式无效".into())
        })
        .transpose()
}
fn encrypted_set(store: &Store, key: &str, value: &impl Serialize) -> AppResult<()> {
    let raw = serde_json::to_vec(value).map_err(err)?;
    store.save_preference(
        key,
        &STANDARD.encode(local_dpapi(&raw, key.as_bytes(), false)?),
    )
}
fn encrypted_clear(store: &Store, key: &str) -> AppResult<()> {
    store
        .db
        .lock()
        .map_err(err)?
        .execute("DELETE FROM app_settings WHERE key=?1", [key])
        .map_err(err)?;
    Ok(())
}
fn profile(store: &Store, market: Market) -> AppResult<Profile> {
    encrypted_get(store, &profile_key(market))?
        .ok_or("CONFIG_REQUIRED: 请在宿主设置的「数据」中配置币安账户".into())
}
fn same_profile(store: &Store, market: Market, id: &str) -> AppResult<()> {
    if profile(store, market)?.id != id {
        return Err("SESSION_EXPIRED: 币安账户配置已变更".into());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Settings {
    Get,
    Save {
        market: Market,
        #[serde(rename = "apiKey")]
        api_key: String,
        secret: String,
        trading: bool,
    },
    SetTrading {
        market: Market,
        trading: bool,
    },
    Test {
        market: Market,
    },
    Remove {
        market: Market,
    },
    ApprovePackage {
        digest: String,
    },
    ArchivePending {
        market: Market,
        id: String,
    },
}

fn installed_package(store: &Store) -> AppResult<Option<Value>> {
    let row: Option<(String, String)> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT digest,manifest FROM widget_packages WHERE id=?1",
            [PACKAGE],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    row.map(|(digest, manifest)| {
        let manifest: Value = serde_json::from_str(&manifest).map_err(err)?;
        let approved = store.preference(APPROVED_PACKAGE).as_deref() == Some(&digest);
        Ok(json!({"digest":digest,"version":manifest["version"],"approved":approved}))
    })
    .transpose()
}

fn settings_status(store: &Store) -> AppResult<Value> {
    let pending: Vec<_> = Market::ALL
        .iter()
        .filter_map(|&market| {
            trade::pending(store, market).ok().flatten().map(
                |record| json!({"market":market,"record":trade::pending_status(Some(&record))}),
            )
        })
        .collect();
    let archive = Market::ALL
        .iter()
        .filter_map(|&market| {
            trade::last_archived(store, market)
                .ok()
                .flatten()
                .map(|record| json!({"market":market,"record":record}))
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"profiles":statuses(store)?,"package":installed_package(store)?,"pending":pending,"archive":archive}),
    )
}

fn statuses(store: &Store) -> AppResult<Value> {
    let values: Vec<_> = Market::ALL.into_iter().map(|market| {
        let config = encrypted_get::<Profile>(store, &profile_key(market));
        let (configured, trading, error) = match config {
            Ok(value) => (value.is_some(), value.is_some_and(|value| value.trading), false),
            Err(_) => (false, false, true),
        };
        json!({"market":market,"configured":configured,"trading":trading,"credentialError":error})
    }).collect();
    Ok(json!(values))
}

fn validate_key(value: &str) -> bool {
    (16..=256).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

#[tauri::command]
pub(crate) async fn binance_account_settings(
    app: AppHandle,
    core: State<'_, Core>,
    window: WebviewWindow,
    args: Settings,
) -> AppResult<Value> {
    if window.label() != "main" {
        return Err("CAPABILITY_DENIED: 请在宿主管理窗口修改账户配置".into());
    }
    if let Settings::ArchivePending { market, id } = &args {
        trade::archive_pending(&app, &window, *market, id).await?;
        return settings_status(&core.store);
    }
    let _guard = core.binance_account.writes.lock().await;
    match args {
        Settings::Get => {}
        Settings::Save {
            market,
            api_key,
            secret,
            trading,
        } => {
            if !validate_key(&api_key) || !validate_key(&secret) {
                return Err("INVALID_ARGUMENT: 请填写币安 HMAC 格式的 API Key 和密钥".into());
            }
            if trade::pending(&core.store, market)?.is_some() {
                return Err("RECONCILE_REQUIRED: 当前账户有待核对指令，请先查单".into());
            }
            let value = Profile {
                id: uuid::Uuid::new_v4().to_string(),
                authorization: uuid::Uuid::new_v4().to_string(),
                api_key,
                secret,
                trading,
            };
            encrypted_set(&core.store, &profile_key(market), &value)?;
            stream::stop(&core.binance_account, market);
            core.binance_account.cache.lock().await.remove(&market);
        }
        Settings::SetTrading { market, trading } => {
            let mut value = profile(&core.store, market)?;
            if value.trading == trading {
                return settings_status(&core.store);
            }
            value.trading = trading;
            value.authorization = uuid::Uuid::new_v4().to_string();
            encrypted_set(&core.store, &profile_key(market), &value)?;
            core.binance_account.cache.lock().await.remove(&market);
        }
        Settings::Test { market } => {
            let value = profile(&core.store, market)?;
            let client = client(&core.store)?;
            read(&client, market, market.account(), &[], &value).await?;
        }
        Settings::Remove { market } => {
            if trade::pending(&core.store, market)?.is_some() {
                return Err("RECONCILE_REQUIRED: 当前账户有待核对指令，请先查单".into());
            }
            encrypted_clear(&core.store, &profile_key(market))?;
            stream::stop(&core.binance_account, market);
            core.binance_account.cache.lock().await.remove(&market);
        }
        Settings::ApprovePackage { digest } => {
            let installed = installed_package(&core.store)?;
            if installed
                .as_ref()
                .and_then(|value| value["digest"].as_str())
                != Some(&digest)
            {
                return Err("VERSION_INCOMPATIBLE: 组件包已变更，请重新查看摘要".into());
            }
            core.store.save_preference(APPROVED_PACKAGE, &digest)?;
        }
        Settings::ArchivePending { .. } => unreachable!(),
    }
    settings_status(&core.store)
}

fn network_config(store: &Store) -> AppResult<network::NetworkConfig> {
    let settings = store
        .preference("network")
        .unwrap_or_else(|| "{\"mode\":\"system\"}".into());
    business::resolve_network(&serde_json::from_str(&settings).map_err(err)?)
}
pub(super) struct ExchangeClient<'a> {
    http: Client,
    store: &'a Store,
}

pub(super) fn client(store: &Store) -> AppResult<ExchangeClient<'_>> {
    make_client(store, Duration::from_secs(8))
}
fn make_client(store: &Store, timeout: Duration) -> AppResult<ExchangeClient<'_>> {
    let config = network_config(store)?;
    let mut builder = Client::builder()
        .no_proxy()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout);
    if let Some(proxy) = config.proxy_url() {
        builder =
            builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "NETWORK_ERROR: 网络代理无效")?);
    }
    builder
        .build()
        .map(|http| ExchangeClient { http, store })
        .map_err(|_| "NETWORK_ERROR: 无法初始化币安连接".into())
}

fn rate_limit_headers(headers: &HeaderMap) -> Map<String, Value> {
    let mut limits = Map::new();
    if let Some(seconds) = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok()?.parse::<u64>().ok())
    {
        limits.insert("retry-after".into(), json!(seconds));
    }
    for (name, value) in headers {
        let name = name.as_str();
        let metered = [
            "x-mbx-used-weight",
            "x-mbx-order-count",
            "x-sapi-used-ip-weight",
            "x-sapi-used-uid-weight",
        ]
        .iter()
        .copied()
        .any(|prefix| {
            name == prefix
                || name
                    .strip_prefix(prefix)
                    .and_then(|suffix| suffix.strip_prefix('-'))
                    .and_then(|suffix| suffix.split_at_checked(suffix.len().checked_sub(1)?))
                    .is_some_and(|(length, unit)| {
                        (1..=5).contains(&length.len())
                            && length.bytes().all(|byte| byte.is_ascii_digit())
                            && matches!(unit, "s" | "m" | "h" | "d")
                    })
        });
        if metered {
            if let Some(count) = value
                .to_str()
                .ok()
                .and_then(|text| text.parse::<u64>().ok())
            {
                limits.insert(name.into(), json!(count));
            }
        }
        if limits.len() >= 12 {
            break;
        }
    }
    limits
}

struct RequestDiagnostic<'a> {
    store: &'a Store,
    market: Market,
    method: &'a Method,
    path: &'a str,
    started: Instant,
    status: Option<StatusCode>,
    rate_limits: Map<String, Value>,
}

impl<'a> RequestDiagnostic<'a> {
    fn new(store: &'a Store, market: Market, method: &'a Method, path: &'a str) -> Self {
        Self {
            store,
            market,
            method,
            path: path.split(['?', '#']).next().unwrap_or_default(),
            started: Instant::now(),
            status: None,
            rate_limits: Map::new(),
        }
    }

    fn failed(&self, stage: &str, failure: &str, code: Option<i64>) {
        let limit = match self.status.map(|status| status.as_u16()) {
            Some(418) => Some("ip_banned"),
            Some(429) => Some("rate_limited"),
            _ if matches!(code, Some(-1003 | -1015)) => Some("rate_limited"),
            _ if code == Some(-1008) => Some("overloaded"),
            _ => None,
        };
        let event = json!({
            "market":self.market.key(),"method":self.method.as_str(),"path":self.path,
            "stage":stage,"failure":failure,"httpStatus":self.status.map(|status|status.as_u16()),
            "binanceCode":code,"limit":limit,"rateLimitHeaders":self.rate_limits,
            "elapsedMs":self.started.elapsed().as_millis(),
        });
        // 请求 URL、原始错误文本和响应正文可能含有敏感数据。
        if self
            .store
            .log("binance.account.http", &event.to_string())
            .is_err()
        {
            eprintln!("币安接口诊断日志保存失败");
        }
    }

    async fn read_json(
        &mut self,
        request: RequestBuilder,
        max_bytes: usize,
    ) -> Result<(StatusCode, Value), ()> {
        let mut response = request.send().await.map_err(|error| {
            let failure = if error.is_timeout() {
                "timeout"
            } else if error.is_connect() {
                "connect_failed"
            } else {
                "request_failed"
            };
            self.failed("send", failure, None);
        })?;
        let status = response.status();
        self.status = Some(status);
        self.rate_limits = rate_limit_headers(response.headers());
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            self.failed("body", "response_too_large", None);
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            self.failed(
                "body",
                if error.is_timeout() {
                    "timeout"
                } else {
                    "read_failed"
                },
                None,
            );
        })? {
            if max_bytes.saturating_sub(bytes.len()) < chunk.len() {
                self.failed("body", "response_too_large", None);
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| {
            self.failed("decode", "invalid_json", None);
        })?;
        Ok((status, value))
    }
}

fn query(params: &[(&str, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(key, value)| (*key, value.as_str())))
        .finish()
}
fn signed_query(params: &[(&str, String)], secret: &str, at: u64) -> String {
    let mut params = params.to_vec();
    params.push(("timestamp", at.to_string()));
    params.push(("recvWindow", "5000".into()));
    let encoded = query(&params);
    let signature = hmac_signature(secret, &encoded);
    format!("{encoded}&signature={signature}")
}
fn hmac_signature(secret: &str, encoded: &str) -> String {
    let mac = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes()),
        encoded.as_bytes(),
    );
    mac.as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) async fn exchange(
    client: &ExchangeClient<'_>,
    market: Market,
    method: Method,
    path: &str,
    params: &[(&str, String)],
    profile: Option<&Profile>,
) -> AppResult<Value> {
    let mut url = url::Url::parse(market.base()).map_err(|_| "INVALID_SOURCE: 币安地址无效")?;
    url.set_path(path);
    let write = method != Method::GET;
    let data = match profile {
        Some(value) => signed_query(params, &value.secret, now_ms()),
        None => query(params),
    };
    let mut request = if write {
        client
            .http
            .request(method.clone(), url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(data)
    } else {
        if !data.is_empty() {
            url.set_query(Some(&data));
        }
        client.http.get(url)
    };
    if let Some(value) = profile {
        request = request.header("X-MBX-APIKEY", &value.api_key);
    }
    let uncertain = "RESULT_UNKNOWN: 请求结果待核对";
    let unavailable = "NETWORK_ERROR: 币安连接或响应不可用";
    let transport_error = || if write { uncertain } else { unavailable };
    let mut diagnostic = RequestDiagnostic::new(client.store, market, &method, path);
    let (status, value) = diagnostic
        .read_json(request, MAX_RESPONSE)
        .await
        .map_err(|_| transport_error())?;
    let code = value["code"].as_i64();
    if !status.is_success() || code.is_some_and(|code| code < 0) {
        diagnostic.failed(
            "exchange",
            if status.is_success() {
                "binance_error"
            } else {
                "http_error"
            },
            code,
        );
    }
    if write && (status.is_server_error() || matches!(code, Some(-1006 | -1007))) {
        return Err(uncertain.into());
    }
    if status.as_u16() == 409 && write {
        return Ok(json!({"partial":true,"response":value}));
    }
    if !status.is_success() || code.is_some_and(|code| code < 0) {
        let message = match code {
            Some(-2015 | -2014) => "密钥、IP 白名单或账户权限未通过",
            Some(-1021) => "本机时间与币安服务器时间不一致",
            Some(-2013) => "未找到指定委托",
            Some(-4067 | -4068) => "当前存在仓位或挂单，币安拒绝切换持仓模式",
            _ if status.as_u16() == 429 || status.as_u16() == 418 => "币安请求频率受限",
            _ => "币安拒绝该请求，检查账户、请求条件和权限",
        };
        return Err(format!(
            "BINANCE_{}: {message}",
            code.unwrap_or(i64::from(status.as_u16()))
        ));
    }
    Ok(value)
}
pub(super) async fn read(
    client: &ExchangeClient<'_>,
    market: Market,
    path: &str,
    params: &[(&str, String)],
    value: &Profile,
) -> AppResult<Value> {
    exchange(client, market, Method::GET, path, params, Some(value)).await
}

pub(super) fn valid_symbol(symbol: &str) -> AppResult<()> {
    if (3..=32).contains(&symbol.len())
        && symbol
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
    {
        Ok(())
    } else {
        Err("INVALID_ARGUMENT: 交易对格式无效".into())
    }
}

fn redact(value: &mut Value) {
    match value {
        Value::Object(value) => {
            for (key, child) in value {
                if matches!(
                    key.as_str(),
                    "apiKey"
                        | "secret"
                        | "signature"
                        | "listenKey"
                        | "uid"
                        | "email"
                        | "accountAlias"
                        | "address"
                        | "addressTag"
                        | "txId"
                        | "txHash"
                ) {
                    *child = json!("已脱敏");
                } else if matches!(
                    key.as_str(),
                    "orderId" | "algoId" | "orderListId" | "tradeId" | "tranId" | "id"
                ) && child.is_number()
                {
                    *child = json!(child.to_string());
                } else {
                    redact(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}
fn valid_asset(value: &str) -> bool {
    (1..=24).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    market: Market,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct History {
    market: Market,
    section: String,
    symbol: Option<String>,
    start_time: Option<u64>,
    end_time: Option<u64>,
}
fn history_route(input: &History) -> AppResult<(Market, String, Params)> {
    let market = input.market;
    let symbol = input.symbol.clone().unwrap_or_default();
    let prefixed = |path: &str| format!("{}{path}", market.prefix());
    let (market, path, needs_symbol, limit) = match (market, input.section.as_str()) {
        (_, "openOrders") => (market, prefixed("/openOrders"), false, false),
        (Market::Usd | Market::Coin, "openAlgo") => {
            (market, prefixed("/openAlgoOrders"), false, false)
        }
        (Market::Portfolio, "openAlgo") => (market, prefixed("/algo/openAlgoOrders"), false, false),
        (Market::Usd, "algoHistory") => (market, prefixed("/allAlgoOrders"), true, true),
        (Market::Portfolio, "algoHistory") => (market, prefixed("/algo/allAlgoOrders"), true, true),
        (_, "orders") => (market, prefixed("/allOrders"), true, true),
        (Market::Spot, "trades") => (market, prefixed("/myTrades"), true, true),
        (_, "trades") => (market, prefixed("/userTrades"), true, true),
        (Market::Spot, "orderLists") => (market, prefixed("/allOrderList"), false, true),
        (Market::Usd | Market::Coin | Market::Portfolio, "forceOrders") => {
            (market, prefixed("/forceOrders"), false, true)
        }
        (Market::Usd | Market::Coin | Market::Portfolio, "income") => {
            (market, prefixed("/income"), false, true)
        }
        (Market::Spot, "deposits") => (
            market,
            "/sapi/v1/capital/deposit/hisrec".into(),
            false,
            true,
        ),
        (Market::Spot, "withdrawals") => (
            market,
            "/sapi/v1/capital/withdraw/history".into(),
            false,
            true,
        ),
        (Market::Spot, "dividends") => (market, "/sapi/v1/asset/assetDividend".into(), false, true),
        (Market::Spot, "dust") => (market, "/sapi/v1/asset/dribblet".into(), false, false),
        (Market::Usd | Market::Coin | Market::Portfolio, "commission") => {
            (market, prefixed("/commissionRate"), true, false)
        }
        (Market::Spot, "commission") => (market, prefixed("/account/commission"), true, false),
        (Market::Usd | Market::Coin | Market::Portfolio, "positionMode") => {
            (market, prefixed("/positionSide/dual"), false, false)
        }
        (Market::Usd, "configuration") => (market, prefixed("/accountConfig"), false, false),
        (Market::Usd | Market::Portfolio, "symbolConfiguration") => {
            (market, prefixed("/symbolConfig"), true, false)
        }
        (Market::Coin, "leverageBrackets") => {
            (market, "/dapi/v2/leverageBracket".into(), true, false)
        }
        (Market::Usd | Market::Portfolio, "leverageBrackets") => {
            (market, prefixed("/leverageBracket"), true, false)
        }
        _ => return Err("CAPABILITY_DENIED: 当前账户未开放该查询".into()),
    };
    let mut params = Vec::new();
    if needs_symbol
        || !symbol.is_empty()
            && matches!(
                input.section.as_str(),
                "openOrders" | "openAlgo" | "income" | "forceOrders"
            )
    {
        valid_symbol(&symbol)?;
        params.push(("symbol", symbol));
    }
    if limit {
        params.push(("limit", "100".into()));
    }
    if limit {
        if let (Some(start), Some(end)) = (input.start_time, input.end_time) {
            if start >= end {
                return Err("INVALID_ARGUMENT: 历史起止时间无效".into());
            }
        }
        if let Some(start) = input.start_time {
            params.push(("startTime", start.to_string()));
        }
        if let Some(end) = input.end_time {
            params.push(("endTime", end.to_string()));
        }
    }
    Ok((market, path, params))
}

pub(super) fn parse<T: DeserializeOwned>(value: Value) -> AppResult<T> {
    serde_json::from_value(value).map_err(|_| "INVALID_ARGUMENT: 币安请求参数无效".into())
}
fn authorized(core: &Core, window: &str, token: &str, cap: &str) -> AppResult<String> {
    checked_session(core, window, token, cap, true)
}

fn checked_session(
    core: &Core,
    window: &str,
    token: &str,
    cap: &str,
    approval_required: bool,
) -> AppResult<String> {
    widgets::with_capability(&core.store, &core.sessions, window, token, cap, |session| {
        authorize_package(&core.store, session, approval_required)?;
        Ok(session.instance.clone())
    })
}

fn authorize_package(
    store: &Store,
    session: &widgets::Session,
    approval_required: bool,
) -> AppResult<()> {
    let (package, installed, approved): (String, String, Option<String>) = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT p.id,p.digest,(SELECT value FROM app_settings WHERE key=?2)
             FROM widget_instances i JOIN widget_packages p ON p.id=i.package_id WHERE i.id=?1",
            rusqlite::params![session.instance, APPROVED_PACKAGE],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(err)?;
    if package != PACKAGE {
        return Err("CAPABILITY_DENIED: 账户接口只允许币安账户监控组件".into());
    }
    if session.package_digest.as_deref() != Some(&installed) {
        return Err("SESSION_EXPIRED: 组件已更新，请重新加载".into());
    }
    if approval_required && approved.as_deref() != Some(&installed) {
        return Err("PACKAGE_APPROVAL_REQUIRED: 请在宿主设置核对并授权当前组件包".into());
    }
    Ok(())
}

async fn snapshot(core: &Core, market: Market, profile: &Profile) -> AppResult<Value> {
    let _gate = core.binance_account.refresh[market as usize].lock().await;
    let connected = stream::connected(&core.binance_account, market, &profile.id);
    if let Some((id, at, value)) = core.binance_account.cache.lock().await.get(&market) {
        if id == &profile.id
            && now_ms().saturating_sub(*at) < if connected { 12_000 } else { 7_000 }
        {
            return Ok(value.clone());
        }
    }
    let client = make_client(&core.store, Duration::from_secs(5))?;
    let reference = make_client(&core.store, Duration::from_secs(2))?;
    let account = read(&client, market, market.account(), &[], profile);
    let balances = async {
        if market == Market::Spot {
            return Ok(Value::Null);
        }
        read(&client, market, market.balances(), &[], profile).await
    };
    let positions = async {
        if market == Market::Spot {
            return Ok(json!([]));
        }
        read(&client, market, market.positions(), &[], profile).await
    };
    let config = async {
        match market {
            Market::Usd => read(&client, market, "/fapi/v1/accountConfig", &[], profile).await,
            Market::Portfolio => read(&client, market, "/papi/v2/um/account", &[], profile).await,
            _ => Ok(Value::Null),
        }
    };
    let symbol_config = async {
        if matches!(market, Market::Usd | Market::Portfolio) {
            return read(
                &client,
                market,
                &format!("{}/symbolConfig", market.prefix()),
                &[],
                profile,
            )
            .await;
        }
        Ok(Value::Null)
    };
    let contracts = async {
        if market == Market::Coin {
            return exchange(
                &reference,
                market,
                Method::GET,
                "/dapi/v1/exchangeInfo",
                &[],
                None,
            )
            .await;
        }
        Ok(Value::Null)
    };
    let usd_index = async {
        if market != Market::Spot {
            return exchange(
                &reference,
                Market::Usd,
                Method::GET,
                "/fapi/v1/assetIndex",
                &[("symbol", "USDTUSD".into())],
                None,
            )
            .await;
        }
        Ok(Value::Null)
    };
    let (account, positions, balances, config, symbol_config, contracts, usd_index) = tokio::join!(
        account,
        positions,
        balances,
        config,
        symbol_config,
        contracts,
        usd_index
    );
    let account = account?;
    let mut errors = Map::new();
    let mut part = |name: &str, result: AppResult<Value>| match result {
        Ok(value) => value,
        Err(error) => {
            errors.insert(name.into(), json!(error));
            Value::Null
        }
    };
    let positions = part("positions", positions);
    let balances = part("balances", balances);
    let config = part("configuration", config);
    let symbol_config = part("symbolConfiguration", symbol_config);
    let mut assets = Vec::new();
    for items in [
        account["assets"].as_array(),
        account["balances"].as_array(),
        balances.as_array(),
    ]
    .into_iter()
    .flatten()
    {
        for item in items {
            if let Some(asset) = item["asset"]
                .as_str()
                .filter(|value| *value != "USDT" && valid_asset(value))
            {
                if [
                    "walletBalance",
                    "free",
                    "locked",
                    "marginBalance",
                    "umWalletBalance",
                    "balance",
                ]
                .iter()
                .any(|field| {
                    item[field].as_str().is_some_and(|value| {
                        value
                            .parse::<rust_decimal::Decimal>()
                            .is_ok_and(|n| !n.is_zero())
                    })
                }) {
                    assets.push(format!("{asset}USDT"));
                }
            }
        }
    }
    for position in positions.as_array().into_iter().flatten() {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let name = position["marginAsset"].as_str().or_else(|| {
            if market == Market::Coin {
                symbol.split_once("USD").map(|(base, _)| base)
            } else {
                ["USDC", "FDUSD"]
                    .into_iter()
                    .find(|asset| symbol.ends_with(asset))
            }
        });
        if let Some(name) = name.filter(|name| *name != "USDT" && valid_asset(name)) {
            assets.push(format!("{name}USDT"));
        }
    }
    assets.sort();
    assets.dedup();
    assets.truncate(40);
    let mut rates = json!({"USDT":"1"});
    if !assets.is_empty() {
        if let Ok(values) = exchange(
            &reference,
            Market::Spot,
            Method::GET,
            "/api/v3/ticker/price",
            &[],
            None,
        )
        .await
        {
            if let Some(values) = values.as_array() {
                for pair in values {
                    if let Some(symbol) = pair["symbol"]
                        .as_str()
                        .filter(|name| assets.iter().any(|entry| entry == *name))
                    {
                        if let Some(base) = symbol.strip_suffix("USDT") {
                            rates[base] = pair["price"].clone();
                        }
                    }
                }
            }
        }
    }
    if let Ok(value) = usd_index {
        let record = value
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["symbol"] == "USDTUSD"))
            .unwrap_or(&value);
        if record["symbol"] == "USDTUSD" {
            let index = record["index"]
                .as_str()
                .and_then(|value| value.parse::<rust_decimal::Decimal>().ok());
            if let Some(rate) = index
                .and_then(|value| rust_decimal::Decimal::ONE.checked_div(value))
                .filter(|value| value.is_sign_positive())
            {
                rates["USD"] = json!(rate.to_string());
            }
        }
    }
    let mut contract_info = Map::new();
    if let Ok(value) = contracts {
        for item in value["symbols"].as_array().into_iter().flatten() {
            if let Some(symbol) = item["symbol"].as_str().filter(|symbol| {
                positions
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|row| row["symbol"] == *symbol))
            }) {
                contract_info.insert(
                    symbol.into(),
                    json!({"contractSize":item["contractSize"],"marginAsset":item["marginAsset"]}),
                );
            }
        }
    }
    same_profile(&core.store, market, &profile.id)?;
    let at = now_ms();
    let mut result = json!({"market":market,"account":account,"positions":positions,"balances":balances,"configuration":config,
        "contracts":contract_info,"symbolConfiguration":symbol_config,"rates":rates,"fetchedAt":at,
        "errors":errors,"trading":profile.trading,"streamConnected":connected});
    redact(&mut result);
    core.binance_account
        .cache
        .lock()
        .await
        .insert(market, (profile.id.clone(), at, result.clone()));
    Ok(result)
}

pub(crate) async fn gateway(
    app: &AppHandle,
    window: &WebviewWindow,
    token: &str,
    method: &str,
    args: Value,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let cap = if matches!(
        method,
        "binanceAccount.trade" | "binanceAccount.tradeStatus" | "binanceAccount.reconcile"
    ) {
        TRADE
    } else {
        READ
    };
    let instance = checked_session(
        &core,
        window.label(),
        token,
        cap,
        method != "binanceAccount.openSettings",
    )?;
    match method {
        "binanceAccount.status" => statuses(&core.store),
        "binanceAccount.openSettings" => {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || windows::account_settings(&app))
                .await
                .map_err(err)??;
            Ok(Value::Null)
        }
        "binanceAccount.snapshot" => {
            let input: Selection = parse(args)?;
            let value = profile(&core.store, input.market)?;
            let mut result = snapshot(&core, input.market, &value).await?;
            authorized(&core, window.label(), token, READ)?;
            same_profile(&core.store, input.market, &value.id)?;
            stream::touch(app, input.market, &value);
            result["ordersRevision"] = json!(core.binance_account.order_revisions
                [input.market as usize]
                .load(Ordering::Relaxed));
            Ok(result)
        }
        "binanceAccount.history" => {
            let input: History = parse(args)?;
            let (market, path, params) = history_route(&input)?;
            let value = profile(&core.store, market)?;
            let mut result = read(&client(&core.store)?, market, &path, &params, &value).await?;
            authorized(&core, window.label(), token, READ)?;
            same_profile(&core.store, market, &value.id)?;
            redact(&mut result);
            Ok(json!({"data":result,"source":path,"fetchedAt":now_ms()}))
        }
        "binanceAccount.trade" => trade::begin(app, window, token, &instance, parse(args)?),
        "binanceAccount.tradeStatus" => trade::status(&core, token, &instance, parse(args)?),
        "binanceAccount.pending" => {
            let input: Selection = parse(args)?;
            Ok(trade::pending_status(
                trade::pending(&core.store, input.market)?.as_ref(),
            ))
        }
        "binanceAccount.reconcile" => {
            let input: Selection = parse(args)?;
            trade::reconcile(&core, window.label(), token, input.market).await
        }
        _ => Err("CAPABILITY_DENIED: 币安账户方法未开放".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn diagnostic_record(store: &Store) -> (String, Value) {
        let line = std::fs::read_to_string(store.root.join("logs/current.jsonl")).unwrap();
        let record: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(record["category"], "binance.account.http");
        let event = serde_json::from_str(record["message"].as_str().unwrap()).unwrap();
        (line, event)
    }

    #[tokio::test]
    async fn an_html_rate_limit_preserves_status_and_never_logs_secrets() {
        let store =
            Store::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 23\r\nX-MBX-USED-WEIGHT-1M: 2400\r\nX-MBX-ORDER-COUNT-10S: 19\r\nX-SAPI-USED-IP-WEIGHT-1M: 500\r\nX-MBX-USED-WEIGHT-1D: header-secret\r\nX-MBX-APIKEY: response-secret\r\nConnection: close\r\n\r\n<html>body-secret</html>").await.unwrap();
        });
        let http = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let mut trace = RequestDiagnostic::new(
            &store,
            Market::Usd,
            &Method::GET,
            "/fapi/v3/positionRisk?signature=query-secret",
        );
        let request = http
            .get(format!(
                "http://{address}/fapi/v3/positionRisk?signature=query-secret"
            ))
            .header("X-MBX-APIKEY", "request-secret");
        assert!(trace.read_json(request, 4096).await.is_err());
        let (line, event) = diagnostic_record(&store);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("测试服务器未收到请求")
            .unwrap();
        assert!(!line.contains("secret"));
        assert_eq!(event["path"], "/fapi/v3/positionRisk");
        assert_eq!(event["stage"], "decode");
        assert_eq!(event["failure"], "invalid_json");
        assert_eq!(event["httpStatus"], 429);
        assert_eq!(event["limit"], "rate_limited");
        assert_eq!(
            event["rateLimitHeaders"],
            json!({
                "retry-after":23,"x-mbx-used-weight-1m":2400,"x-mbx-order-count-10s":19,"x-sapi-used-ip-weight-1m":500,
            })
        );
    }

    #[tokio::test]
    async fn a_request_timeout_without_headers_has_no_http_status() {
        let store =
            Store::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        let http = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let mut trace =
            RequestDiagnostic::new(&store, Market::Coin, &Method::GET, "/dapi/v1/account");
        assert!(trace
            .read_json(http.get(format!("http://{address}/dapi/v1/account")), 4096)
            .await
            .is_err());
        server.abort();

        let (_, event) = diagnostic_record(&store);
        assert_eq!(event["market"], "coin");
        assert_eq!(event["stage"], "send");
        assert_eq!(event["failure"], "timeout");
        assert_eq!(event["httpStatus"], Value::Null);
        assert_eq!(event["limit"], Value::Null);
        assert_eq!(event["rateLimitHeaders"], json!({}));
    }

    #[test]
    fn package_approval_follows_the_installed_digest_and_loaded_session() {
        let root = std::env::temp_dir().join(format!("souldesk-binance-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let original = "a".repeat(64);
        let replacement = "b".repeat(64);
        let package = json!({
            "schemaVersion":1,"id":PACKAGE,"name":"币安账户测试","version":"0.1.1",
            "hostApiMajor":1,"loadCheck":1,"entry":"dist/index.html",
            "defaultSize":{"width":450,"height":490},"minSize":{"width":250,"height":280},
            "capabilities":[READ,TRADE],"settingsSchema":{"type":"object","properties":{},"additionalProperties":false}
        });
        store.db.lock().unwrap().execute(
            "INSERT INTO widget_packages(id,manifest,directory,digest) VALUES(?1,?2,'unused',?3)",
            rusqlite::params![PACKAGE, package.to_string(), original],
        ).unwrap();
        let instance = widgets::create_instance(&store, PACKAGE).unwrap();
        let sessions = widgets::Sessions::default();
        let window = format!("detail-{instance}");
        let token =
            widgets::open_display_session(&store, &sessions, &window, &instance, Some(&original))
                .unwrap()["token"]
                .as_str()
                .unwrap()
                .to_owned();
        let check = |token: &str, approval| {
            widgets::with_capability(&store, &sessions, &window, token, READ, |session| {
                authorize_package(&store, session, approval)
            })
        };
        assert!(check(&token, false).is_ok());
        assert!(check(&token, true)
            .unwrap_err()
            .starts_with("PACKAGE_APPROVAL_REQUIRED:"));
        store.save_preference(APPROVED_PACKAGE, &original).unwrap();
        assert!(check(&token, true).is_ok());

        store
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE widget_packages SET digest=?1 WHERE id=?2",
                rusqlite::params![replacement, PACKAGE],
            )
            .unwrap();
        assert!(check(&token, true)
            .unwrap_err()
            .starts_with("SESSION_EXPIRED:"));
        let token = widgets::open_display_session(
            &store,
            &sessions,
            &window,
            &instance,
            Some(&replacement),
        )
        .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(check(&token, true)
            .unwrap_err()
            .starts_with("PACKAGE_APPROVAL_REQUIRED:"));
        store
            .save_preference(APPROVED_PACKAGE, &replacement)
            .unwrap();
        assert!(check(&token, true).is_ok());
    }

    #[test]
    fn request_signature_encodes_params_before_signing() {
        assert_eq!(
            hmac_signature("key", "The quick brown fox jumps over the lazy dog"),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        let encoded = signed_query(&[("symbol", "BTCUSDT".into())], "example", 4711);
        let (params, signature) = encoded.rsplit_once("&signature=").unwrap();
        assert_eq!(params, "symbol=BTCUSDT&timestamp=4711&recvWindow=5000");
        assert_eq!(signature, hmac_signature("example", params));
    }

    #[test]
    fn read_queries_are_fixed_to_market_and_do_not_allow_authenticated_custom_addresses() {
        assert!(parse::<History>(
            json!({"market":"usd","section":"orders","url":"https://example.org"})
        )
        .is_err());
        let req: History =
            parse(json!({"market":"portfolio","section":"openAlgo","symbol":"BTCUSDT"})).unwrap();
        let (market, path, params) = history_route(&req).unwrap();
        assert_eq!(market, Market::Portfolio);
        assert_eq!(path, "/papi/v1/um/algo/openAlgoOrders");
        assert_eq!(params, vec![("symbol", "BTCUSDT".into())]);
        let forbidden: History = parse(json!({"market":"spot","section":"withdraw"})).unwrap();
        assert!(history_route(&forbidden).is_err());
    }

    #[test]
    fn response_keeps_large_order_ids_and_masks_account_identifiers() {
        let mut value = json!({"uid":8,"items":[{"orderId":9223372036854775807u64,"price":"0.010000","address":"example"}]});
        redact(&mut value);
        assert_eq!(value["items"][0]["orderId"], "9223372036854775807");
        assert_eq!(value["items"][0]["price"], "0.010000");
        assert_eq!(value["items"][0]["address"], "已脱敏");
        assert_eq!(value["uid"], "已脱敏");
    }

    #[cfg(windows)]
    #[test]
    fn private_records_use_their_scope_as_dpapi_entropy() {
        let raw = b"synthetic-account-data";
        let cipher = local_dpapi(raw, b"binance.account.profile/usd", false).unwrap();
        assert_ne!(cipher, raw);
        assert_eq!(
            local_dpapi(&cipher, b"binance.account.profile/usd", true).unwrap(),
            raw
        );
        assert!(local_dpapi(&cipher, b"binance.account.profile/coin", true).is_err());
    }
}
