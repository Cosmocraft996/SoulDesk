use crate::store::{err, AppResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum NetworkConfig {
    Direct,
    Http { host: String, port: u16 },
    Socks5 { host: String, port: u16 },
}
impl NetworkConfig {
    pub fn validate(&self) -> AppResult<()> {
        match self {
            Self::Direct => Ok(()),
            Self::Http { host, port } | Self::Socks5 { host, port } => {
                if host.is_empty()
                    || host.len() > 253
                    || host.contains(['/', '\\', '@', ':', ' ', '\r', '\n'])
                    || *port == 0
                {
                    Err("代理主机或端口无效；当前版本不接收代理凭证".into())
                } else {
                    Ok(())
                }
            }
        }
    }
    fn proxy_url(&self) -> Option<String> {
        match self {
            Self::Direct => None,
            Self::Http { host, port } => Some(format!("http://{host}:{port}")),
            Self::Socks5 { host, port } => Some(format!("socks5h://{host}:{port}")),
        }
    }
}

pub async fn http_get(
    config: &NetworkConfig,
    url: &str,
    cancel: &CancellationToken,
) -> AppResult<String> {
    config.validate()?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15));
    if let Some(proxy) = config.proxy_url() {
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(err)?);
    }
    let client = builder.build().map_err(err)?;
    tokio::select! { biased; _=cancel.cancelled()=>Err("CANCELLED".into()), result=async { client.get(url).send().await.map_err(err)?.error_for_status().map_err(err)?.text().await.map_err(err) }=>result }
}

async fn tcp(config: &NetworkConfig, host: &str, port: u16) -> AppResult<TcpStream> {
    match config {
        NetworkConfig::Direct => TcpStream::connect((host, port)).await.map_err(err),
        NetworkConfig::Socks5 {
            host: proxy,
            port: proxy_port,
        } => Ok(tokio_socks::tcp::Socks5Stream::connect(
            (proxy.as_str(), *proxy_port),
            (host, port),
        )
        .await
        .map_err(err)?
        .into_inner()),
        NetworkConfig::Http {
            host: proxy,
            port: proxy_port,
        } => {
            let mut stream = TcpStream::connect((proxy.as_str(), *proxy_port))
                .await
                .map_err(err)?;
            stream
                .write_all(
                    format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .map_err(err)?;
            let mut response = Vec::new();
            while !response.ends_with(b"\r\n\r\n") {
                if response.len() >= 8192 {
                    return Err("HTTP 代理响应头过大".into());
                }
                response.push(stream.read_u8().await.map_err(err)?);
            }
            let status = String::from_utf8_lossy(&response)
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_owned();
            if status != "200" {
                return Err(format!("HTTP CONNECT 失败：{status}"));
            }
            Ok(stream)
        }
    }
}

pub async fn connect_ws(
    config: &NetworkConfig,
    url: &str,
    cancel: &CancellationToken,
) -> AppResult<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    config.validate()?;
    let parsed = Url::parse(url).map_err(err)?;
    if !["ws", "wss"].contains(&parsed.scheme()) {
        return Err("不支持的 WS 协议".into());
    }
    let host = parsed.host_str().ok_or("WS 缺少主机")?;
    let port = parsed.port_or_known_default().ok_or("WS 缺少端口")?;
    tokio::select! { biased; _=cancel.cancelled()=>Err("CANCELLED".into()), result=tokio::time::timeout(Duration::from_secs(15),async {
        let stream=tcp(config,host,port).await?;
        let limits=tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(512*1024)).max_frame_size(Some(512*1024));
        let (ws,_)=tokio_tungstenite::client_async_tls_with_config(url,stream,Some(limits),None).await.map_err(err)?;
        Ok(ws)
    })=>result.map_err(err)? }
}

pub async fn first_ws(
    config: &NetworkConfig,
    url: &str,
    cancel: &CancellationToken,
) -> AppResult<String> {
    let mut ws = connect_ws(config, url, cancel).await?;
    let result = tokio::select! { biased; _=cancel.cancelled()=>Err("CANCELLED".into()), result=tokio::time::timeout(Duration::from_secs(15),async {
        while let Some(frame)=ws.next().await { match frame.map_err(err)? { tokio_tungstenite::tungstenite::Message::Text(text)=>return Ok(text.to_string()),tokio_tungstenite::tungstenite::Message::Close(_)=>return Err("WS 提前关闭".into()),_=>{} } }
        Err("WS 无数据".into())
    })=>result.map_err(err)? };
    // 取消时直接丢弃套接字，避免等待对端 Close 使退出再次阻塞。
    if !cancel.is_cancelled() {
        let _ = tokio::time::timeout(Duration::from_millis(500), ws.close(None)).await;
    }
    result
}

#[cfg(windows)]
pub fn system_proxy() -> AppResult<NetworkConfig> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .map_err(err)?;
    if key
        .get_value::<String, _>("AutoConfigURL")
        .is_ok_and(|s| !s.is_empty())
    {
        return Err("PAC 未验证，请选择手动 HTTP 或 SOCKS5".into());
    }
    if key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 0 {
        return Ok(NetworkConfig::Direct);
    }
    let server: String = key.get_value("ProxyServer").map_err(err)?;
    parse_system_proxy(&server)
}

fn parse_system_proxy(server: &str) -> AppResult<NetworkConfig> {
    let server = if server.contains('=') {
        server
            .split(';')
            .find_map(|s| s.strip_prefix("https="))
            .or_else(|| server.split(';').find_map(|s| s.strip_prefix("http=")))
            .ok_or("无法解析系统代理，请手动设置")?
    } else {
        server
    };
    let (host, port) = server.rsplit_once(':').ok_or("无法解析系统代理端口")?;
    let config = NetworkConfig::Http {
        host: host.into(),
        port: port.parse().map_err(err)?,
    };
    config.validate()?;
    Ok(config)
}

pub fn validate_probe_payloads(rest: &str, ws: &str) -> AppResult<serde_json::Value> {
    let rest: serde_json::Value = serde_json::from_str(rest).map_err(err)?;
    let ws: serde_json::Value = serde_json::from_str(ws).map_err(err)?;
    if rest["serverTime"].as_u64().is_none_or(|v| v == 0) {
        return Err("REST 不是有效的服务器时间响应".into());
    }
    let price = ws["p"]
        .as_str()
        .and_then(|p| p.parse::<f64>().ok())
        .filter(|p| p.is_finite() && *p > 0.);
    if ws["e"] != "markPriceUpdate"
        || ws["s"] != "BTCUSDT"
        || ws["E"].as_u64().is_none_or(|v| v == 0)
        || price.is_none()
        || ws.get("st").is_some_and(|st| st != 1)
    {
        return Err("WS 不是预期的 USD-M BTCUSDT 标记价格事件".into());
    }
    Ok(serde_json::json!({"serverTime":rest["serverTime"],"event":ws}))
}
#[cfg(not(windows))]
pub fn system_proxy() -> AppResult<NetworkConfig> {
    Ok(NetworkConfig::Direct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;
    use tokio::net::TcpListener;

    #[test]
    fn secure_endpoints_prefer_https_system_proxy() {
        let c = parse_system_proxy("http=127.0.0.1:8081;https=127.0.0.1:8082").unwrap();
        assert!(matches!(c, NetworkConfig::Http { port: 8082, .. }));
    }

    #[test]
    fn errors_and_wrong_market_are_not_probe_success() {
        let good = r#"{"e":"markPriceUpdate","s":"BTCUSDT","E":1,"p":"100.0","st":1}"#;
        assert!(validate_probe_payloads(r#"{"serverTime":1}"#, good).is_ok());
        assert!(validate_probe_payloads(r#"{"code":-1,"msg":"error"}"#, good).is_err());
        assert!(validate_probe_payloads(r#"{"serverTime":1}"#, r#"{"code":-1}"#).is_err());
        assert!(validate_probe_payloads(
            r#"{"serverTime":1}"#,
            &good.replace("\"st\":1", "\"st\":2")
        )
        .is_err());
    }

    async fn http_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0; 2048];
                    let _ = s.read(&mut buf).await;
                    let _ = s
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        port
    }
    async fn proxy(socks: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut source, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let host;
                    let target_port;
                    if socks {
                        assert_eq!(source.read_u8().await.unwrap(), 5);
                        let n = source.read_u8().await.unwrap();
                        let mut methods = vec![0; n as usize];
                        source.read_exact(&mut methods).await.unwrap();
                        source.write_all(&[5, 0]).await.unwrap();
                        let mut header = [0; 4];
                        source.read_exact(&mut header).await.unwrap();
                        assert_eq!(&header[..3], &[5, 1, 0]);
                        host = match header[3] {
                            1 => {
                                let mut ip = [0; 4];
                                source.read_exact(&mut ip).await.unwrap();
                                std::net::Ipv4Addr::from(ip).to_string()
                            }
                            3 => {
                                let n = source.read_u8().await.unwrap();
                                let mut h = vec![0; n as usize];
                                source.read_exact(&mut h).await.unwrap();
                                String::from_utf8(h).unwrap()
                            }
                            _ => panic!("地址类型"),
                        };
                        target_port = source.read_u16().await.unwrap();
                        source
                            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                            .await
                            .unwrap();
                    } else {
                        let mut request = Vec::new();
                        while !request.ends_with(b"\r\n\r\n") {
                            request.push(source.read_u8().await.unwrap());
                        }
                        let req = String::from_utf8(request).unwrap();
                        let line = req.lines().next().unwrap();
                        let address = line.split_whitespace().nth(1).unwrap();
                        if line.starts_with("CONNECT ") {
                            let (h, p) = address.rsplit_once(':').unwrap();
                            host = h.to_owned();
                            target_port = p.parse().unwrap();
                            source.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
                        } else {
                            let url = Url::parse(address).unwrap();
                            host = url.host_str().unwrap().to_owned();
                            target_port = url.port_or_known_default().unwrap();
                            let mut target = TcpStream::connect((host.as_str(), target_port))
                                .await
                                .unwrap();
                            target.write_all(req.as_bytes()).await.unwrap();
                            let _ = tokio::io::copy_bidirectional(&mut source, &mut target).await;
                            return;
                        }
                    }
                    let mut target = TcpStream::connect((host.as_str(), target_port))
                        .await
                        .unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut source, &mut target).await;
                });
            }
        });
        port
    }
    async fn ws_server() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (s, _) = l.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(
                        "fresh".into(),
                    ))
                    .await
                    .unwrap();
                    let _ = ws.next().await;
                });
            }
        });
        p
    }
    #[tokio::test]
    async fn http_ws_direct_and_both_proxies() {
        let h = http_server().await;
        let w = ws_server().await;
        for c in [
            NetworkConfig::Direct,
            NetworkConfig::Http {
                host: "127.0.0.1".into(),
                port: proxy(false).await,
            },
            NetworkConfig::Socks5 {
                host: "127.0.0.1".into(),
                port: proxy(true).await,
            },
        ] {
            let cancel = CancellationToken::new();
            assert_eq!(
                http_get(&c, &format!("http://127.0.0.1:{h}/"), &cancel)
                    .await
                    .unwrap(),
                "ok"
            );
            assert_eq!(
                first_ws(&c, &format!("ws://127.0.0.1:{w}/"), &cancel)
                    .await
                    .unwrap(),
                "fresh"
            );
        }
    }
    #[tokio::test]
    async fn cancellation_and_recovery() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        let c = CancellationToken::new();
        let cc = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cc.cancel();
        });
        let started = std::time::Instant::now();
        assert_eq!(
            connect_ws(&NetworkConfig::Direct, &format!("ws://127.0.0.1:{p}"), &c)
                .await
                .unwrap_err(),
            "CANCELLED"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        let c = CancellationToken::new();
        let cc = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cc.cancel();
        });
        assert_eq!(
            http_get(&NetworkConfig::Direct, &format!("http://127.0.0.1:{p}"), &c)
                .await
                .unwrap_err(),
            "CANCELLED"
        );
        assert_eq!(
            first_ws(
                &NetworkConfig::Direct,
                &format!("ws://127.0.0.1:{}/", ws_server().await),
                &CancellationToken::new()
            )
            .await
            .unwrap(),
            "fresh"
        );
    }

    #[tokio::test]
    async fn cancel_established_reader_then_recover() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            let _ = ws.next().await;
        });
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            child.cancel();
        });
        let started = std::time::Instant::now();
        assert_eq!(
            first_ws(
                &NetworkConfig::Direct,
                &format!("ws://127.0.0.1:{port}/"),
                &cancel
            )
            .await
            .unwrap_err(),
            "CANCELLED"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first_ws(
                &NetworkConfig::Direct,
                &format!("ws://127.0.0.1:{}/", ws_server().await),
                &CancellationToken::new()
            )
            .await
            .unwrap(),
            "fresh"
        );
    }

    #[tokio::test]
    async fn cancel_proxy_handshakes_without_waiting_for_timeout() {
        for socks in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = [0; 1024];
                while stream.read(&mut data).await.unwrap_or(0) > 0 {}
            });
            let config = if socks {
                NetworkConfig::Socks5 {
                    host: "127.0.0.1".into(),
                    port,
                }
            } else {
                NetworkConfig::Http {
                    host: "127.0.0.1".into(),
                    port,
                }
            };
            let cancel = CancellationToken::new();
            let child = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                child.cancel();
            });
            let started = std::time::Instant::now();
            assert_eq!(
                connect_ws(&config, "wss://example.com/ws", &cancel)
                    .await
                    .unwrap_err(),
                "CANCELLED"
            );
            assert!(started.elapsed() < Duration::from_secs(1));
            tokio::time::timeout(Duration::from_secs(1), server)
                .await
                .unwrap()
                .unwrap();
        }
    }
}
