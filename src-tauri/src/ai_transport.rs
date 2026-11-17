//! 固定AI流式POST；地址/模型由专用配置提供，不向组件开放通用HTTP。
use crate::{ai_protocol::Decoder, network::NetworkConfig, store::AppResult};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
pub(crate) async fn generate(
    network: &NetworkConfig,
    url: url::Url,
    key: &str,
    body: Value,
    cancel: &CancellationToken,
    mut update: impl FnMut(&str) -> AppResult<()>,
) -> AppResult<String> {
    network.validate()?;
    let bytes = serde_json::to_vec(&body).map_err(|_| "INVALID_ARGUMENT: AI请求格式无效")?;
    if bytes.len() > 64 * 1024 || body["stream"] != true {
        return Err("INVALID_ARGUMENT: AI请求须为64KiB内流式请求".into());
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120));
    if let Some(proxy) = network.proxy_url() {
        builder =
            builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "NETWORK_ERROR: AI代理配置无效")?);
    }
    let client = builder
        .build()
        .map_err(|_| "NETWORK_ERROR: AI网络初始化失败")?;
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .body(bytes);
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let operation = async {
        let mut response = request
            .send()
            .await
            .map_err(|_| "NETWORK_ERROR: AI连接失败，请检查服务和网络；未自动重试")?;
        if !response.status().is_success() {
            return Err(format!(
                "DATA_UNAVAILABLE: AI服务HTTP {}，未自动重试",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > 2 * 1024 * 1024)
        {
            return Err("LIMIT: AI响应超过2MiB".into());
        }
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("text/event-stream"))
            })
        {
            return Err("UNSUPPORTED: AI服务没有返回流式文本接口".into());
        }
        let mut decoder = Decoder::default();
        let mut sent = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "NETWORK_ERROR: AI流中断，已保留部分结果；未自动重试")?
        {
            let parsed = decoder.feed(&chunk);
            if decoder.text().len() != sent {
                update(decoder.text())?;
                sent = decoder.text().len();
            }
            parsed?;
            if let Ok(text) = decoder.finish() {
                return Ok(text);
            }
        }
        decoder.finish()
    };
    tokio::select! {biased;_=cancel.cancelled()=>Err("CANCELLED".into()),result=operation=>result}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_protocol::{self, Task};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn fixture(response: Vec<u8>) -> (url::Url, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut header_end = None;
            loop {
                let mut buffer = [0; 2048];
                let n = stream.read(&mut buffer).await.unwrap();
                assert!(n > 0 && request.len() < 128 * 1024);
                request.extend_from_slice(&buffer[..n]);
                if header_end.is_none() {
                    header_end = request
                        .windows(4)
                        .position(|part| part == b"\r\n\r\n")
                        .map(|end| end + 4);
                }
                if let Some(end) = header_end {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    if request.len() >= end + length {
                        break;
                    }
                }
            }
            for part in response.chunks(11) {
                stream.write_all(part).await.unwrap();
                tokio::task::yield_now().await;
            }
            String::from_utf8(request).unwrap()
        });
        (
            ai_protocol::endpoint(&format!("http://{address}/v1")).unwrap(),
            server,
        )
    }
    fn response(status: &str, body: &[u8], headers: &str) -> Vec<u8> {
        let mut response=format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n{headers}\r\n",body.len()).into_bytes();
        response.extend_from_slice(body);
        response
    }
    fn body() -> Value {
        ai_protocol::request_body(
            Task::Translate,
            "fixture-model",
            json!({"content":"你好","firstLang":"zh","secondLang":"en"}),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn ai_transport_sends_only_fixed_post_and_bearer_header_then_delivers_real_stream_text() {
        let content="data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"👋\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let (url, server) = fixture(response("200 OK", content.as_bytes(), "")).await;
        let mut parts = Vec::new();
        let text = generate(
            &NetworkConfig::Direct,
            url,
            "fixture-key",
            body(),
            &CancellationToken::new(),
            |text| {
                parts.push(text.to_owned());
                Ok(())
            },
        )
        .await
        .unwrap();
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer fixture-key\r\n"));
        assert!(!request.lines().next().unwrap().contains("fixture-key"));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["model"], "fixture-model");
        assert_eq!(text, "你好👋");
        assert_eq!(parts.last().unwrap(), "你好👋");
    }
    #[tokio::test]
    async fn ai_transport_rejects_redirect_and_server_body_secrets_without_following_or_echo() {
        let (url, server) = fixture(response(
            "302 Found",
            b"private provider text",
            "Location: http://127.0.0.1:1/elsewhere\r\n",
        ))
        .await;
        let error = generate(
            &NetworkConfig::Direct,
            url,
            "fixture-key",
            body(),
            &CancellationToken::new(),
            |_| Ok(()),
        )
        .await
        .unwrap_err();
        assert!(error.contains("302"));
        assert!(!error.contains("private provider text") && !error.contains("fixture-key"));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn ai_transport_incomplete_stream_retains_partial_output_and_cannot_return_success() {
        let (url, server) = fixture(response(
            "200 OK",
            b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "",
        ))
        .await;
        let mut partial = String::new();
        let error = generate(
            &NetworkConfig::Direct,
            url,
            "fixture-key",
            body(),
            &CancellationToken::new(),
            |text| {
                partial = text.into();
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("中断"));
        assert_eq!(partial, "partial");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn ai_transport_cancellation_and_expired_consumer_stop_before_unwanted_updates() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut calls = 0;
        let result = generate(
            &NetworkConfig::Direct,
            ai_protocol::endpoint("http://127.0.0.1:1/v1").unwrap(),
            "fixture-key",
            body(),
            &cancel,
            |_| {
                calls += 1;
                Ok(())
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), "CANCELLED");
        assert_eq!(calls, 0);
        let (url, server) = fixture(response(
            "200 OK",
            b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\ndata: [DONE]\n\n",
            "",
        ))
        .await;
        let error = generate(
            &NetworkConfig::Direct,
            url,
            "fixture-key",
            body(),
            &CancellationToken::new(),
            |_| Err("SESSION_EXPIRED: consumer gone".into()),
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("SESSION_EXPIRED:"));
        let _ = server.await;
    }
}
