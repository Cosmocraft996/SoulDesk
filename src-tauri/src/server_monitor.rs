//! 原服务器组件的固定监控协议，不提供通用HTTP、任意请求头或远程命令。
use crate::{
    business,
    network::NetworkConfig,
    store::{now_ms, AppResult, Store},
    widgets, Core,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Manager};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    url: String,
    token: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Stats,
    Info,
    Pull,
    Push,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    kind: Kind,
    connection: Connection,
    forward: Option<Connection>,
    servers: Option<Vec<Value>>,
}
struct Request {
    url: String,
    body: Option<Value>,
    token: String,
    kind: Kind,
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("监控固定请求（认证信息隐藏）")
    }
}
fn connection(value: &Connection) -> AppResult<String> {
    if value.url.len() > 2048
        || value.url.chars().any(char::is_whitespace)
        || value.token.is_empty()
        || value.token.len() > 4096
        || reqwest::header::HeaderValue::from_str(&value.token).is_err()
    {
        return Err("INVALID_ARGUMENT: 监控服务地址或令牌无效".into());
    }
    let mut url =
        url::Url::parse(&value.url).map_err(|_| "INVALID_ARGUMENT: 监控服务地址无效".to_owned())?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "INVALID_ARGUMENT: 服务地址仅支持HTTP/HTTPS，不接收内嵌认证、查询或片段".into(),
        );
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}
fn request(args: Value) -> AppResult<Request> {
    if serde_json::to_vec(&args)
        .map_err(|_| "INVALID_ARGUMENT: 监控请求无效")?
        .len()
        > 4 * 1024 * 1024
    {
        return Err("LIMIT: 监控请求过大".into());
    }
    let args: Args = serde_json::from_value(args)
        .map_err(|_| "INVALID_ARGUMENT: 监控参数或操作无效".to_owned())?;
    let mut url = connection(&args.connection)?;
    if matches!(args.kind, Kind::Pull | Kind::Push) && args.forward.is_some()
        || !matches!(args.kind, Kind::Push) && args.servers.is_some()
    {
        return Err("INVALID_ARGUMENT: 本操作不接受转发或服务器列表".into());
    }
    let body = match args.kind {
        Kind::Pull => {
            url.push_str("/servers");
            None
        }
        Kind::Push => {
            url.push_str("/servers");
            let servers = args.servers.ok_or("INVALID_ARGUMENT: 缺少服务器列表")?;
            if servers.len() > 128
                || servers
                    .iter()
                    .any(|s| !s.is_object() || !s["name"].is_string() || !s["ip"].is_string())
            {
                return Err("INVALID_ARGUMENT: 服务器列表格式无效".into());
            }
            Some(json!(servers))
        }
        Kind::Stats | Kind::Info => {
            let mut body = json!({"url":url,"token":args.connection.token,"mem":"*"});
            if matches!(args.kind, Kind::Stats) {
                body["currentLoad"] =
                    json!("avgLoad,currentLoad,currentLoadUser,currentLoadIdle,currentLoadSystem");
                body["fsSize"] = json!("*");
            } else {
                body["cpu"] = json!("*");
                body["osInfo"] = json!("*");
            }
            if let Some(forward) = args.forward {
                body["forwardUrl"] = json!(connection(&forward)?);
                body["forwardToken"] = json!(forward.token);
            }
            Some(body)
        }
    };
    Ok(Request {
        url,
        body,
        token: args.connection.token,
        kind: args.kind,
    })
}
async fn execute(config: &NetworkConfig, request: Request) -> AppResult<Value> {
    config.validate()?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8));
    let proxy = match config {
        NetworkConfig::Direct => None,
        NetworkConfig::Http { host, port } => Some(format!("http://{host}:{port}")),
        NetworkConfig::Socks5 { host, port } => Some(format!("socks5h://{host}:{port}")),
    };
    if let Some(proxy) = proxy {
        builder = builder
            .proxy(reqwest::Proxy::all(proxy).map_err(|_| "NETWORK: 监控代理配置无效".to_owned())?);
    }
    let client = builder
        .build()
        .map_err(|_| "NETWORK: 无法创建监控客户端".to_owned())?;
    let call = if let Some(body) = request.body {
        client.post(&request.url).json(&body)
    } else {
        client.get(&request.url)
    };
    let mut response = call
        .header("token", request.token)
        .send()
        .await
        .map_err(|_| "NETWORK: 监控请求失败或超时".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "DATA_UNAVAILABLE: 监控服务HTTP {}",
            response.status()
        ));
    }
    const LIMIT: usize = 4 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|size| size > LIMIT as u64)
    {
        return Err("LIMIT: 监控响应过大".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "NETWORK: 监控响应未完整读取".to_owned())?
    {
        if chunk.len() > LIMIT.saturating_sub(bytes.len()) {
            return Err("LIMIT: 监控响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if matches!(request.kind, Kind::Push) {
        return Ok(json!({"accepted":true,"receivedAt":now_ms()}));
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "DATA_UNAVAILABLE: 监控响应不是有效JSON".to_owned())?;
    if matches!(request.kind, Kind::Pull) {
        if !data
            .as_array()
            .is_some_and(|list| list.len() <= 128 && list.iter().all(Value::is_object))
        {
            return Err("DATA_UNAVAILABLE: 同步列表格式无效或超过128项".into());
        }
    } else if !data.is_object() {
        return Err("DATA_UNAVAILABLE: 监控响应不是指标对象".into());
    }
    Ok(json!({"data":data,"receivedAt":now_ms()}))
}
fn authorize(
    store: &Store,
    sessions: &widgets::Sessions,
    window: &str,
    token: &str,
    args: Value,
) -> AppResult<(String, Request)> {
    let id = widgets::require_capability(store, sessions, window, token, "servers.monitor")?;
    Ok((id, request(args)?))
}
pub async fn gateway(app: &AppHandle, window: &str, token: &str, args: Value) -> AppResult<Value> {
    let core = app.state::<Core>();
    let (id, request) = authorize(&core.store, &core.sessions, window, token, args)?;
    let config: Value = serde_json::from_str(
        &core
            .store
            .preference("network")
            .unwrap_or("{\"mode\":\"system\"}".into()),
    )
    .map_err(|_| "NETWORK: 网络配置无效".to_owned())?;
    let config = business::resolve_network(&config)?;
    let result = execute(&config, request).await?;
    if widgets::require_capability(
        &core.store,
        &core.sessions,
        window,
        token,
        "servers.monitor",
    )? != id
    {
        return Err("SESSION_EXPIRED: 监控实例已变化".into());
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    fn connection() -> Value {
        json!({"url":"http://127.0.0.1:5549/api/monitor/","token":"private-test-token"})
    }
    #[test]
    fn formal_package_requires_capability_before_network_and_keeps_config_per_instance() {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-server-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            crate::presets::find("local.souldesk.widgetjs.server")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        let defaults = widgets::initial_settings(&staged.manifest);
        widgets::activate(&store, &staged).unwrap();
        let first = widgets::create_instance(&store, "local.souldesk.widgetjs.server").unwrap();
        let second = widgets::create_instance(&store, "local.souldesk.widgetjs.server").unwrap();
        let sessions = widgets::Sessions::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        let args = json!({"kind":"stats","connection":connection()});
        assert!(authorize(&store, &sessions, "wrong-window", token, args.clone()).is_err());
        sessions.0.lock().unwrap().insert(
            "preview".into(),
            widgets::Session {
                instance: first.clone(),
                window: "trial-server".into(),
                shared_cursor: None,
                preview: Some(std::sync::Arc::new(std::sync::Mutex::new(
                    widgets::Preview {
                        package: staged,
                        settings: defaults,
                        storage: json!({}),
                        ready_since: None,
                        failure: None,
                    },
                ))),
            },
        );
        assert!(authorize(&store, &sessions, "trial-server", "preview", args.clone()).is_err());
        assert_eq!(
            authorize(&store, &sessions, &label, token, args).unwrap().0,
            first
        );
        let saved = json!({"version":1,"servers":[{"id":"one","name":"我的服务器","ip":"47.1.2.3","api":{"url":"http://127.0.0.1:5549/api/monitor","token":"fixture-token","interval":10}}],"master":{"url":"","token":""}});
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "storage.set",
            saved.clone(),
        )
        .unwrap();
        let missing = widgets::stage_bytes(
            &store,
            crate::presets::find("local.example.clock")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        widgets::activate(&store, &missing).unwrap();
        let id = widgets::create_instance(&store, "local.example.clock").unwrap();
        let other_label = format!("float-{id}");
        let other =
            widgets::open_display_session(&store, &sessions, &other_label, &id, None).unwrap();
        assert!(authorize(
            &store,
            &sessions,
            &other_label,
            other["token"].as_str().unwrap(),
            json!({"kind":"stats","connection":connection()})
        )
        .is_err());
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        let sessions = widgets::Sessions::default();
        for (id, expected) in [(&first, saved), (&second, json!({}))] {
            let label = format!("float-{id}");
            let session =
                widgets::open_display_session(&reopened, &sessions, &label, id, None).unwrap();
            assert_eq!(
                widgets::gateway(
                    &reopened,
                    &sessions,
                    &label,
                    session["token"].as_str().unwrap(),
                    "storage.get",
                    Value::Null
                )
                .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn original_operations_choose_fixed_fields_headers_and_servers_path() {
        let stats = request(json!({"kind":"stats","connection":connection()})).unwrap();
        assert_eq!(stats.url, "http://127.0.0.1:5549/api/monitor");
        assert_eq!(stats.token, "private-test-token");
        let body = stats.body.unwrap();
        assert_eq!(body["mem"], "*");
        assert_eq!(body["fsSize"], "*");
        assert_eq!(
            body["currentLoad"],
            "avgLoad,currentLoad,currentLoadUser,currentLoadIdle,currentLoadSystem"
        );
        assert!(body.get("shell").is_none());
        let info=request(json!({"kind":"info","connection":connection(),"forward":{"url":"https://my-server.example/api/monitor","token":"forward-test"}})).unwrap();
        let body = info.body.unwrap();
        assert_eq!(body["cpu"], "*");
        assert_eq!(body["osInfo"], "*");
        assert_eq!(body["forwardToken"], "forward-test");
        assert_eq!(body["forwardUrl"], "https://my-server.example/api/monitor");
        let pull = request(json!({"kind":"pull","connection":connection()})).unwrap();
        assert_eq!(pull.url, "http://127.0.0.1:5549/api/monitor/servers");
        assert!(pull.body.is_none());
    }
    #[test]
    fn unknown_fields_invalid_urls_and_header_injection_are_rejected_without_secret_echo() {
        for url in [
            "file:///C:/data",
            "http://name:secret@example.com/api/monitor",
            "http://example.com/api?token=secret",
            "http://example.com/api#secret",
        ] {
            let error =
                request(json!({"kind":"stats","connection":{"url":url,"token":"do-not-echo"}}))
                    .unwrap_err();
            assert!(!error.contains("secret"));
            assert!(!error.contains("do-not-echo"));
        }
        for args in [
            json!({"kind":"shell","connection":connection()}),
            json!({"kind":"stats","connection":connection(),"method":"DELETE"}),
            json!({"kind":"stats","connection":{"url":"http://127.0.0.1/api","token":"secret\r\nHost: attacker"}}),
            json!({"kind":"pull","connection":connection(),"forward":connection()}),
            json!({"kind":"push","connection":connection(),"servers":["not-a-server"]}),
        ] {
            assert!(request(args).is_err());
        }
    }
    async fn serve(response: impl AsRef<[u8]>) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let response = response.as_ref().to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                assert!(bytes.len() < 16384);
                bytes.push(socket.read_u8().await.unwrap());
            }
            let headers = String::from_utf8(bytes.clone()).unwrap();
            let size = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|n| n.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0; size];
            socket.read_exact(&mut body).await.unwrap();
            bytes.extend(body);
            let _ = socket.write_all(&response).await;
            bytes
        });
        (format!("http://{address}/api/monitor"), handle)
    }
    #[tokio::test]
    async fn native_client_sends_original_token_and_post_body_to_local_fixture() {
        let(url,server)=serve(b"HTTP/1.1 200 OK\r\nContent-Length: 42\r\nConnection: close\r\n\r\n{\"currentLoad\":{\"currentLoad\":0},\"mem\":{}}").await;
        let value = execute(
            &NetworkConfig::Direct,
            request(json!({"kind":"stats","connection":{"url":url,"token":"fixture-token"}}))
                .unwrap(),
        )
        .await
        .unwrap();
        assert!(value["receivedAt"].as_u64().is_some());
        assert_eq!(value["data"]["currentLoad"]["currentLoad"], 0);
        let bytes = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(bytes.starts_with("POST /api/monitor HTTP/1.1"));
        assert!(bytes
            .to_ascii_lowercase()
            .contains("token: fixture-token\r\n"));
        let body: Value = serde_json::from_str(bytes.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["fsSize"], "*");
    }
    #[tokio::test]
    async fn redirects_large_streams_invalid_json_and_http_errors_do_not_leak_tokens() {
        for response in [b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret",b"HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n",b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret"]{
            let(url,server)=serve(response).await;let error=execute(&NetworkConfig::Direct,request(json!({"kind":"stats","connection":{"url":url,"token":"fixture-secret"}})).unwrap()).await.unwrap_err();assert!(!error.contains("fixture-secret"));assert!(!error.contains("secret"));assert!(!error.contains("127.0.0.1"));server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn sync_gets_and_posts_only_servers_and_reports_acceptance() {
        let (url, server) =
            serve(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await;
        let reply = execute(
            &NetworkConfig::Direct,
            request(json!({"kind":"pull","connection":{"url":url,"token":"fixture"}})).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(reply["data"], json!([]));
        assert!(String::from_utf8(server.await.unwrap())
            .unwrap()
            .starts_with("GET /api/monitor/servers HTTP/1.1"));
        let (url, server) =
            serve(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        let list = json!([{"id":"one","name":"服务器","ip":"47.1.2.3","api":{"url":"http://server.example/api/monitor","token":"remote-fixture"}}]);
        let reply = execute(
            &NetworkConfig::Direct,
            request(
                json!({"kind":"push","connection":{"url":url,"token":"fixture"},"servers":list}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(reply["accepted"], true);
        let bytes = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(bytes.starts_with("POST /api/monitor/servers HTTP/1.1"));
        assert_eq!(
            serde_json::from_str::<Value>(bytes.split_once("\r\n\r\n").unwrap().1).unwrap(),
            list
        );
    }
    #[tokio::test]
    async fn streaming_limit_applies_without_content_length_and_pull_rejects_wrong_shape() {
        let body = "x".repeat(4 * 1024 * 1024 + 1);
        let response=format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len());
        let (url, server) = serve(response).await;
        let error = execute(
            &NetworkConfig::Direct,
            request(json!({"kind":"stats","connection":{"url":url,"token":"fixture"}})).unwrap(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("LIMIT"));
        server.await.unwrap();
        let (url, server) =
            serve(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await;
        assert!(execute(
            &NetworkConfig::Direct,
            request(json!({"kind":"pull","connection":{"url":url,"token":"fixture"}})).unwrap()
        )
        .await
        .unwrap_err()
        .contains("列表"));
        server.await.unwrap();
    }
}
