//! 固定AI生成工具的请求与流式协议；不执行HTTP或读取凭据。
use crate::store::AppResult;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Task {
    Mindmap,
    Mermaid,
    Translate,
}
pub(crate) fn endpoint(base: &str) -> AppResult<url::Url> {
    if base.len() > 2048 || base.chars().any(char::is_control) {
        return Err("INVALID_ARGUMENT: AI接口地址无效".into());
    }
    let mut url = url::Url::parse(base.trim()).map_err(|_| "INVALID_ARGUMENT: AI接口地址无效")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url
            .path()
            .trim_end_matches('/')
            .ends_with("/chat/completions")
    {
        return Err("INVALID_ARGUMENT: 请填HTTPS API Base URL（本机回环允许HTTP），不含密钥、查询或完整chat/completions路径".into());
    }
    url.set_path(&format!(
        "{}/chat/completions",
        url.path().trim_end_matches('/')
    ));
    Ok(url)
}
pub(crate) fn request_body(task: Task, model: &str, input: Value) -> AppResult<Value> {
    if model.trim().is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
        return Err("INVALID_ARGUMENT: 请填写自己的模型名称".into());
    }
    fn content(text: &str) -> AppResult<()> {
        if text.trim().is_empty() || text.chars().count() > 10000 {
            return Err("INVALID_ARGUMENT: 内容须为1到10000字".into());
        }
        Ok(())
    }
    let prompts: Value = serde_json::from_str(include_str!(
        "../../components/widgetjs/ai-source/prompts.json"
    ))
    .map_err(|_| "AI原提示词资源无效")?;
    let (prompt, text) = match task {
        Task::Mindmap => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                content: String,
                #[serde(default)]
                extra: String,
            }
            let value: Input =
                serde_json::from_value(input).map_err(|_| "INVALID_ARGUMENT: 思维导图参数无效")?;
            content(&value.content)?;
            if value.extra.chars().count() > 500 {
                return Err("INVALID_ARGUMENT: 其他说明最多500字".into());
            }
            (
                prompts["mindMap"]
                    .as_str()
                    .unwrap_or_default()
                    .replace("@{content}", &value.content)
                    .replace("@{extra}", &value.extra),
                value.content,
            )
        }
        Task::Translate => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Input {
                content: String,
                first_lang: String,
                second_lang: String,
            }
            let value: Input =
                serde_json::from_value(input).map_err(|_| "INVALID_ARGUMENT: 翻译参数无效")?;
            content(&value.content)?;
            let languages = [
                "en", "zh", "es", "hi", "ar", "pt", "bn", "ru", "ja", "pa", "de", "ko", "fr", "te",
                "mr", "ta", "ur", "it", "tr", "fa",
            ];
            if !languages.contains(&value.first_lang.as_str())
                || !languages.contains(&value.second_lang.as_str())
            {
                return Err("INVALID_ARGUMENT: 语言不在原20种列表内".into());
            }
            (
                prompts["translate"]
                    .as_str()
                    .unwrap_or_default()
                    .replace("{{firstLang}}", &value.first_lang)
                    .replace("{{secondLang}}", &value.second_lang),
                value.content,
            )
        }
        Task::Mermaid => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Input {
                content: String,
                chart_type: String,
            }
            let value: Input =
                serde_json::from_value(input).map_err(|_| "INVALID_ARGUMENT: 图表参数无效")?;
            content(&value.content)?;
            let charts = [
                "Flowchart",
                "Sequence Diagram",
                "Class Diagram",
                "State Diagram",
                "ER Diagram",
                "Gantt Chart",
                "Pie Chart",
                "Quadrant Chart",
                "Git Graph",
                "Mindmap",
                "Timeline",
            ];
            if !charts.contains(&value.chart_type.as_str()) {
                return Err("INVALID_ARGUMENT: 图表类型不在原11种列表内".into());
            }
            (
                prompts["mermaidDiagram"]
                    .as_str()
                    .unwrap_or_default()
                    .replace("{{chartType}}", &value.chart_type),
                value.content,
            )
        }
    };
    let messages = if task == Task::Mindmap {
        json!([{"role":"user","content":prompt}])
    } else {
        json!([{"role":"system","content":prompt},{"role":"user","content":text}])
    };
    Ok(json!({"model":model.trim(),"stream":true,"messages":messages}))
}
#[derive(Default)]
pub(crate) struct Decoder {
    pending: Vec<u8>,
    data: Vec<String>,
    data_bytes: usize,
    total: usize,
    text: String,
    done: bool,
    failed: bool,
}
impl Decoder {
    pub fn feed(&mut self, bytes: &[u8]) -> AppResult<()> {
        if self.failed {
            return Err("DATA_UNAVAILABLE: AI数据流已中断".into());
        }
        let result = self.feed_inner(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn feed_inner(&mut self, bytes: &[u8]) -> AppResult<()> {
        self.total = self.total.saturating_add(bytes.len());
        if self.total > 2 * 1024 * 1024 {
            return Err("LIMIT: AI响应超过2MiB".into());
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            if end > 256 * 1024 {
                return Err("LIMIT: AI单帧过大".into());
            }
            let line = self.pending.drain(..=end).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line[..line.len() - 1])
                .map_err(|_| "DATA_UNAVAILABLE: AI流不是有效UTF-8")?
                .trim_end_matches('\r');
            if line.is_empty() {
                self.event()?;
            } else if let Some(data) = line.strip_prefix("data:") {
                let data = data.strip_prefix(' ').unwrap_or(data);
                self.data_bytes = self.data_bytes.saturating_add(data.len() + 1);
                if self.data_bytes > 256 * 1024 {
                    return Err("LIMIT: AI单帧过大".into());
                }
                self.data.push(data.into());
            }
        }
        if self.pending.len() > 256 * 1024 {
            return Err("LIMIT: AI单帧过大".into());
        }
        Ok(())
    }
    fn event(&mut self) -> AppResult<()> {
        if self.data.is_empty() {
            return Ok(());
        }
        let data = self.data.join("\n");
        self.data.clear();
        self.data_bytes = 0;
        if self.done {
            return Err("DATA_UNAVAILABLE: AI流结束后收到额外事件".into());
        }
        if data.trim() == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let event: Value =
            serde_json::from_str(&data).map_err(|_| "DATA_UNAVAILABLE: AI流事件格式无效")?;
        if event.get("error").is_some() {
            return Err("DATA_UNAVAILABLE: AI服务返回错误，请检查模型和服务配置".into());
        }
        let choices = event["choices"]
            .as_array()
            .ok_or("DATA_UNAVAILABLE: AI流缺少choices")?;
        for choice in choices {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                continue;
            }
            if choice["delta"].get("tool_calls").is_some()
                || choice["delta"].get("function_call").is_some()
            {
                return Err("UNSUPPORTED: 此组件不执行AI工具调用".into());
            }
            if let Some(content) = choice["delta"]
                .get("content")
                .filter(|value| !value.is_null())
            {
                let content = content
                    .as_str()
                    .ok_or("DATA_UNAVAILABLE: AI流内容不是文本")?;
                if content.len() > (512 * 1024usize).saturating_sub(self.text.len()) {
                    return Err("LIMIT: AI文本超过512KiB".into());
                }
                self.text.push_str(content);
            }
            // 先保留本帧已收到的真实文本，再标记截断；部分结果不能冒充完成。
            if let Some(reason) = choice["finish_reason"].as_str() {
                if reason != "stop" {
                    return Err(if reason == "length" {
                        "DATA_UNAVAILABLE: AI输出达到服务长度上限，结果不完整"
                    } else {
                        "DATA_UNAVAILABLE: AI输出被服务中断或要求不支持的动作"
                    }
                    .into());
                }
            }
        }
        Ok(())
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn finish(&self) -> AppResult<String> {
        if self.failed || !self.done {
            return Err("DATA_UNAVAILABLE: AI流中断，已保留部分结果".into());
        }
        if self.text.trim().is_empty() {
            return Err("DATA_UNAVAILABLE: AI未返回文本内容".into());
        }
        Ok(self.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn frame(content: &str) -> Vec<u8> {
        format!(
            "data: {}\n\n",
            json!({"choices":[{"index":0,"delta":{"content":content},"finish_reason":null}]})
        )
        .into_bytes()
    }
    #[test]
    fn ai_protocol_endpoint_is_fixed_post_path_and_rejects_url_credentials_or_plain_remote_http() {
        assert_eq!(
            endpoint("https://api.example.com/v1/").unwrap().as_str(),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint("http://127.0.0.1:1234/v1").unwrap().path(),
            "/v1/chat/completions"
        );
        for base in [
            "http://api.example.com/v1",
            "https://user:password@api.example.com/v1",
            "https://api.example.com/v1?key=hidden",
            "https://api.example.com/v1#key",
            "file:///local",
            "https://api.example.com/v1/chat/completions",
        ] {
            assert!(endpoint(base).is_err(), "不应接受{base}");
        }
    }
    #[test]
    fn ai_protocol_preserves_original_prompts_languages_and_chart_types_with_strict_arguments() {
        let body = request_body(
            Task::Translate,
            "fixture-model",
            json!({"content":"你好","firstLang":"zh","secondLang":"en"}),
        )
        .unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["model"], "fixture-model");
        assert_eq!(body["messages"][1]["content"], "你好");
        let prompt = body["messages"][0]["content"].as_str().unwrap();
        assert!(prompt.contains("母语是zh") && prompt.contains("第二语言是en"));
        let map = request_body(
            Task::Mindmap,
            "fixture",
            json!({"content":"测试","extra":"层次清楚"}),
        )
        .unwrap();
        assert!(map["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("每个节点的子节点数量不超过30个"));
        for chart in [
            "Flowchart",
            "Sequence Diagram",
            "Class Diagram",
            "State Diagram",
            "ER Diagram",
            "Gantt Chart",
            "Pie Chart",
            "Quadrant Chart",
            "Git Graph",
            "Mindmap",
            "Timeline",
        ] {
            assert!(request_body(
                Task::Mermaid,
                "fixture",
                json!({"content":"测试","chartType":chart})
            )
            .is_ok());
        }
        for input in [
            json!({"content":"","firstLang":"zh","secondLang":"en"}),
            json!({"content":"测试","firstLang":"unsafe","secondLang":"en"}),
            json!({"content":"测试","firstLang":"zh","secondLang":"en","url":"https://elsewhere"}),
            json!({"content":"测".repeat(10001),"firstLang":"zh","secondLang":"en"}),
        ] {
            assert!(request_body(Task::Translate, "fixture", input).is_err());
        }
        assert!(request_body(
            Task::Mermaid,
            "fixture",
            json!({"content":"测试","chartType":"arbitrary"})
        )
        .is_err());
    }
    #[test]
    fn ai_protocol_stream_accepts_utf8_split_at_every_byte_and_requires_done() {
        let mut decoder = Decoder::default();
        let mut bytes = frame("你好👋");
        bytes.extend(frame("，世界"));
        bytes.extend_from_slice(b"data: [DONE]\r\n\r\n");
        for byte in bytes {
            decoder.feed(&[byte]).unwrap();
        }
        assert_eq!(decoder.text(), "你好👋，世界");
        assert_eq!(decoder.finish().unwrap(), "你好👋，世界");
        let mut incomplete = Decoder::default();
        incomplete.feed(&frame("部分结果")).unwrap();
        assert_eq!(incomplete.text(), "部分结果");
        assert!(incomplete.finish().unwrap_err().contains("中断"));
    }
    #[test]
    fn ai_protocol_stream_supports_multiline_events_comments_and_empty_usage_chunks() {
        let mut decoder = Decoder::default();
        decoder.feed(b": keepalive\r\nevent: message\r\ndata: {\"choices\":\r\ndata: [{\"index\":0,\"delta\":{\"content\":\"ok\"}}]}\r\n\r\ndata: {\"choices\":[],\"usage\":{}}\n\ndata: [DONE]\n\n").unwrap();
        assert_eq!(decoder.finish().unwrap(), "ok");
    }
    #[test]
    fn ai_protocol_stream_refuses_provider_errors_invalid_utf8_tool_calls_and_truncation() {
        for bytes in [
            b"data: {\"error\":{\"message\":\"do not echo hidden key\"}}\n\n".as_slice(),
            b"data: {invalid}\n\n",
            b"data: \xff\n\n",
            b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
        ] {
            let mut decoder = Decoder::default();
            let error = decoder.feed(bytes).unwrap_err();
            assert!(!error.contains("hidden key"));
        }
        assert!(Decoder::default().finish().is_err());
    }
    #[test]
    fn ai_protocol_stream_bounds_single_event_total_input_and_output() {
        let mut decoder = Decoder::default();
        assert!(decoder.feed(&vec![b'x'; 300000]).is_err());
        let mut decoder = Decoder::default();
        let frame = frame(&"x".repeat(60000));
        let mut limited = false;
        for _ in 0..50 {
            if decoder.feed(&frame).is_err() {
                limited = true;
                break;
            }
        }
        assert!(limited, "不能无限积累模型输出");
        let mut decoder = Decoder::default();
        let keepalive = vec![b':'; 60000];
        let mut limited = false;
        for _ in 0..50 {
            let mut event = keepalive.clone();
            event.extend_from_slice(b"\n\n");
            if decoder.feed(&event).is_err() {
                limited = true;
                break;
            }
        }
        assert!(limited, "即使没有文本输出，也必须限制累计流量");
    }
    #[test]
    fn ai_protocol_truncated_final_chunk_still_retains_its_actual_text() {
        let mut decoder = Decoder::default();
        decoder.feed(&frame("已有")).unwrap();
        assert!(decoder.feed("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"最后\"},\"finish_reason\":\"length\"}]}\n\n".as_bytes()).is_err());
        assert_eq!(
            decoder.text(),
            "已有最后",
            "截断不能丢失同一帧已收到的真实文本"
        );
        assert!(decoder.finish().is_err());
    }
}
