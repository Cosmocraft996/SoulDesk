use super::*;
use rust_decimal::Decimal;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Request {
    market: Market,
    operation: Operation,
}
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Operation {
    Close {
        symbol: String,
        position_side: String,
        quantity: String,
    },
    Order {
        symbol: String,
        side: String,
        position_side: Option<String>,
        quantity: String,
        price: Option<String>,
        time_in_force: Option<String>,
    },
    Cancel {
        symbol: String,
        order_id: String,
        algorithmic: bool,
    },
    Amend {
        symbol: String,
        order_id: String,
        quantity: String,
        price: Option<String>,
    },
    Setting {
        setting: Setting,
        symbol: Option<String>,
        value: String,
        position_side: Option<String>,
    },
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Setting {
    Leverage,
    PositionMode,
    MarginType,
    MultiAssets,
    AddMargin,
    RemoveMargin,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Check {
    Order {
        symbol: String,
        id: String,
        client: bool,
        expected: Option<String>,
        price: Option<String>,
        quantity: Option<String>,
    },
    Replace {
        symbol: String,
        original: String,
        client_id: String,
    },
    SpotReplaceCancel {
        symbol: String,
        original: String,
    },
    SpotReplaceNew {
        symbol: String,
        old: Value,
    },
    OrderList {
        id: String,
    },
    Algo {
        symbol: String,
        id: String,
    },
    Setting {
        setting: Setting,
        symbol: Option<String>,
        value: String,
        position_side: Option<String>,
    },
}
#[derive(Serialize, Deserialize)]
pub(super) struct Pending {
    profile: String,
    id: String,
    label: String,
    created_at: u64,
    check: Check,
}
struct Plan {
    method: Method,
    path: String,
    params: Params,
    label: String,
    check: Check,
    visible: Option<Decimal>,
}

fn remaining_after_cancel(
    order: &Value,
    symbol: &str,
    id: &str,
    total: Decimal,
) -> AppResult<Decimal> {
    let filled = exchange_number(&order["executedQty"]).filter(|value| *value >= Decimal::ZERO);
    if order["status"] != "CANCELED"
        || order["symbol"] != symbol
        || !equal_id(&order["orderId"], id)
    {
        return Err("未收到旧单最终撤销回执，新单未发送".into());
    }
    let filled = filled.ok_or("旧单回执缺少已成交数量，新单未发送")?;
    if filled >= total {
        return Err("旧单已成交量达到确认的总量，新单未发送".into());
    }
    Ok((total - filled).normalize())
}

fn param<'a>(params: &'a Params, key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value.as_str())
}

fn close_quote_allowed(confirmed: &Plan, latest: &Plan) -> bool {
    if confirmed.path != latest.path
        || !confirmed
            .params
            .iter()
            .filter(|(key, _)| *key != "price")
            .eq(latest.params.iter().filter(|(key, _)| *key != "price"))
    {
        return false;
    }
    let number =
        |plan: &Plan, key| param(&plan.params, key).and_then(|value| value.parse::<Decimal>().ok());
    let Some((price, latest_price)) = number(confirmed, "price").zip(number(latest, "price"))
    else {
        return false;
    };
    let side = param(&confirmed.params, "side");
    let same_price = price == latest_price;
    let better =
        side == Some("SELL") && latest_price > price || side == Some("BUY") && latest_price < price;
    let depth = confirmed.visible.zip(latest.visible);
    let enough = depth
        .zip(number(confirmed, "quantity"))
        .is_some_and(|((before, now), quantity)| now >= before.min(quantity));
    (same_price || better) && enough
}
pub(super) struct Job {
    token: String,
    instance: String,
    at: u64,
    result: Option<AppResult<Value>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JobSelection {
    id: String,
}

pub(super) fn begin(
    app: &AppHandle,
    window: &WebviewWindow,
    token: &str,
    instance: &str,
    request: Request,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let mut jobs = core.binance_account.jobs.lock().map_err(err)?;
    let at = now_ms();
    jobs.retain(|_, job| job.result.is_none() || at.saturating_sub(job.at) < 300_000);
    if jobs.values().any(|job| job.result.is_none()) {
        return Err("BUSY: 另一个交易操作正在确认或处理".into());
    }
    if jobs.len() >= 64 {
        return Err("BUSY: 交易回执暂存容量已满，请等待".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let token = token.to_owned();
    let instance = instance.to_owned();
    jobs.insert(
        id.clone(),
        Job {
            token: token.clone(),
            instance: instance.clone(),
            at,
            result: None,
        },
    );
    drop(jobs);
    let app = app.clone();
    let window = window.clone();
    let job_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let result = execute(&app, &window, &token, &instance, request).await;
        let core = app.state::<Core>();
        if let Ok(mut jobs) = core.binance_account.jobs.lock() {
            if let Some(job) = jobs.get_mut(&job_id) {
                job.at = now_ms();
                job.result = Some(result);
            }
        };
    });
    Ok(json!({"id":id}))
}
pub(super) fn status(
    core: &Core,
    token: &str,
    instance: &str,
    input: JobSelection,
) -> AppResult<Value> {
    let jobs = core.binance_account.jobs.lock().map_err(err)?;
    let job = jobs
        .get(&input.id)
        .filter(|job| job.token == token && job.instance == instance)
        .ok_or("SESSION_EXPIRED: 没有找到该交易确认")?;
    match &job.result {
        None => Ok(json!({"status":"processing"})),
        Some(result) => result.clone(),
    }
}
fn pending_key(market: Market) -> String {
    format!("binance.account.pending/{}", market.key())
}
pub(super) fn pending(store: &Store, market: Market) -> AppResult<Option<Pending>> {
    encrypted_get(store, &pending_key(market))
}
pub(super) fn pending_status(value: Option<&Pending>) -> Value {
    value
        .map(|value| json!({"id":value.id,"label":value.label,"createdAt":value.created_at}))
        .unwrap_or(Value::Null)
}

pub(super) async fn archive_pending(
    app: &AppHandle,
    window: &WebviewWindow,
    market: Market,
    id: &str,
) -> AppResult<()> {
    let core = app.state::<Core>();
    let record = pending(&core.store, market)?
        .filter(|record| record.id == id)
        .ok_or("STATE_CHANGED: 待核对指令已更新，请重新查看")?;
    // 覆盖签名请求的有效窗口及正常网络超时，仍由用户核对交易所最终状态。
    if now_ms().saturating_sub(record.created_at) < 60_000 {
        return Err("RECONCILE_REQUIRED: 请在指令发起一分钟后核对币安记录".into());
    }
    let label = format!("{}\n\n{}\n指令编号 {}\n\n请先在币安核对订单、成交或合约设置。归档后会解除本机交易锁定，原结果仍记为未确认。", market.name(), record.label, record.id);
    if !confirm_action(app, window, "归档待核对指令", "已核对并归档", label).await? {
        return Err("CANCELED: 已取消归档".into());
    }
    let _guard = core.binance_account.writes.lock().await;
    let record = pending(&core.store, market)?
        .filter(|record| record.id == id)
        .ok_or("STATE_CHANGED: 待核对指令已更新，请重新查看")?;
    archive_record(&core.store, market, &record)
}

fn archive_record(store: &Store, market: Market, record: &Pending) -> AppResult<()> {
    let archive = format!("binance.account.archived/{}/{}", market.key(), record.id);
    let raw =
        serde_json::to_vec(&json!({"record":record,"archivedAt":now_ms(),"outcome":"unknown"}))
            .map_err(err)?;
    let cipher = STANDARD.encode(local_dpapi(&raw, archive.as_bytes(), false)?);
    let mut db = store.db.lock().map_err(err)?;
    let tx = db.transaction().map_err(err)?;
    tx.execute(
        "INSERT INTO app_settings(key,value) VALUES(?1,?2)",
        rusqlite::params![archive, cipher],
    )
    .map_err(err)?;
    tx.execute(
        "DELETE FROM app_settings WHERE key=?1",
        [pending_key(market)],
    )
    .map_err(err)?;
    tx.commit().map_err(err)
}

pub(super) fn last_archived(store: &Store, market: Market) -> AppResult<Option<Value>> {
    let key: Option<String> = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT key FROM app_settings WHERE key GLOB ?1 ORDER BY rowid DESC LIMIT 1",
            [format!("binance.account.archived/{}/*", market.key())],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(key) = key else {
        return Ok(None);
    };
    let value: Option<Value> = encrypted_get(store, &key)?;
    Ok(value.map(|value| json!({
        "id":value["record"]["id"],"label":value["record"]["label"],"archivedAt":value["archivedAt"]
    })))
}

fn decimal(value: &str) -> AppResult<Decimal> {
    if value.len() > 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Err("INVALID_ARGUMENT: 金额或数量格式无效".into());
    }
    let n: Decimal = value
        .parse()
        .map_err(|_| "INVALID_ARGUMENT: 金额或数量格式无效")?;
    if n <= Decimal::ZERO || n.scale() > 16 {
        return Err("INVALID_ARGUMENT: 金额或数量必须为正数且符合精度".into());
    }
    Ok(n.normalize())
}
fn exchange_number(value: &Value) -> Option<Decimal> {
    if let Some(text) = value.as_str() {
        text.parse().ok()
    } else {
        value
            .as_number()
            .and_then(|number| number.to_string().parse().ok())
    }
}
fn numeric_id(value: &str) -> AppResult<()> {
    if !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err("INVALID_ARGUMENT: 委托编号无效".into())
    }
}
fn equal_id(value: &Value, id: &str) -> bool {
    value.as_str() == Some(id) || value.as_u64().is_some_and(|n| n.to_string() == id)
}
fn order_path(market: Market) -> String {
    format!("{}/order", market.prefix())
}
fn algo_path(market: Market) -> String {
    format!(
        "{}/{}",
        market.prefix(),
        if market == Market::Portfolio {
            "algo/algoOrder"
        } else {
            "algoOrder"
        }
    )
}
fn active(status: &Value) -> bool {
    matches!(status.as_str(), Some("NEW" | "PARTIALLY_FILLED"))
}

fn spot_list_id(order: &Value) -> AppResult<Option<String>> {
    match order["orderListId"].as_i64() {
        Some(-1) => Ok(None),
        Some(id) if id >= 0 => Ok(Some(id.to_string())),
        _ => Err("DATA_UNAVAILABLE: 缺少可核对的现货订单列表编号".into()),
    }
}

async fn spot_order_list(
    client: &ExchangeClient<'_>,
    profile: &Profile,
    id: &str,
) -> AppResult<Value> {
    read(
        client,
        Market::Spot,
        "/api/v3/orderList",
        &[("orderListId", id.into())],
        profile,
    )
    .await
}
fn open_algo_path(market: Market) -> String {
    format!(
        "{}/{}",
        market.prefix(),
        if market == Market::Portfolio {
            "algo/openAlgoOrders"
        } else {
            "openAlgoOrders"
        }
    )
}
async fn query_order(
    client: &ExchangeClient<'_>,
    market: Market,
    profile: &Profile,
    symbol: &str,
    id: &str,
    by_client: bool,
) -> AppResult<Value> {
    read(
        client,
        market,
        &order_path(market),
        &[
            ("symbol", symbol.into()),
            (
                if by_client {
                    "origClientOrderId"
                } else {
                    "orderId"
                },
                id.into(),
            ),
        ],
        profile,
    )
    .await
}
async fn dual(client: &ExchangeClient<'_>, market: Market, profile: &Profile) -> AppResult<bool> {
    read(
        client,
        market,
        &format!("{}/positionSide/dual", market.prefix()),
        &[],
        profile,
    )
    .await?["dualSidePosition"]
        .as_bool()
        .ok_or("DATA_UNAVAILABLE: 无法核对当前持仓模式".into())
}

async fn position_rows(
    client: &ExchangeClient<'_>,
    market: Market,
    profile: &Profile,
    symbol: &str,
) -> AppResult<Value> {
    let params = if market == Market::Coin {
        vec![]
    } else {
        vec![("symbol", symbol.to_owned())]
    };
    read(client, market, market.positions(), &params, profile).await
}
async fn setting_value(
    client: &ExchangeClient<'_>,
    market: Market,
    profile: &Profile,
    setting: Setting,
    symbol: Option<&str>,
    side: Option<&str>,
) -> AppResult<String> {
    match setting {
        Setting::PositionMode => return Ok(dual(client, market, profile).await?.to_string()),
        Setting::MultiAssets => {
            if market != Market::Usd {
                return Err("INVALID_ARGUMENT: 该账户不支持资产模式切换".into());
            }
            return read(client, market, "/fapi/v1/multiAssetsMargin", &[], profile).await?
                ["multiAssetsMargin"]
                .as_bool()
                .map(|value| value.to_string())
                .ok_or("DATA_UNAVAILABLE: 无法核对资产模式".into());
        }
        _ => {}
    }
    let symbol = symbol.ok_or("INVALID_ARGUMENT: 请填写设置适用合约")?;
    if matches!(setting, Setting::Leverage | Setting::MarginType)
        && matches!(market, Market::Usd | Market::Portfolio)
    {
        let raw = read(
            client,
            market,
            &format!("{}/symbolConfig", market.prefix()),
            &[("symbol", symbol.into())],
            profile,
        )
        .await?;
        let row = raw
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["symbol"] == symbol))
            .ok_or("DATA_UNAVAILABLE: 无法核对该合约设置")?;
        return match setting {
            Setting::Leverage => exchange_number(&row["leverage"]).map(|value| value.to_string()),
            Setting::MarginType => row["marginType"]
                .as_str()
                .map(|value| value.to_ascii_uppercase()),
            _ => None,
        }
        .ok_or("DATA_UNAVAILABLE: 缺少合约设置原值".into());
    }
    let raw = position_rows(client, market, profile, symbol).await?;
    let row = raw
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row["symbol"] == symbol && side.is_none_or(|side| row["positionSide"] == side)
            })
        })
        .ok_or("DATA_UNAVAILABLE: 无法核对该合约仓位设置")?;
    match setting {
        Setting::Leverage => exchange_number(&row["leverage"]).map(|value| value.to_string()),
        Setting::MarginType => row["marginType"].as_str().map(|value| match value {
            "cross" | "CROSS" => "CROSSED".into(),
            value => value.to_ascii_uppercase(),
        }),
        Setting::AddMargin | Setting::RemoveMargin => exchange_number(&row["isolatedWallet"])
            .or_else(|| exchange_number(&row["isolatedMargin"]))
            .map(|value| value.to_string()),
        _ => None,
    }
    .ok_or("DATA_UNAVAILABLE: 缺少合约设置原值".into())
}

async fn prepare(
    client: &ExchangeClient<'_>,
    profile: &Profile,
    market: Market,
    op: &Operation,
    id: &str,
) -> AppResult<Plan> {
    let path = order_path(market);
    match op {
        Operation::Close {
            symbol,
            position_side,
            quantity,
        } => {
            if market == Market::Spot {
                return Err("INVALID_ARGUMENT: 现货请使用卖出委托".into());
            }
            valid_symbol(symbol)?;
            let quantity = decimal(quantity)?;
            if !matches!(position_side.as_str(), "BOTH" | "LONG" | "SHORT") {
                return Err("INVALID_ARGUMENT: 仓位方向无效".into());
            }
            let positions = position_rows(client, market, profile, symbol).await?;
            let amount = positions
                .as_array()
                .and_then(|items| {
                    items.iter().find(|item| {
                        item["symbol"] == *symbol && item["positionSide"] == *position_side
                    })
                })
                .and_then(|item| exchange_number(&item["positionAmt"]))
                .ok_or("DATA_UNAVAILABLE: 没有找到对应的开放仓位")?;
            if amount.abs() < quantity || amount.is_zero() {
                return Err("POSITION_CHANGED: 实际可核对仓位量小于选定平仓量".into());
            }
            let sell = amount.is_sign_positive();
            if matches!(
                (position_side.as_str(), sell),
                ("LONG", false) | ("SHORT", true)
            ) {
                return Err("POSITION_CHANGED: 币安仓位方向与数量不一致".into());
            }
            let (target, book) = market.public_book();
            let requested = std::time::Instant::now();
            let quote = exchange(
                client,
                target,
                Method::GET,
                book,
                &[("symbol", symbol.clone())],
                None,
            )
            .await?;
            if requested.elapsed() > Duration::from_secs(3) {
                return Err("DATA_UNAVAILABLE: 对手档报价响应过慢，请重新获取".into());
            }
            let quote = quote
                .as_array()
                .and_then(|list| list.iter().find(|row| row["symbol"] == *symbol))
                .unwrap_or(&quote);
            let price = exchange_number(&quote[if sell { "bidPrice" } else { "askPrice" }])
                .filter(|value| *value > Decimal::ZERO)
                .ok_or("DATA_UNAVAILABLE: 币安未返回有效对手档价格")?;
            let visible = exchange_number(&quote[if sell { "bidQty" } else { "askQty" }])
                .filter(|value| *value > Decimal::ZERO)
                .ok_or("DATA_UNAVAILABLE: 币安未返回有效对手档数量")?;
            let mut params = vec![
                ("symbol", symbol.clone()),
                ("side", if sell { "SELL" } else { "BUY" }.into()),
                ("positionSide", position_side.clone()),
                ("type", "LIMIT".into()),
                ("timeInForce", "IOC".into()),
                ("price", price.normalize().to_string()),
                ("quantity", quantity.to_string()),
                ("newClientOrderId", id.into()),
                ("newOrderRespType", "RESULT".into()),
            ];
            if position_side == "BOTH" {
                params.push(("reduceOnly", "true".into()));
            }
            let direction = if sell { "卖出平多" } else { "买入平空" };
            let unit = if market == Market::Coin {
                "张"
            } else {
                "基础资产"
            };
            Ok(Plan {method:Method::POST,path,params,
                label:format!("{symbol} {direction}\n仓位侧 {position_side} · {quantity} {unit}\n限价 {price} · 当前对手档数量 {visible}\n立即成交，剩余委托自动撤销。"),
                visible:Some(visible),
                check:Check::Order{symbol:symbol.clone(),id:id.into(),client:true,expected:None,price:None,quantity:None}})
        }
        Operation::Order {
            symbol,
            side,
            position_side,
            quantity,
            price,
            time_in_force,
        } => {
            valid_symbol(symbol)?;
            let quantity = decimal(quantity)?;
            if !matches!(side.as_str(), "BUY" | "SELL") {
                return Err("INVALID_ARGUMENT: 买卖方向无效".into());
            }
            let mut params = vec![
                ("symbol", symbol.clone()),
                ("side", side.clone()),
                ("quantity", quantity.to_string()),
                ("newClientOrderId", id.into()),
            ];
            let mut position_label = String::new();
            if market != Market::Spot {
                let position_side = position_side.as_deref().unwrap_or("BOTH");
                let hedge = dual(client, market, profile).await?;
                if hedge
                    && !matches!(
                        (side.as_str(), position_side),
                        ("BUY", "LONG") | ("SELL", "SHORT")
                    )
                    || !hedge && position_side != "BOTH"
                {
                    return Err(
                        "POSITION_CHANGED: 下单方向和当前持仓模式不匹配，双向减仓请使用仓位平仓"
                            .into(),
                    );
                }
                params.push(("positionSide", position_side.into()));
                position_label = format!(" · 仓位侧 {position_side}");
            }
            let order_type = if let Some(price) = price {
                let price = decimal(price)?;
                let tif = time_in_force.as_deref().unwrap_or("GTC");
                if !matches!(tif, "GTC" | "IOC" | "FOK") {
                    return Err("INVALID_ARGUMENT: 无效委托有效方式".into());
                }
                params.extend([
                    ("type", "LIMIT".into()),
                    ("price", price.to_string()),
                    ("timeInForce", tif.into()),
                ]);
                format!("限价 {price} · {tif}")
            } else {
                params.push(("type", "MARKET".into()));
                "市价 · 根据实际订单簿成交".into()
            };
            let direction = if side == "BUY" { "买入" } else { "卖出" };
            Ok(Plan {
                method: Method::POST,
                visible: None,
                path,
                params,
                label: format!(
                    "{symbol} {direction}{position_label}\n数量 {quantity}\n{order_type}"
                ),
                check: Check::Order {
                    symbol: symbol.clone(),
                    id: id.into(),
                    client: true,
                    expected: None,
                    price: None,
                    quantity: None,
                },
            })
        }
        Operation::Cancel {
            symbol,
            order_id,
            algorithmic,
        } => {
            valid_symbol(symbol)?;
            numeric_id(order_id)?;
            if *algorithmic {
                if market == Market::Spot {
                    return Err("INVALID_ARGUMENT: 现货组合委托请通过原订单撤销".into());
                }
                let result = read(
                    client,
                    market,
                    &open_algo_path(market),
                    &[("symbol", symbol.clone())],
                    profile,
                )
                .await?;
                if !result.as_array().is_some_and(|items| {
                    items.iter().any(|item| {
                        item["symbol"] == *symbol && equal_id(&item["algoId"], order_id)
                    })
                }) {
                    return Err("ORDER_CHANGED: 未找到当前活动条件单".into());
                }
                return Ok(Plan {
                    method: Method::DELETE,
                    visible: None,
                    path: algo_path(market),
                    params: vec![("algoId", order_id.clone())],
                    label: format!("撤销 {symbol} 条件委托 #{order_id}"),
                    check: Check::Algo {
                        symbol: symbol.clone(),
                        id: order_id.clone(),
                    },
                });
            }
            let order = query_order(client, market, profile, symbol, order_id, false).await?;
            if market == Market::Spot {
                if let Some(list_id) = spot_list_id(&order)? {
                    let group = spot_order_list(client, profile, &list_id).await?;
                    let ids = group["orders"]
                        .as_array()
                        .and_then(|orders| {
                            orders
                                .iter()
                                .map(|row| row["orderId"].as_u64())
                                .collect::<Option<Vec<_>>>()
                        })
                        .filter(|ids| {
                            ids.len() >= 2 && order_id.parse().is_ok_and(|id| ids.contains(&id))
                        })
                        .ok_or("DATA_UNAVAILABLE: 未能核对组合委托中的全部订单")?;
                    if group["symbol"] != *symbol
                        || !equal_id(&group["orderListId"], &list_id)
                        || group["listOrderStatus"] != "EXECUTING"
                    {
                        return Err("ORDER_CHANGED: 组合委托已变化".into());
                    }
                    let children = ids
                        .iter()
                        .map(|id| format!("#{id}"))
                        .collect::<Vec<_>>()
                        .join("、");
                    return Ok(Plan {
                        method: Method::DELETE,
                        path: "/api/v3/orderList".into(),
                        params: vec![("symbol", symbol.clone()), ("orderListId", list_id.clone())],
                        label: format!("撤销 {symbol} 整组组合委托 #{list_id}\n涉及 {} 条委托 {children}\n尚未完成的子单将一并撤销或失效。", ids.len()),
                        visible: None,
                        check: Check::OrderList { id: list_id },
                    });
                }
            }
            if !active(&order["status"]) {
                return Err("ORDER_CHANGED: 原委托已不可撤销".into());
            }
            Ok(Plan {
                method: Method::DELETE,
                visible: None,
                path,
                params: vec![("symbol", symbol.clone()), ("orderId", order_id.clone())],
                label: format!(
                    "撤销 {symbol} 委托 #{order_id}\n原数量 {} · 已成交 {}",
                    order["origQty"], order["executedQty"]
                ),
                check: Check::Order {
                    symbol: symbol.clone(),
                    id: order_id.clone(),
                    client: false,
                    expected: Some("CANCELED".into()),
                    price: None,
                    quantity: None,
                },
            })
        }
        Operation::Amend {
            symbol,
            order_id,
            quantity,
            price,
        } => {
            valid_symbol(symbol)?;
            numeric_id(order_id)?;
            let total = decimal(quantity)?;
            let order = query_order(client, market, profile, symbol, order_id, false).await?;
            if !active(&order["status"]) || order["type"] != "LIMIT" {
                return Err("ORDER_CHANGED: 只允许修改当前活动限价单".into());
            }
            let filled =
                exchange_number(&order["executedQty"]).ok_or("DATA_UNAVAILABLE: 缺少已成交数量")?;
            let previous =
                exchange_number(&order["origQty"]).ok_or("DATA_UNAVAILABLE: 缺少原订单数量")?;
            let old_price =
                exchange_number(&order["price"]).ok_or("DATA_UNAVAILABLE: 缺少原订单价格")?;
            let side = order["side"]
                .as_str()
                .filter(|side| matches!(*side, "BUY" | "SELL"))
                .ok_or("DATA_UNAVAILABLE: 原委托方向无效")?;
            if total <= filled {
                return Err("INVALID_ARGUMENT: 新总量必须大于已经成交的数量".into());
            }
            let new_price = price
                .as_deref()
                .map(decimal)
                .transpose()?
                .unwrap_or(old_price);
            if new_price == old_price && total == previous {
                return Err("INVALID_ARGUMENT: 委托价格和数量未变化".into());
            }
            let summary=format!("{symbol} #{order_id} {side}\n原价 {old_price} → 新价 {new_price}\n原总量 {previous} → 新总量 {total} · 已成交 {filled}");
            let mut params = vec![("symbol", symbol.clone())];
            if market == Market::Spot && new_price == old_price && total < previous {
                params.extend([("orderId", order_id.clone()), ("newQty", total.to_string())]);
                Ok(Plan {
                    method: Method::PUT,
                    visible: None,
                    path: "/api/v3/order/amend/keepPriority".into(),
                    params,
                    label: format!("现货减量，保留排队优先级\n{summary}"),
                    check: Check::Order {
                        symbol: symbol.clone(),
                        id: order_id.clone(),
                        client: false,
                        expected: None,
                        price: None,
                        quantity: Some(total.to_string()),
                    },
                })
            } else if market == Market::Spot {
                if spot_list_id(&order)?.is_some() {
                    return Err(
                        "INVALID_ARGUMENT: 组合委托的子单不能单独撤旧重挂，请在币安修改组合策略"
                            .into(),
                    );
                }
                let tif = order["timeInForce"]
                    .as_str()
                    .filter(|v| matches!(*v, "GTC" | "IOC" | "FOK"))
                    .ok_or("INVALID_ARGUMENT: 原委托有效方式不可重挂")?;
                params.extend([
                    ("newClientOrderId", id.into()),
                    ("type", "LIMIT".into()),
                    ("side", side.into()),
                    ("timeInForce", tif.into()),
                    ("price", new_price.to_string()),
                ]);
                Ok(Plan {
                    method: Method::POST,
                    visible: None,
                    path,
                    params,
                    label: format!(
                        "现货撤旧建新\n{summary}\n新单最多 {}，将按撤单时的最终已成交量扣减。\n旧单撤销后，新单仍可能未挂出。",
                        total - filled
                    ),
                    check: Check::SpotReplaceCancel {
                        symbol: symbol.clone(),
                        original: order_id.clone(),
                    },
                })
            } else {
                params.extend([
                    ("orderId", order_id.clone()),
                    ("side", side.into()),
                    ("price", new_price.to_string()),
                    ("quantity", total.to_string()),
                ]);
                Ok(Plan {
                    method: Method::PUT,
                    visible: None,
                    path,
                    params,
                    label: format!("修改合约限价单，订单将重新排队\n{summary}"),
                    check: Check::Order {
                        symbol: symbol.clone(),
                        id: order_id.clone(),
                        client: false,
                        expected: None,
                        price: Some(new_price.to_string()),
                        quantity: Some(total.to_string()),
                    },
                })
            }
        }
        Operation::Setting {
            setting,
            symbol,
            value,
            position_side,
        } => {
            if market == Market::Spot
                || market == Market::Portfolio
                    && matches!(
                        setting,
                        Setting::MarginType | Setting::AddMargin | Setting::RemoveMargin
                    )
            {
                return Err("INVALID_ARGUMENT: 当前账户不支持该项合约设置".into());
            }
            let scope = symbol.as_deref().unwrap_or("账户");
            if let Some(symbol) = symbol {
                valid_symbol(symbol)?;
            }
            if position_side
                .as_deref()
                .is_some_and(|side| !matches!(side, "LONG" | "SHORT" | "BOTH"))
            {
                return Err("INVALID_ARGUMENT: 逐仓方向不合法".into());
            }
            let prior = setting_value(
                client,
                market,
                profile,
                *setting,
                symbol.as_deref(),
                position_side.as_deref(),
            )
            .await?;
            let mut params = Vec::new();
            let (suffix, label, expected) = match setting {
                Setting::Leverage => {
                    let leverage = value
                        .parse::<u8>()
                        .ok()
                        .filter(|value| (1..=125).contains(value))
                        .ok_or("INVALID_ARGUMENT: 杠杆倍数无效")?;
                    params.push(("leverage", leverage.to_string()));
                    ("/leverage", "初始杠杆", leverage.to_string())
                }
                Setting::PositionMode | Setting::MultiAssets => {
                    let enabled = value
                        .parse::<bool>()
                        .map_err(|_| "INVALID_ARGUMENT: 模式值无效")?;
                    if matches!(setting, Setting::PositionMode) {
                        params.push(("dualSidePosition", enabled.to_string()));
                        ("/positionSide/dual", "持仓模式", enabled.to_string())
                    } else {
                        params.push(("multiAssetsMargin", enabled.to_string()));
                        ("/multiAssetsMargin", "资产模式", enabled.to_string())
                    }
                }
                Setting::MarginType => {
                    if !matches!(value.as_str(), "ISOLATED" | "CROSSED") {
                        return Err("INVALID_ARGUMENT: 保证金模式无效".into());
                    }
                    params.push(("marginType", value.clone()));
                    ("/marginType", "保证金模式", value.clone())
                }
                Setting::AddMargin | Setting::RemoveMargin => {
                    if market == Market::Coin {
                        return Err(
                            "CAPABILITY_DENIED: 币本位逐仓钱包缺少独立核对口径，暂不提交".into(),
                        );
                    }
                    let amount = decimal(value)?;
                    let balance = prior
                        .parse::<Decimal>()
                        .map_err(|_| "DATA_UNAVAILABLE: 无法核对逐仓余额")?;
                    let add = matches!(setting, Setting::AddMargin);
                    let expected = if add {
                        balance.checked_add(amount)
                    } else {
                        balance.checked_sub(amount)
                    };
                    let expected = expected
                        .filter(|value| *value >= Decimal::ZERO)
                        .ok_or("INVALID_ARGUMENT: 调整后余额无效")?;
                    let side = position_side
                        .as_deref()
                        .ok_or("INVALID_ARGUMENT: 请选择逐仓方向")?;
                    params.extend([
                        ("type", if add { "1" } else { "2" }.into()),
                        ("amount", amount.to_string()),
                        ("positionSide", side.into()),
                    ]);
                    (
                        "/positionMargin",
                        "逐仓钱包余额",
                        expected.normalize().to_string(),
                    )
                }
            };
            if !matches!(setting, Setting::PositionMode | Setting::MultiAssets) {
                params.push((
                    "symbol",
                    symbol.clone().ok_or("INVALID_ARGUMENT: 缺少适用合约")?,
                ));
            }
            if prior == expected {
                return Err("INVALID_ARGUMENT: 合约设置未变化".into());
            }
            let pretty = |value: &str| match (setting, value) {
                (Setting::PositionMode, "true") => "双向".to_owned(),
                (Setting::PositionMode, "false") => "单向".to_owned(),
                (Setting::MultiAssets, "true") => "多资产".to_owned(),
                (Setting::MultiAssets, "false") => "单资产".to_owned(),
                (Setting::MarginType, "ISOLATED") => "逐仓".to_owned(),
                (Setting::MarginType, "CROSSED") => "全仓".to_owned(),
                _ => value.to_owned(),
            };
            let common = if matches!(setting, Setting::PositionMode) && market != Market::Portfolio
            {
                "\n普通 U 本位和币本位共用此模式。"
            } else {
                ""
            };
            Ok(Plan {
                visible: None,
                method: Method::POST,
                path: format!("{}{suffix}", market.prefix()),
                params,
                label: format!(
                    "{scope} · {label}\n{} → {}{common}",
                    pretty(&prior),
                    pretty(&expected)
                ),
                check: Check::Setting {
                    setting: *setting,
                    symbol: symbol.clone(),
                    value: expected,
                    position_side: position_side.clone(),
                },
            })
        }
    }
}

async fn confirm_action(
    app: &AppHandle,
    window: &WebviewWindow,
    title: &str,
    button: &str,
    label: String,
) -> AppResult<bool> {
    let (send, wait) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(label)
        .title(title)
        .buttons(MessageDialogButtons::OkCancelCustom(
            button.into(),
            "取消".into(),
        ))
        .parent(window)
        .show(move |approved| {
            let _ = send.send(approved);
        });
    wait.await.map_err(|_| "CANCELED: 确认窗口已关闭".into())
}

pub(super) async fn execute(
    app: &AppHandle,
    window: &WebviewWindow,
    token: &str,
    instance: &str,
    request: Request,
) -> AppResult<Value> {
    let core = app.state::<Core>();
    let market = request.market;
    let before = profile(&core.store, market)?;
    if !before.trading {
        return Err("TRADE_DISABLED: 请先在宿主设置授权交易".into());
    }
    if !window.label().starts_with("detail-")
        && !matches!(request.operation, Operation::Close { .. })
    {
        return Err("CAPABILITY_DENIED: 请在账户详情发起该操作".into());
    }
    if pending(&core.store, market)?.is_some() {
        return Err("RECONCILE_REQUIRED: 当前账户有待核对指令".into());
    }
    let id = format!(
        "sd-{}",
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(28)
            .collect::<String>()
    );
    let client = client(&core.store)?;
    let mut plan = prepare(&client, &before, market, &request.operation, &id).await?;
    if authorized(&core, window.label(), token, TRADE)? != instance {
        return Err("SESSION_EXPIRED: 账户组件会话已变更".into());
    }
    if !confirm_action(
        app,
        window,
        "确认发送币安交易指令",
        "确认发送",
        format!("{}\n\n{}", market.name(), plan.label),
    )
    .await?
    {
        return Ok(json!({"status":"canceled"}));
    }
    let _guard = core.binance_account.writes.lock().await;
    if authorized(&core, window.label(), token, TRADE)? != instance {
        return Err("SESSION_EXPIRED: 账户组件会话已变更".into());
    }
    let current = profile(&core.store, market)?;
    if current.id != before.id || current.authorization != before.authorization || !current.trading
    {
        return Err("TRADE_DISABLED: 账户配置或交易授权已变更".into());
    }
    if pending(&core.store, market)?.is_some() {
        return Err("RECONCILE_REQUIRED: 当前账户有待核对指令".into());
    }
    let latest = prepare(&client, &current, market, &request.operation, &id).await?;
    let unchanged = if matches!(request.operation, Operation::Close { .. }) {
        close_quote_allowed(&plan, &latest)
    } else {
        plan.path == latest.path && plan.params == latest.params && plan.label == latest.label
    };
    if !unchanged {
        return Err("STATE_CHANGED: 报价、仓位或委托状态已变化，请重新查看并确认".into());
    }
    if authorized(&core, window.label(), token, TRADE)? != instance {
        return Err("SESSION_EXPIRED: 账户组件会话已变更".into());
    }
    let mut record = Pending {
        profile: current.id.clone(),
        id,
        label: plan.label,
        created_at: now_ms(),
        check: plan.check,
    };
    encrypted_set(&core.store, &pending_key(market), &record)?;
    if let Operation::Amend { quantity, .. } = &request.operation {
        if let Check::SpotReplaceCancel { symbol, original } = &record.check {
            let old = exchange(
                &client,
                market,
                Method::DELETE,
                &plan.path,
                &[("symbol", symbol.clone()), ("orderId", original.clone())],
                Some(&current),
            )
            .await;
            core.binance_account.orders_changed(market);
            core.binance_account.cache.lock().await.remove(&market);
            let old = match old {
                Ok(old) => old,
                Err(error) if error.starts_with("RESULT_UNKNOWN:") => {
                    return Ok(json!({"status":"unknown","pending":pending_status(Some(&record))}));
                }
                Err(error) => {
                    encrypted_clear(&core.store, &pending_key(market))?;
                    return Err(error);
                }
            };
            let remaining = remaining_after_cancel(&old, symbol, original, decimal(quantity)?);
            let permitted =
                authorized(&core, window.label(), token, TRADE).is_ok_and(|id| id == instance);
            if !permitted || remaining.is_err() {
                encrypted_clear(&core.store, &pending_key(market))?;
                let reason = remaining
                    .err()
                    .unwrap_or_else(|| "组件会话已变更，新单未发送".into());
                return Ok(trade_receipt(
                    &record,
                    "partial",
                    json!({"old":old,"newStatus":"notSent","reason":reason}),
                ));
            }
            let remaining = remaining?;
            plan.params.push(("quantity", remaining.to_string()));
            record.check = Check::SpotReplaceNew {
                symbol: symbol.clone(),
                old: old.clone(),
            };
            record.label = format!(
                "{}\n已撤销旧单，正核对数量为 {remaining} 的新委托",
                record.label
            );
            record.created_at = now_ms();
            encrypted_set(&core.store, &pending_key(market), &record)?;
            let new = exchange(
                &client,
                market,
                plan.method,
                &plan.path,
                &plan.params,
                Some(&current),
            )
            .await;
            core.binance_account.orders_changed(market);
            core.binance_account.cache.lock().await.remove(&market);
            if new
                .as_ref()
                .is_err_and(|error| error.starts_with("RESULT_UNKNOWN:"))
            {
                return Ok(json!({"status":"unknown","pending":pending_status(Some(&record))}));
            }
            encrypted_clear(&core.store, &pending_key(market))?;
            return Ok(match new {
                Ok(new) => trade_receipt(&record, "accepted", json!({"old":old,"new":new})),
                Err(reason) => trade_receipt(
                    &record,
                    "partial",
                    json!({"old":old,"newStatus":"rejected","reason":reason}),
                ),
            });
        }
    }
    let response = exchange(
        &client,
        market,
        plan.method,
        &plan.path,
        &plan.params,
        Some(&current),
    )
    .await;
    core.binance_account.orders_changed(market);
    core.binance_account.cache.lock().await.remove(&market);
    match response {
        Ok(value) => {
            encrypted_clear(&core.store, &pending_key(market))?;
            let status = if value["partial"] == true {
                "partial"
            } else {
                "accepted"
            };
            Ok(trade_receipt(&record, status, value))
        }
        Err(error) if error.starts_with("RESULT_UNKNOWN:") => {
            Ok(json!({"status":"unknown","pending":pending_status(Some(&record))}))
        }
        Err(error) => {
            encrypted_clear(&core.store, &pending_key(market))?;
            Err(error)
        }
    }
}

fn trade_receipt(record: &Pending, status: &str, mut value: Value) -> Value {
    redact(&mut value);
    json!({"status":status,"receipt":value,"id":record.id})
}

fn order_evidence(result: AppResult<Value>) -> AppResult<Option<Value>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.starts_with("BINANCE_-2013:") => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) async fn reconcile(
    core: &Core,
    window: &str,
    token: &str,
    market: Market,
) -> AppResult<Value> {
    let _guard = core.binance_account.writes.lock().await;
    let Some(record) = pending(&core.store, market)? else {
        return Ok(json!({"status":"clear"}));
    };
    let profile = profile(&core.store, market)?;
    if profile.id != record.profile {
        return Err("SESSION_EXPIRED: 待核对指令属于原账户凭据".into());
    }
    let client = client(&core.store)?;
    let mut evidence = Value::Null;
    let result = match record.check.clone() {
        Check::Order {
            symbol,
            id,
            client: by_client,
            expected,
            price,
            quantity,
        } => {
            let order = order_evidence(
                query_order(&client, market, &profile, &symbol, &id, by_client).await,
            )?;
            order.filter(|order| {
                expected.is_none_or(|value| order["status"] == value || !active(&order["status"]))
                    && price
                        .is_none_or(|value| exchange_number(&order["price"]) == value.parse().ok())
                    && quantity.is_none_or(|value| {
                        exchange_number(&order["origQty"]) == value.parse().ok()
                    })
            })
        }
        Check::Replace {
            symbol,
            original,
            client_id,
        } => {
            let old = order_evidence(
                query_order(&client, market, &profile, &symbol, &original, false).await,
            )?;
            let new = order_evidence(
                query_order(&client, market, &profile, &symbol, &client_id, true).await,
            )?;
            let done = old.as_ref().is_some_and(|old| old["status"] == "CANCELED") && new.is_some();
            evidence = json!({"old":old,"new":new});
            done.then(|| evidence.clone())
        }
        Check::SpotReplaceCancel { symbol, original } => {
            let old = order_evidence(
                query_order(&client, market, &profile, &symbol, &original, false).await,
            )?;
            old.filter(|order| {
                matches!(
                    order["status"].as_str(),
                    Some("CANCELED" | "FILLED" | "EXPIRED" | "EXPIRED_IN_MATCH" | "REJECTED")
                )
            })
            .map(|old| json!({"old":old,"newStatus":"notSent"}))
        }
        Check::SpotReplaceNew { symbol, old } => {
            let new = order_evidence(
                query_order(&client, market, &profile, &symbol, &record.id, true).await,
            )?;
            let done = new.is_some();
            evidence = json!({"old":old,"new":new});
            done.then(|| evidence.clone())
        }
        Check::OrderList { id } => order_evidence(spot_order_list(&client, &profile, &id).await)?
            .filter(|list| list["listOrderStatus"] == "ALL_DONE"),
        Check::Algo { symbol, id } => {
            let order = read(
                &client,
                market,
                &algo_path(market),
                &[("algoId", id)],
                &profile,
            )
            .await?;
            (order["symbol"] == symbol
                && matches!(
                    order["algoStatus"].as_str(),
                    Some("CANCELED" | "EXPIRED" | "FINISHED")
                ))
            .then_some(order)
        }
        Check::Setting {
            setting,
            symbol,
            value,
            position_side,
        } => {
            let current = setting_value(
                &client,
                market,
                &profile,
                setting,
                symbol.as_deref(),
                position_side.as_deref(),
            )
            .await?;
            (current == value
                || current
                    .parse::<Decimal>()
                    .ok()
                    .zip(value.parse::<Decimal>().ok())
                    .is_some_and(|(a, b)| a == b))
            .then_some(json!({"current":current}))
        }
    };
    authorized(core, window, token, TRADE)?;
    if let Some(mut value) = result {
        encrypted_clear(&core.store, &pending_key(market))?;
        core.binance_account.orders_changed(market);
        core.binance_account.cache.lock().await.remove(&market);
        redact(&mut value);
        Ok(json!({"status":"reconciled","receipt":value,"id":record.id}))
    } else {
        redact(&mut evidence);
        Ok(json!({"status":"unknown","pending":pending_status(Some(&record)),"evidence":evidence}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_replacement_uses_the_completed_cancel_fill() {
        let mut order =
            json!({"symbol":"BTCUSDT","orderId":29,"status":"CANCELED","executedQty":"0.30"});
        assert_eq!(
            remaining_after_cancel(&order, "BTCUSDT", "29", Decimal::ONE)
                .unwrap()
                .to_string(),
            "0.7"
        );
        assert!(remaining_after_cancel(&order, "BTCUSDT", "30", Decimal::ONE).is_err());
        order["executedQty"] = json!("1.00");
        assert!(remaining_after_cancel(&order, "BTCUSDT", "29", Decimal::ONE).is_err());
        order["status"] = json!("PARTIALLY_FILLED");
        order["executedQty"] = json!("0.30");
        assert!(remaining_after_cancel(&order, "BTCUSDT", "29", Decimal::ONE).is_err());
    }

    #[test]
    fn better_close_quotes_preserve_price_limit_side_and_confirmed_fill_capacity() {
        let plan = |side: &str, price: &str, visible: &str| Plan {
            method: Method::POST,
            path: "/fapi/v1/order".into(),
            params: vec![
                ("side", side.into()),
                ("price", price.into()),
                ("quantity", "3".into()),
            ],
            label: String::new(),
            check: Check::OrderList { id: String::new() },
            visible: Some(decimal(visible).unwrap()),
        };
        let original = plan("SELL", "100", "8");
        assert!(close_quote_allowed(&original, &plan("SELL", "100", "3")));
        assert!(close_quote_allowed(&original, &plan("SELL", "101", "3")));
        assert!(!close_quote_allowed(&original, &plan("SELL", "99", "8")));
        assert!(!close_quote_allowed(&original, &plan("SELL", "100", "2")));
        assert!(!close_quote_allowed(&original, &plan("BUY", "100", "8")));
        assert!(close_quote_allowed(
            &plan("BUY", "100", "2"),
            &plan("BUY", "99", "2")
        ));
        assert!(!close_quote_allowed(
            &plan("BUY", "100", "2"),
            &plan("BUY", "101", "8")
        ));
        assert_eq!(param(&original.params, "price"), Some("100"));
    }

    #[test]
    fn absent_order_remains_unconfirmed_and_spot_lists_are_explicit() {
        assert!(order_evidence(Err("BINANCE_-2013: 未找到指定委托".into()))
            .unwrap()
            .is_none());
        assert!(order_evidence(Err("NETWORK_ERROR: 网络不可用".into())).is_err());
        assert!(spot_list_id(&json!({"orderListId":-1})).unwrap().is_none());
        assert_eq!(
            spot_list_id(&json!({"orderListId":300})).unwrap(),
            Some("300".into())
        );
        assert!(spot_list_id(&json!({})).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn archiving_releases_the_lock_and_keeps_the_unknown_record() {
        let root = std::env::temp_dir().join(format!("souldesk-binance-{}", uuid::Uuid::new_v4()));
        let store = Store::open(root).unwrap();
        let record = Pending {
            profile: "sample".into(),
            id: "sd-test".into(),
            label: "测试委托".into(),
            created_at: 1,
            check: Check::OrderList { id: "300".into() },
        };
        encrypted_set(&store, &pending_key(Market::Spot), &record).unwrap();
        assert!(pending(&store, Market::Spot).unwrap().is_some());
        archive_record(&store, Market::Spot, &record).unwrap();
        assert!(pending(&store, Market::Spot).unwrap().is_none());
        let latest = last_archived(&store, Market::Spot).unwrap().unwrap();
        assert_eq!(latest["id"], "sd-test");
        assert_eq!(latest["label"], "测试委托");
        assert!(last_archived(&store, Market::Usd).unwrap().is_none());
    }

    #[test]
    fn order_inputs_preserve_the_client_decimal_and_registration_id() {
        let req: Request = parse(json!({"market":"usd","operation":{
            "kind":"close","symbol":"BTCUSDT","positionSide":"SHORT","quantity":"0.010000"
        }}))
        .unwrap();
        assert!(
            matches!(req.operation,Operation::Close{position_side,quantity,..}
            if position_side=="SHORT" && decimal(&quantity).unwrap().to_string()=="0.01")
        );
        assert!(decimal("0").is_err());
        assert!(decimal("1e-8").is_err());
        assert!(decimal("1&side=SELL").is_err());
        assert!(numeric_id("9223372036854775807").is_ok());
        assert!(numeric_id("12&orderId=5").is_err());
        assert!(parse::<Request>(json!({"market":"usd","operation":{
            "kind":"close","symbol":"BTCUSDT","positionSide":"SHORT","quantity":"1","side":"SELL"
        }}))
        .is_err());
    }
}
