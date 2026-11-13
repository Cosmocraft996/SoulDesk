//! 显式恢复验收工具；网络闸门只作用于本次测试客户端。
use crate::{
    network::NetworkConfig,
    store::{err, AppResult},
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

struct Gate {
    port: u16,
    enabled: AtomicBool,
    epoch: Mutex<CancellationToken>,
    cancel: CancellationToken,
    active: AtomicUsize,
}
impl Gate {
    async fn start(upstream: NetworkConfig) -> AppResult<Arc<Self>> {
        upstream.validate()?;
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(err)?;
        let gate = Arc::new(Self {
            port: listener.local_addr().map_err(err)?.port(),
            enabled: AtomicBool::new(true),
            epoch: Mutex::new(CancellationToken::new()),
            cancel: CancellationToken::new(),
            active: AtomicUsize::new(0),
        });
        let server = gate.clone();
        tokio::spawn(async move {
            loop {
                let accepted =
                    tokio::select! {_=server.cancel.cancelled()=>break,r=listener.accept()=>r};
                let Ok((socket, _)) = accepted else {
                    break;
                };
                let epoch = {
                    let token = server.epoch.lock().unwrap_or_else(|e| e.into_inner());
                    if !server.enabled.load(Ordering::SeqCst) {
                        continue;
                    }
                    token.clone()
                };
                server.active.fetch_add(1, Ordering::SeqCst);
                let server = server.clone();
                let upstream = upstream.clone();
                tokio::spawn(async move {
                    tokio::select! {_=server.cancel.cancelled()=>{},_=epoch.cancelled()=>{},_=relay(socket,&upstream)=>{}}
                    server.active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Ok(gate)
    }
    fn offline(&self) -> usize {
        let token = self.epoch.lock().unwrap_or_else(|e| e.into_inner());
        self.enabled.store(false, Ordering::SeqCst);
        let active = self.active.load(Ordering::SeqCst);
        token.cancel();
        active
    }
    fn online(&self) {
        let mut token = self.epoch.lock().unwrap_or_else(|e| e.into_inner());
        *token = CancellationToken::new();
        self.enabled.store(true, Ordering::SeqCst);
    }
    fn stop(&self) {
        self.offline();
        self.cancel.cancel();
    }
}
async fn relay(mut client: TcpStream, upstream: &NetworkConfig) -> AppResult<()> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err("闸门请求头过大".into());
        }
        header.push(client.read_u8().await.map_err(err)?);
    }
    let text = String::from_utf8(header.clone()).map_err(err)?;
    let target = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("CONNECT "))
        .and_then(|s| s.split_whitespace().next())
        .ok_or("闸门只接收CONNECT")?;
    let (host, port) = target.rsplit_once(':').ok_or("闸门目标无效")?;
    let port: u16 = port.parse().map_err(err)?;
    if !["fapi.binance.com", "fstream.binance.com"].contains(&host) || port != 443 {
        return Err("闸门只允许固定币安公开域名及443端口".into());
    }
    let mut remote = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        match upstream {
            NetworkConfig::Http { host, port } => {
                let mut s = TcpStream::connect((host.as_str(), *port))
                    .await
                    .map_err(err)?;
                s.write_all(&header).await.map_err(err)?;
                Ok::<_, String>(s)
            }
            NetworkConfig::Direct => {
                let s = TcpStream::connect((host, port)).await.map_err(err)?;
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .map_err(err)?;
                Ok(s)
            }
            NetworkConfig::Socks5 {
                host: proxy,
                port: proxy_port,
            } => {
                let s = tokio_socks::tcp::Socks5Stream::connect(
                    (proxy.as_str(), *proxy_port),
                    (host, port),
                )
                .await
                .map_err(err)?
                .into_inner();
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .map_err(err)?;
                Ok(s)
            }
        }
    })
    .await
    .map_err(err)??;
    tokio::io::copy_bidirectional(&mut client, &mut remote)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn run(report: std::path::PathBuf, sleep_mode: bool) -> AppResult<()> {
    use crate::{
        alert_store,
        alerts::{Clock, Direction, MarketKey, Mode, PriceKind, Rule},
        market, network,
        store::{now_ms, Store},
    };
    use serde_json::json;
    use std::{
        collections::{HashMap, HashSet},
        time::{Duration, Instant},
    };
    let parent = report
        .parent()
        .filter(|p| p.is_dir())
        .ok_or("报告父目录不存在")?;
    if report.exists() {
        return Err("报告已存在，请使用新的运行标识".into());
    }
    let store = Arc::new(Store::open(
        parent.join(format!("recovery-data-{}", uuid::Uuid::new_v4())),
    )?);
    let observer = if sleep_mode {
        Some(crate::power::Observer::new()?)
    } else {
        None
    };
    let gate = Gate::start(network::system_proxy()?).await?;
    let (feed, rx, tx, mut inputs) = market::Feed::new(NetworkConfig::Http {
        host: "127.0.0.1".into(),
        port: gate.port,
    });
    let client_config = feed.desired.borrow().network.clone();
    let catalog = market::parse_catalog(
        &network::http_get(
            &client_config,
            "https://fapi.binance.com/fapi/v1/exchangeInfo",
            &feed.cancel,
        )
        .await?,
    )?;
    let mut symbols = [
        "BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT", "XRPUSDT", "DOGEUSDT", "ADAUSDT", "LINKUSDT",
        "AVAXUSDT", "LTCUSDT", "BCHUSDT", "DOTUSDT", "SUIUSDT", "TRXUSDT", "NEARUSDT", "UNIUSDT",
        "AAVEUSDT", "ETCUSDT", "FILUSDT", "XLMUSDT",
    ]
    .into_iter()
    .filter(|s| {
        catalog
            .iter()
            .any(|c| c.symbol == *s && c.status == "TRADING")
    })
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for c in &catalog {
        if symbols.len() >= 20 {
            break;
        }
        if c.status == "TRADING" && c.contract_type == "PERPETUAL" && !symbols.contains(&c.symbol) {
            symbols.push(c.symbol.clone());
        }
    }
    let keys = symbols
        .iter()
        .flat_map(|s| {
            [
                MarketKey::binance(s, PriceKind::Last),
                MarketKey::binance(s, PriceKind::Mark),
            ]
        })
        .collect::<HashSet<_>>();
    if keys.len() != 40 {
        gate.stop();
        return Err("不足20个可验证合约".into());
    }
    let rule = Rule {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 1,
        name: "传输中断去重验证".into(),
        key: MarketKey::binance("BTCUSDT", PriceKind::Mark),
        direction: Direction::Up,
        threshold: "0.00000001".into(),
        mode: Mode::Once,
        cooldown_ms: 1000,
        enabled: true,
        notification: false,
        sound: false,
    };
    alert_store::save_rule(&store, &rule, None)?;
    feed.desired.send_modify(|d| d.keys = keys.clone());
    let seen = Arc::new(Mutex::new((0u64, HashMap::new())));
    let fatal = Arc::new(Mutex::new(None::<String>));
    let worker_feed = feed.clone();
    let worker_store = store.clone();
    let worker_seen = seen.clone();
    let worker_fatal = fatal.clone();
    let worker = tokio::spawn(async move {
        let mut engine = alert_store::Engine::default();
        while let Some(mut input) = inputs.recv().await {
            let _guard = worker_feed
                .processing
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if input.observation.generation != worker_feed.generation.load(Ordering::SeqCst) {
                continue;
            }
            let Some(source_now) = worker_feed.source_now() else {
                continue;
            };
            input.context.clock = Clock {
                utc_ms: now_ms(),
                mono_ms: worker_feed.origin.elapsed().as_millis() as u64,
            };
            input.context.source_now_utc_ms = source_now;
            if let Err(e) = engine.process(&worker_store, &input.observation, input.context) {
                *worker_fatal.lock().unwrap() = Some(e);
                worker_feed.cancel.cancel();
                break;
            }
            let mut seen = worker_seen.lock().unwrap();
            if seen.0 != input.observation.generation {
                seen.0 = input.observation.generation;
                seen.1.clear();
            }
            seen.1.insert(
                input.observation.key,
                (
                    input.observation.received_mono_ms,
                    input.observation.source_utc_ms,
                ),
            );
        }
    });
    let runner = tokio::spawn(market::run(feed.clone(), rx, tx));
    let mut value = json!({"phase":if sleep_mode{"system-sleep-core-probe"}else{"live-transport-fault-probe"},"passed":false,"started":now_ms(),"symbols":symbols,"cycles":[],"scope":if sleep_mode{"观察人工Windows休眠/唤醒事件，不主动休眠、不修改电源设置，不强制行情重连；独立核心探针，不是完整桌面程序24小时验收"}else{"仅切断本进程本地闸门中的真实TLS连接，不修改整机网络或系统代理；不是系统休眠测试"},"dataRoot":store.root});
    let persist = |value: &serde_json::Value| {
        std::fs::write(&report, serde_json::to_vec_pretty(value).map_err(err)?).map_err(err)
    };
    persist(&value)?;
    let result=async{
        wait_ready(&feed,&seen,&fatal,0,None).await?;
        if alert_store::list_events(&store,0,100)?.len()!=1{return Err("首次逻辑事件数不为1".into());}
        let count=if sleep_mode{5}else{10};let mut power_sequence=0;
        for cycle in 1..=count{
            let old=feed.generation.load(Ordering::SeqCst);
            let (dropped,start,power)=if let Some(observer)=&observer {
                value["stage"]=json!(format!("等待第{cycle}/5次人工休眠；请从开始菜单选择睡眠，唤醒后等待本轮恢复完成"));persist(&value)?;println!("{}",value["stage"]);
                let(suspend,resume)=wait_resume(observer,power_sequence).await?;power_sequence=resume.sequence;
                tokio::time::timeout(Duration::from_secs(60).saturating_sub(resume.instant.elapsed()),async {
                    let source_now=loop{if let Some(value)=feed.source_now(){break value;}tokio::time::sleep(Duration::from_millis(50)).await;};
                    let elapsed=resume.instant.elapsed().as_millis() as u64;let mono=feed.origin.elapsed().as_millis() as u64;let source=source_now.saturating_sub(elapsed);
                    wait_ready(&feed,&seen,&fatal,0,Some((mono.saturating_sub(elapsed),source))).await
                }).await.map_err(|_|"唤醒后60秒内未恢复全部必要流".to_owned())??;
                (0,resume.instant,json!({"suspend":suspend,"resume":resume}))
            }else{
                let dropped=gate.offline();if dropped==0{return Err("故障注入时没有活动传输，不能记为断网验证".into());}
                let invalidated=tokio::time::timeout(Duration::from_secs(5),async{while feed.generation.load(Ordering::SeqCst)==old{tokio::time::sleep(Duration::from_millis(20)).await;}}).await;
                if invalidated.is_err(){return Err("断开真实传输后未使旧代次失效".into());}
                tokio::time::sleep(Duration::from_millis(1500)).await;gate.online();let start=Instant::now();wait_ready(&feed,&seen,&fatal,old,None).await?;(dropped,start,serde_json::Value::Null)
            };
            let events=alert_store::list_events(&store,0,100)?;let rules=alert_store::list_rules(&store)?;
            if events.len()!=1||!rules[0].1.completed||rules[0].1.sequence!=1{return Err("恢复后单次状态或事件重复".into());}
            let row=json!({"cycle":cycle,"droppedTransports":dropped,"power":power,"restoredKeys":40,"recoveryMs":start.elapsed().as_millis(),"oldGeneration":old,"newGeneration":feed.generation.load(Ordering::SeqCst),"eventCount":events.len(),"completed":rules[0].1.completed,"snapshot":feed.snapshot()?});
            println!("恢复检查 {cycle}/{count}：{}ms，40个价格键，历史1条",row["recoveryMs"]);value["cycles"].as_array_mut().unwrap().push(row);persist(&value)?;
        }Ok::<_,String>(())
    }.await;
    feed.cancel.cancel();
    gate.stop();
    let stopped = matches!(
        tokio::time::timeout(Duration::from_secs(3), runner).await,
        Ok(Ok(()))
    );
    let worker_stopped = matches!(
        tokio::time::timeout(Duration::from_secs(3), worker).await,
        Ok(Ok(()))
    );
    value["finished"] = json!(now_ms());
    value["passed"] = json!(result.is_ok() && stopped && worker_stopped);
    value["failure"] = json!(result.as_ref().err());
    value["readerStopped"] = json!(stopped);
    value["workerStopped"] = json!(worker_stopped);
    persist(&value)?;
    result?;
    if !stopped || !worker_stopped {
        return Err("测试任务未及时退出".into());
    }
    Ok(())
}
async fn wait_ready(
    feed: &crate::market::Feed,
    seen: &Mutex<(
        u64,
        std::collections::HashMap<crate::alerts::MarketKey, (u64, u64)>,
    )>,
    fatal: &Mutex<Option<String>>,
    old: u64,
    after: Option<(u64, u64)>,
) -> AppResult<()> {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            if let Some(e) = fatal.lock().map_err(err)?.clone() {
                return Err(e);
            }
            let ready = {
                let s = seen.lock().map_err(err)?;
                s.0 > old
                    && s.0 == feed.generation.load(Ordering::SeqCst)
                    && s.1.len() == 40
                    && after.is_none_or(|(mono, source)| {
                        s.1.values()
                            .all(|(at, time)| *at >= mono && *time >= source)
                    })
            };
            if ready {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "60秒内未恢复所有必要流：{}",
            feed.snapshot().unwrap_or_default()
        )
    })?
}
async fn wait_resume(
    observer: &crate::power::Observer,
    after: u64,
) -> AppResult<(crate::power::Event, crate::power::Event)> {
    tokio::time::timeout(std::time::Duration::from_secs(2700), async {
        loop {
            let events = observer.latest()?;
            if let Some(suspend) = events.iter().find(|e| e.sequence > after && e.kind == 4) {
                if let Some(resume) = events
                    .iter()
                    .find(|e| e.sequence > suspend.sequence && e.kind == 18)
                {
                    return Ok((suspend.clone(), resume.clone()));
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "45分钟内未观察到实际休眠/唤醒，不计为通过".to_owned())?
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn gate_drops_existing_transport_refuses_offline_attempts_and_recovers() {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = upstream.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            loop {
                let (mut s, _) = upstream.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0; 1024];
                    s.read(&mut buf).await.unwrap();
                    s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
                    while s.read(&mut buf).await.unwrap_or(0) > 0 {}
                });
            }
        });
        let gate = Gate::start(crate::network::NetworkConfig::Http {
            host: "127.0.0.1".into(),
            port,
        })
        .await
        .unwrap();
        let connect = || async {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", gate.port))
                .await
                .unwrap();
            s.write_all(b"CONNECT fstream.binance.com:443 HTTP/1.1\r\nHost: fstream.binance.com:443\r\n\r\n").await.unwrap();
            let mut buf = [0; 64];
            let n = s.read(&mut buf).await.unwrap_or(0);
            (s, n)
        };
        let (mut first, n) = connect().await;
        assert!(n > 0);
        assert!(gate.offline() > 0);
        let mut buf = [0; 8];
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), first.read(&mut buf))
                .await
                .unwrap()
                .unwrap_or(0),
            0
        );
        assert_eq!(connect().await.1, 0);
        gate.online();
        assert!(connect().await.1 > 0);
        gate.stop();
        server.abort();
    }
}
