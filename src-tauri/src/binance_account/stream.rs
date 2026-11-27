use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

pub(super) struct Lease {
    identity: String,
    touched: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    stop: CancellationToken,
}

pub(super) fn stop(service: &Service, market: Market) {
    if let Ok(mut leases) = service.streams.lock() {
        if let Some(lease) = leases.remove(&market) {
            lease.stop.cancel();
        }
    }
}
pub(super) fn connected(service: &Service, market: Market, id: &str) -> bool {
    service.streams.lock().ok().is_some_and(|leases| {
        leases
            .get(&market)
            .is_some_and(|lease| lease.identity == id && lease.connected.load(Ordering::Relaxed))
    })
}
pub(super) fn touch(app: &AppHandle, market: Market, profile: &Profile) {
    let core = app.state::<Core>();
    let Ok(mut leases) = core.binance_account.streams.lock() else {
        return;
    };
    if let Some(lease) = leases.get(&market) {
        if lease.identity == profile.id && !lease.stop.is_cancelled() {
            lease.touched.store(now_ms(), Ordering::Relaxed);
            return;
        }
        lease.stop.cancel();
    }
    let touched = Arc::new(AtomicU64::new(now_ms()));
    let connected = Arc::new(AtomicBool::new(false));
    let stop = CancellationToken::new();
    leases.insert(
        market,
        Lease {
            identity: profile.id.clone(),
            touched: touched.clone(),
            connected: connected.clone(),
            stop: stop.clone(),
        },
    );
    drop(leases);
    let app = app.clone();
    let profile = profile.clone();
    tauri::async_runtime::spawn(run(app, market, profile, touched, connected, stop));
}

fn futures_stream(market: Market) -> (&'static str, &'static str) {
    match market {
        Market::Usd => (
            "/fapi/v1/listenKey",
            "wss://fstream.binance.com/private/ws/",
        ),
        Market::Coin => ("/dapi/v1/listenKey", "wss://dstream.binance.com/ws/"),
        Market::Portfolio => ("/papi/v1/listenKey", "wss://fstream.binance.com/pm/ws/"),
        Market::Spot => unreachable!(),
    }
}
async fn listen_key(
    client: &ExchangeClient<'_>,
    market: Market,
    profile: &Profile,
    method: Method,
) -> AppResult<Value> {
    let (path, _) = futures_stream(market);
    let request = client
        .http
        .request(method.clone(), format!("{}{path}", market.base()))
        .header("X-MBX-APIKEY", &profile.api_key);
    let mut diagnostic = RequestDiagnostic::new(client.store, market, &method, path);
    let (status, value) = diagnostic
        .read_json(request, 4096)
        .await
        .map_err(|_| "STREAM_UNAVAILABLE")?;
    let code = value["code"].as_i64().filter(|code| *code < 0);
    if !status.is_success() {
        diagnostic.failed("exchange", "http_error", code);
        return Err("STREAM_UNAVAILABLE".into());
    }
    if code.is_some() {
        diagnostic.failed("exchange", "binance_error", code);
    }
    Ok(value)
}

fn spot_subscription(profile: &Profile) -> Value {
    let now = now_ms();
    let plain = format!("apiKey={}&recvWindow=5000&timestamp={now}", profile.api_key);
    json!({"id":"account-stream","method":"userDataStream.subscribe.signature","params":{
        "apiKey":profile.api_key,"timestamp":now,"recvWindow":5000,"signature":hmac_signature(&profile.secret,&plain)}})
}

async fn invalidate(app: &AppHandle, market: Market, id: &str) {
    let core = app.state::<Core>();
    let mut cache = core.binance_account.cache.lock().await;
    if cache.get(&market).is_some_and(|(owner, _, _)| owner == id) {
        cache.remove(&market);
    }
}

async fn run(
    app: AppHandle,
    market: Market,
    profile: Profile,
    touched: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
    stop: CancellationToken,
) {
    let active = || {
        !stop.is_cancelled() && now_ms().saturating_sub(touched.load(Ordering::Relaxed)) < 30_000
    };
    let mut delay = 2;
    while active() {
        let core = app.state::<Core>();
        if core.quitting.load(Ordering::Relaxed) {
            break;
        }
        let attempt = session(&app, market, &profile, &touched, &connected, &stop);
        let _ = tokio::select! { _=stop.cancelled()=>Ok(()), result=attempt=>result };
        let had_connection = connected.swap(false, Ordering::Relaxed);
        if had_connection {
            invalidate(&app, market, &profile.id).await;
            delay = 2;
        }
        if active() {
            tokio::select! { _=stop.cancelled()=>{}, _=tokio::time::sleep(Duration::from_secs(delay))=>{} }
            delay = (delay * 2).min(30);
        }
    }
    connected.store(false, Ordering::Relaxed);
    stop.cancel();
}
async fn session(
    app: &AppHandle,
    market: Market,
    profile: &Profile,
    touched: &AtomicU64,
    connected: &AtomicBool,
    stop: &CancellationToken,
) -> AppResult<()> {
    let core = app.state::<Core>();
    let network = network_config(&core.store)?;
    let api = client(&core.store)?;
    let endpoint = if market == Market::Spot {
        "wss://ws-api.binance.com/ws-api/v3".to_owned()
    } else {
        let value = listen_key(&api, market, profile, Method::POST).await?;
        let key = value["listenKey"]
            .as_str()
            .filter(|value| validate_key(value))
            .ok_or("STREAM_UNAVAILABLE")?;
        format!("{}{key}", futures_stream(market).1)
    };
    let mut socket = network::connect_ws(&network, &endpoint, stop)
        .await
        .map_err(|_| "STREAM_UNAVAILABLE")?;
    if market == Market::Spot {
        socket
            .send(Message::Text(spot_subscription(profile).to_string().into()))
            .await
            .map_err(|_| "STREAM_UNAVAILABLE")?;
    } else {
        connected.store(true, Ordering::Relaxed);
        invalidate(app, market, &profile.id).await;
    }
    let mut maintenance = tokio::time::interval(Duration::from_secs(4));
    let started = now_ms();
    let mut keepalive = started;
    loop {
        tokio::select! {
            _=stop.cancelled()=>return Ok(()),
            _=maintenance.tick()=>{
                let now=now_ms();
                if now.saturating_sub(touched.load(Ordering::Relaxed))>=30_000 ||
                    core.quitting.load(Ordering::Relaxed) || network_config(&core.store).ok().as_ref()!=Some(&network) {
                    return Ok(());
                }
                if market==Market::Spot && !connected.load(Ordering::Relaxed) && now.saturating_sub(started)>8_000 {
                    return Err("STREAM_UNAVAILABLE".into());
                }
                if market!=Market::Spot && now.saturating_sub(keepalive)>=25*60_000 {
                    listen_key(&api,market,profile,Method::PUT).await?;
                    keepalive=now;
                }
            }
            message=socket.next()=>{
                let Some(Ok(message))=message else {return Err("STREAM_UNAVAILABLE".into());};
                match message {
                    Message::Ping(value)=>socket.send(Message::Pong(value)).await.map_err(|_|"STREAM_UNAVAILABLE")?,
                    Message::Close(_)=>return Ok(()),
                    Message::Text(text)=>{
                        let Ok(value)=serde_json::from_str::<Value>(&text) else{continue};
                        if value["id"]=="account-stream" {
                            if value["status"]!=200 {return Err("STREAM_UNAVAILABLE".into());}
                            connected.store(true,Ordering::Relaxed);
                            invalidate(app,market,&profile.id).await;
                        }
                        let event=value.get("event").unwrap_or(&value)["e"].as_str();
                        if matches!(event,Some("listenKeyExpired"|"eventStreamTerminated")) {return Ok(());}
                        if matches!(event,Some("ORDER_TRADE_UPDATE"|"ALGO_UPDATE"|"executionReport"|"listStatus"))
                            && !stop.is_cancelled() {
                            app.state::<Core>().binance_account.orders_changed(market);
                        }
                        if matches!(event,Some("ACCOUNT_UPDATE"|"ORDER_TRADE_UPDATE"|"ALGO_UPDATE"|"ACCOUNT_CONFIG_UPDATE"|
                            "outboundAccountPosition"|"balanceUpdate"|"executionReport"|"listStatus")) {
                            invalidate(app,market,&profile.id).await;
                        }
                    }
                    _=>{}
                }
            }
        }
    }
}
