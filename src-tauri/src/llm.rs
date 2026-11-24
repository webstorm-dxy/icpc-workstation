use crate::error::{AppError, AppResult};
use crate::models::{ModelDto, ProviderTestResult, ProviderUsageDto, UsageMeterDto};
use crate::util::strip_code_fence;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const APP_UA: &str = concat!("ICPCWorkstation/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: String,
    pub base_url: String,
    pub api_key: String,
    pub session_id: String,
}

impl ProviderConfig {
    pub fn base(&self) -> String {
        self.base_url.trim_end_matches('/').to_string()
    }

    pub fn is_anthropic(&self) -> bool {
        self.kind == "anthropic"
    }

    pub fn chat_url(&self) -> String {
        match self.kind.as_str() {
            "opencodego" => format!("{}/openai/v1/chat/completions", self.base()),
            "anthropic" => format!("{}/messages", self.base()),
            _ => format!("{}/chat/completions", self.base()),
        }
    }

    pub fn models_url(&self) -> String {
        match self.kind.as_str() {
            "opencodego" => format!("{}/v1/models", self.base()),
            _ => format!("{}/models", self.base()),
        }
    }

    pub fn usage_url(&self) -> String {
        format!("{}/v1/usage", self.base())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

impl LlmMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        }
    }
    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }
    pub fn tool(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i64>,
    pub json_mode: bool,
    pub session_id: String,
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Debug, Default)]
pub struct ChatOutcome {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Value>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum LlmEvent {
    Text(String),
    Reasoning(String),
    ToolCall(ToolCall),
    Usage(Value),
}

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(APP_UA)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    fn request(
        &self,
        cfg: &ProviderConfig,
        session_id: &str,
        url: &str,
    ) -> reqwest::RequestBuilder {
        let builder = self.http.post(url);
        let builder = match cfg.kind.as_str() {
            "anthropic" => builder
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01"),
            _ => builder.bearer_auth(&cfg.api_key),
        };
        let session = if session_id.is_empty() {
            cfg.session_id.as_str()
        } else {
            session_id
        };
        let builder = if cfg.kind == "opencodego" {
            builder.header("x-opencode-session", session)
        } else {
            builder
        };
        builder.header("User-Agent", APP_UA)
    }

    fn get(&self, cfg: &ProviderConfig, url: &str) -> reqwest::RequestBuilder {
        let builder = self.http.get(url);
        let builder = match cfg.kind.as_str() {
            "anthropic" => builder
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01"),
            _ => builder.bearer_auth(&cfg.api_key),
        };
        let builder = if cfg.kind == "opencodego" {
            builder.header("x-opencode-session", &cfg.session_id)
        } else {
            builder
        };
        builder.header("User-Agent", APP_UA)
    }

    pub async fn list_models(&self, cfg: &ProviderConfig) -> AppResult<Vec<ModelDto>> {
        let url = cfg.models_url();
        let resp = self
            .get(cfg, &url)
            .send()
            .await
            .map_err(AppError::network)?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(map_http_error(cfg, status.as_u16(), &text));
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| AppError {
            code: "E_PROVIDER_RESPONSE".into(),
            message: "模型列表返回格式无法解析".into(),
            detail: Some(e.to_string()),
            retryable: false,
        })?;
        let mut out = Vec::new();
        for key in ["data", "models"] {
            if let Some(items) = value.get(key).and_then(|v| v.as_array()) {
                for item in items {
                    let id = item
                        .get("id")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if id.is_empty() {
                        continue;
                    }
                    let name = item
                        .get("display_name")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    let context = item
                        .get("context_length")
                        .or_else(|| item.get("context_window"))
                        .and_then(|v| v.as_i64());
                    out.push(ModelDto { id, name, context });
                }
                break;
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out.dedup_by(|a, b| a.id == b.id);
        Ok(out)
    }

    pub async fn usage(&self, cfg: &ProviderConfig) -> AppResult<ProviderUsageDto> {
        if cfg.kind != "opencodego" {
            return Ok(ProviderUsageDto {
                meters: vec![],
                message: Some("该提供商不提供用量查询接口".into()),
            });
        }
        let resp = self
            .get(cfg, &cfg.usage_url())
            .send()
            .await
            .map_err(AppError::network)?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Ok(ProviderUsageDto {
                meters: vec![],
                message: Some(format!(
                    "用量接口返回 HTTP {}：{}",
                    status.as_u16(),
                    short(&text, 240)
                )),
            });
        }
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let mut meters = Vec::new();
        if let Some(usage) = value.get("usage").and_then(|v| v.as_object()) {
            for key in ["rolling", "weekly", "monthly"] {
                if let Some(m) = usage.get(key) {
                    meters.push(UsageMeterDto {
                        window: key.to_string(),
                        percent: m.get("percent").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        status: m
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("ok")
                            .to_string(),
                        resets_at: m
                            .get("resetsAt")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    });
                }
            }
        }
        Ok(ProviderUsageDto {
            meters,
            message: None,
        })
    }

    pub async fn test(&self, cfg: &ProviderConfig) -> ProviderTestResult {
        match self.list_models(cfg).await {
            Ok(models) if !models.is_empty() => ProviderTestResult {
                ok: true,
                models,
                message: "连接成功".into(),
            },
            Ok(_) => {
                // Some OpenAI-compatible gateways don't expose /models. Try a tiny chat.
                match self
                    .complete_text(cfg, "default", "你是连接测试助手。", "只回复 OK", 8)
                    .await
                {
                    Ok(_) => ProviderTestResult {
                        ok: true,
                        models: vec![],
                        message: "连接成功（该端点不返回模型列表，请在设置中手动填写模型 ID）"
                            .into(),
                    },
                    Err(err) => ProviderTestResult {
                        ok: false,
                        models: vec![],
                        message: err.message,
                    },
                }
            }
            Err(err) => ProviderTestResult {
                ok: false,
                models: vec![],
                message: format!("{}：{}", err.message, err.detail.unwrap_or_default()),
            },
        }
    }

    pub async fn chat_stream(
        &self,
        cfg: &ProviderConfig,
        req: &ChatRequest,
        on_event: &mut (dyn FnMut(LlmEvent) + Send),
    ) -> AppResult<ChatOutcome> {
        if cfg.kind == "anthropic" {
            self.anthropic_stream(cfg, req, on_event).await
        } else {
            self.openai_stream(cfg, req, on_event).await
        }
    }

    fn openai_body(&self, req: &ChatRequest, stream: bool, include_usage: bool) -> Value {
        let mut messages = Vec::new();
        for msg in &req.messages {
            let mut item = json!({ "role": msg.role, "content": msg.content });
            if msg.role == "assistant" && !msg.tool_calls.is_empty() {
                item = json!({
                    "role": "assistant",
                    "content": msg.content,
                    "tool_calls": msg.tool_calls.iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments.to_string()
                        }
                    })).collect::<Vec<_>>()
                });
            }
            if msg.role == "tool" {
                item = json!({
                    "role": "tool",
                    "tool_call_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": msg.content
                });
            }
            messages.push(item);
        }
        let mut body = json!({
            "model": req.model,
            "messages": messages,
            "stream": stream,
        });
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = req.max_tokens {
            body["max_tokens"] = json!(m);
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|t| json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    }))
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = json!("auto");
        }
        if req.json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        if stream && include_usage {
            body["stream_options"] = json!({ "include_usage": true });
        }
        body
    }

    fn anthropic_body(
        &self,
        req: &ChatRequest,
        stream: bool,
        forced_tool: Option<(&str, &Value)>,
    ) -> Value {
        let mut system_parts = Vec::new();
        let mut messages = Vec::new();
        for msg in &req.messages {
            match msg.role.as_str() {
                "system" => system_parts.push(msg.content.clone()),
                "tool" => {
                    messages.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                            "content": msg.content
                        }]
                    }));
                }
                "assistant" => {
                    let mut blocks: Vec<Value> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(json!({ "type": "text", "text": msg.content }));
                    }
                    for tc in &msg.tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments
                        }));
                    }
                    if blocks.is_empty() {
                        blocks.push(json!({ "type": "text", "text": "" }));
                    }
                    messages.push(json!({ "role": "assistant", "content": blocks }));
                }
                _ => messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": msg.content }]
                })),
            }
        }
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens.unwrap_or(4096),
            "messages": messages,
            "stream": stream,
        });
        if !system_parts.is_empty() {
            body["system"] = json!(system_parts.join("\n\n"));
        }
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some((name, schema)) = forced_tool {
            body["tools"] =
                json!([{ "name": name, "description": "输出最终结果", "input_schema": schema }]);
            body["tool_choice"] = json!({ "type": "tool", "name": name });
        } else if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools
                    .iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    }))
                    .collect::<Vec<_>>()
            );
            body["tool_choice"] = json!({ "type": "auto" });
        }
        body
    }

    async fn openai_stream(
        &self,
        cfg: &ProviderConfig,
        req: &ChatRequest,
        on_event: &mut (dyn FnMut(LlmEvent) + Send),
    ) -> AppResult<ChatOutcome> {
        let body = self.openai_body(req, true, cfg.kind != "deepseek");
        let resp = self
            .request(cfg, &req.session_id, &cfg.chat_url())
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(AppError::network)?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_error(cfg, status.as_u16(), &text));
        }
        let mut outcome = ChatOutcome::default();
        let mut tool_acc: BTreeMap<i64, (String, String, String)> = BTreeMap::new();
        let mut parser = SseParser::default();
        let mut stream = resp.bytes_stream();
        loop {
            if cancelled(req) {
                outcome.finish_reason = Some("cancelled".into());
                break;
            }
            let next = tokio::time::timeout(Duration::from_secs(180), stream.next()).await;
            let chunk = match next {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(err))) => return Err(AppError::network(err)),
                Ok(None) => break,
                Err(_) => {
                    return Err(AppError::new("E_PROVIDER_TIMEOUT", "模型响应超时")
                        .with_detail("超过 180 秒未收到新的流式数据")
                        .retryable());
                }
            };
            let text = String::from_utf8_lossy(&chunk);
            for event in parser.push(&text) {
                let data = event.data.trim().to_string();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let value: Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(err) = value.get("error") {
                    return Err(provider_error_from_body(cfg, err));
                }
                if let Some(choices) = value.get("choices").and_then(|v| v.as_array()) {
                    for choice in choices {
                        if let Some(delta) = choice.get("delta") {
                            if let Some(s) = delta.get("content").and_then(|v| v.as_str()) {
                                if !s.is_empty() {
                                    outcome.content.push_str(s);
                                    on_event(LlmEvent::Text(s.to_string()));
                                }
                            }
                            if let Some(s) = delta.get("reasoning_content").and_then(|v| v.as_str())
                            {
                                if !s.is_empty() {
                                    outcome.reasoning.push_str(s);
                                    on_event(LlmEvent::Reasoning(s.to_string()));
                                }
                            }
                            if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array())
                            {
                                for call in calls {
                                    let idx =
                                        call.get("index").and_then(|v| v.as_i64()).unwrap_or(0);
                                    let entry = tool_acc.entry(idx).or_default();
                                    if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                                        entry.0 = id.to_string();
                                    }
                                    if let Some(name) = call
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|v| v.as_str())
                                    {
                                        entry.1 = name.to_string();
                                    }
                                    if let Some(args) = call
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|v| v.as_str())
                                    {
                                        entry.2.push_str(args);
                                    }
                                }
                            }
                        }
                        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                            outcome.finish_reason = Some(reason.to_string());
                        }
                    }
                }
                if let Some(usage) = value.get("usage") {
                    if !usage.is_null() {
                        outcome.usage = Some(usage.clone());
                        on_event(LlmEvent::Usage(usage.clone()));
                    }
                }
            }
        }
        for (_, (id, name, args)) in tool_acc {
            if name.is_empty() {
                continue;
            }
            let arguments = serde_json::from_str(&args).unwrap_or(Value::String(args));
            let call = ToolCall {
                id,
                name,
                arguments,
            };
            on_event(LlmEvent::ToolCall(call.clone()));
            outcome.tool_calls.push(call);
        }
        Ok(outcome)
    }

    async fn anthropic_stream(
        &self,
        cfg: &ProviderConfig,
        req: &ChatRequest,
        on_event: &mut (dyn FnMut(LlmEvent) + Send),
    ) -> AppResult<ChatOutcome> {
        let body = self.anthropic_body(req, true, None);
        let resp = self
            .request(cfg, &req.session_id, &cfg.chat_url())
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(AppError::network)?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_error(cfg, status.as_u16(), &text));
        }
        let mut outcome = ChatOutcome::default();
        let mut blocks: BTreeMap<i64, (String, String, String, String)> = BTreeMap::new();
        let mut parser = SseParser::default();
        let mut stream = resp.bytes_stream();
        loop {
            if cancelled(req) {
                outcome.finish_reason = Some("cancelled".into());
                break;
            }
            let next = tokio::time::timeout(Duration::from_secs(180), stream.next()).await;
            let chunk = match next {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(err))) => return Err(AppError::network(err)),
                Ok(None) => break,
                Err(_) => {
                    return Err(AppError::new("E_PROVIDER_TIMEOUT", "模型响应超时")
                        .with_detail("超过 180 秒未收到新的流式数据")
                        .retryable());
                }
            };
            let text = String::from_utf8_lossy(&chunk);
            for event in parser.push(&text) {
                if event.data.trim().is_empty() {
                    continue;
                }
                let value: Value = match serde_json::from_str(event.data.trim()) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let kind = value
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or(event.event.as_deref().unwrap_or(""));
                match kind {
                    "error" => {
                        return Err(provider_error_from_body(cfg, &value));
                    }
                    "message_start" => {
                        if let Some(usage) = value.pointer("/message/usage") {
                            outcome.usage = Some(usage.clone());
                        }
                    }
                    "content_block_start" => {
                        let index = value.get("index").and_then(|v| v.as_i64()).unwrap_or(0);
                        let block = value.get("content_block").cloned().unwrap_or(Value::Null);
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let id = block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            blocks.insert(index, (id, name, String::new(), "tool".into()));
                        } else {
                            blocks.insert(
                                index,
                                (String::new(), String::new(), String::new(), "text".into()),
                            );
                        }
                    }
                    "content_block_delta" => {
                        let index = value.get("index").and_then(|v| v.as_i64()).unwrap_or(0);
                        let delta = value.get("delta").cloned().unwrap_or(Value::Null);
                        match delta.get("type").and_then(|v| v.as_str()) {
                            Some("text_delta") => {
                                if let Some(s) = delta.get("text").and_then(|v| v.as_str()) {
                                    outcome.content.push_str(s);
                                    on_event(LlmEvent::Text(s.to_string()));
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(s) = delta.get("thinking").and_then(|v| v.as_str()) {
                                    outcome.reasoning.push_str(s);
                                    on_event(LlmEvent::Reasoning(s.to_string()));
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(s) = delta.get("partial_json").and_then(|v| v.as_str())
                                {
                                    if let Some(entry) = blocks.get_mut(&index) {
                                        entry.2.push_str(s);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) =
                            value.pointer("/delta/stop_reason").and_then(|v| v.as_str())
                        {
                            outcome.finish_reason = Some(reason.to_string());
                        }
                        if let Some(usage) = value.get("usage") {
                            outcome.usage = Some(usage.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        for (_, (id, name, args, kind)) in blocks {
            if kind != "tool" || name.is_empty() {
                continue;
            }
            let arguments = serde_json::from_str(&args).unwrap_or(Value::String(args));
            let call = ToolCall {
                id,
                name,
                arguments,
            };
            on_event(LlmEvent::ToolCall(call.clone()));
            outcome.tool_calls.push(call);
        }
        Ok(outcome)
    }

    pub async fn complete_text(
        &self,
        cfg: &ProviderConfig,
        model: &str,
        system: &str,
        user: &str,
        max_tokens: i64,
    ) -> AppResult<String> {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![LlmMessage::system(system), LlmMessage::user(user)],
            tools: vec![],
            temperature: Some(0.3),
            max_tokens: Some(max_tokens),
            json_mode: false,
            session_id: cfg.session_id.clone(),
            cancel: None,
        };
        let body = if cfg.is_anthropic() {
            self.anthropic_body(&req, false, None)
        } else {
            self.openai_body(&req, false, false)
        };
        let value = self.post_json(cfg, &cfg.chat_url(), &body).await?;
        if cfg.is_anthropic() {
            let mut out = String::new();
            if let Some(blocks) = value.get("content").and_then(|v| v.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            out.push_str(text);
                        }
                    }
                }
            }
            Ok(out.trim().to_string())
        } else {
            Ok(extract_openai_content(&value))
        }
    }

    pub async fn complete_json(
        &self,
        cfg: &ProviderConfig,
        model: &str,
        system: &str,
        user: &str,
        schema: Value,
        max_tokens: i64,
    ) -> AppResult<Value> {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![LlmMessage::system(system), LlmMessage::user(user)],
            tools: vec![],
            temperature: Some(0.2),
            max_tokens: Some(max_tokens),
            json_mode: !cfg.is_anthropic(),
            session_id: cfg.session_id.clone(),
            cancel: None,
        };
        let body = if cfg.is_anthropic() {
            self.anthropic_body(&req, false, Some(("submit_result", &schema)))
        } else {
            self.openai_body(&req, false, false)
        };
        let value = self.post_json(cfg, &cfg.chat_url(), &body).await?;
        let raw = if cfg.is_anthropic() {
            let mut tool_input: Option<Value> = None;
            if let Some(blocks) = value.get("content").and_then(|v| v.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                        tool_input = block.get("input").cloned();
                    }
                }
            }
            match tool_input {
                Some(input) => return Ok(input),
                None => extract_anthropic_text(&value),
            }
        } else {
            extract_openai_content(&value)
        };
        match serde_json::from_str::<Value>(&strip_code_fence(&raw)) {
            Ok(parsed) => Ok(parsed),
            Err(err) => {
                let repaired_user = format!(
                    "{user}\n\n你上一次的输出不是合法 JSON（{err}）。请只输出一个合法 JSON 对象，不要 markdown 代码块。上一次输出：\n{raw}"
                );
                self.complete_json_repair(cfg, model, system, &repaired_user, schema, max_tokens)
                    .await
            }
        }
    }

    /// Streaming variant used when the caller wants to show the agent's raw
    /// output while it is being produced (training-plan diagnosis, etc.).
    #[allow(clippy::too_many_arguments)]
    pub async fn complete_json_stream<F>(
        &self,
        cfg: &ProviderConfig,
        model: &str,
        system: &str,
        user: &str,
        schema: Value,
        max_tokens: i64,
        on_delta: &mut F,
    ) -> AppResult<Value>
    where
        F: FnMut(&str) + Send,
    {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![LlmMessage::system(system), LlmMessage::user(user)],
            tools: vec![],
            temperature: Some(0.2),
            max_tokens: Some(max_tokens),
            json_mode: !cfg.is_anthropic(),
            session_id: cfg.session_id.clone(),
            cancel: None,
        };
        let mut buffer = String::new();
        {
            let mut on_event = |event: LlmEvent| {
                if let LlmEvent::Text(delta) = event {
                    buffer.push_str(&delta);
                    on_delta(&delta);
                }
            };
            self.chat_stream(cfg, &req, &mut on_event).await?;
        }
        if buffer.trim().is_empty() {
            // Some providers emit no text deltas (or only reasoning); use the
            // regular JSON path so the plan still gets a valid object.
            return self
                .complete_json(cfg, model, system, user, schema, max_tokens)
                .await;
        }
        match serde_json::from_str::<Value>(&strip_code_fence(&buffer)) {
            Ok(parsed) => Ok(parsed),
            Err(err) => {
                let repaired_user = format!(
                    "{user}\n\n你上一次的输出不是合法 JSON（{err}）。请只输出一个合法 JSON 对象，不要 markdown 代码块。上一次输出：\n{buffer}"
                );
                self.complete_json_repair(cfg, model, system, &repaired_user, schema, max_tokens)
                    .await
            }
        }
    }

    async fn complete_json_repair(
        &self,
        cfg: &ProviderConfig,
        model: &str,
        system: &str,
        user: &str,
        schema: Value,
        max_tokens: i64,
    ) -> AppResult<Value> {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![LlmMessage::system(system), LlmMessage::user(user)],
            tools: vec![],
            temperature: Some(0.0),
            max_tokens: Some(max_tokens),
            json_mode: !cfg.is_anthropic(),
            session_id: cfg.session_id.clone(),
            cancel: None,
        };
        let body = if cfg.is_anthropic() {
            self.anthropic_body(&req, false, Some(("submit_result", &schema)))
        } else {
            self.openai_body(&req, false, false)
        };
        let value = self.post_json(cfg, &cfg.chat_url(), &body).await?;
        if cfg.is_anthropic() {
            if let Some(blocks) = value.get("content").and_then(|v| v.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                        return Ok(block.get("input").cloned().unwrap_or(Value::Null));
                    }
                }
            }
            let raw = extract_anthropic_text(&value);
            return serde_json::from_str(&strip_code_fence(&raw)).map_err(|e| AppError {
                code: "E_LLM_PARSE".into(),
                message: "模型没有返回合法 JSON".into(),
                detail: Some(e.to_string()),
                retryable: true,
            });
        }
        let raw = extract_openai_content(&value);
        serde_json::from_str(&strip_code_fence(&raw)).map_err(|e| AppError {
            code: "E_LLM_PARSE".into(),
            message: "模型没有返回合法 JSON".into(),
            detail: Some(e.to_string()),
            retryable: true,
        })
    }

    async fn post_json(&self, cfg: &ProviderConfig, url: &str, body: &Value) -> AppResult<Value> {
        let mut last_error: Option<AppError> = None;
        for attempt in 0..3 {
            let resp = self
                .request(cfg, &cfg.session_id, url)
                .json(body)
                .send()
                .await;
            match resp {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        return serde_json::from_str(&text).map_err(|e| AppError {
                            code: "E_PROVIDER_RESPONSE".into(),
                            message: "模型返回格式无法解析".into(),
                            detail: Some(format!("{e}: {}", short(&text, 500))),
                            retryable: false,
                        });
                    }
                    let err = map_http_error(cfg, status.as_u16(), &text);
                    if err.retryable && attempt < 2 {
                        last_error = Some(err);
                        tokio::time::sleep(Duration::from_millis(800 * (attempt as u64 + 1))).await;
                        continue;
                    }
                    return Err(err);
                }
                Err(err) => {
                    let mapped = AppError::network(err);
                    if attempt < 2 {
                        last_error = Some(mapped);
                        tokio::time::sleep(Duration::from_millis(800 * (attempt as u64 + 1))).await;
                        continue;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| AppError::new("E_PROVIDER", "模型请求失败")))
    }
}

fn cancelled(req: &ChatRequest) -> bool {
    req.cancel
        .as_ref()
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn extract_openai_content(value: &Value) -> String {
    value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn extract_anthropic_text(value: &Value) -> String {
    let mut out = String::new();
    if let Some(blocks) = value.get("content").and_then(|v| v.as_array()) {
        for block in blocks {
            if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
        }
    }
    out
}

fn short(input: &str, max: usize) -> String {
    let s: String = input.chars().take(max).collect();
    if input.chars().count() > max {
        format!("{s}…")
    } else {
        s
    }
}

fn provider_error_from_body(cfg: &ProviderConfig, err: &Value) -> AppError {
    let message = err
        .get("message")
        .and_then(|v| v.as_str())
        .or_else(|| {
            err.get("error")
                .and_then(|v| v.get("message"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("模型提供商返回错误");
    AppError::provider("E_PROVIDER", message)
        .with_detail(format!("provider={} kind={}", cfg.id, cfg.kind))
}

pub fn map_http_error(cfg: &ProviderConfig, status: u16, body: &str) -> AppError {
    let detail = short(body, 1200);
    let provider_message = extract_error_message(body);
    match status {
        401 => AppError::provider(
            "E_PROVIDER_AUTH",
            format!("API Key 无效或已过期（{}）", cfg.kind),
        )
        .with_detail(detail),
        403 => {
            let mut message = format!("没有访问该模型的权限（{}）", cfg.kind);
            if cfg.kind == "opencodego" {
                message = "OpenCode Go 拒绝访问：请确认订阅有效；DeepSeek V4.1 Flash 需要在 Console → Privacy → Regions 选择 Global".into();
            }
            AppError::provider("E_PROVIDER_FORBIDDEN", message).with_detail(
                if provider_message.is_empty() {
                    detail
                } else {
                    format!("{provider_message}\n{detail}")
                },
            )
        }
        404 => AppError::provider(
            "E_PROVIDER_ENDPOINT",
            "模型接口不存在，请检查 Base URL 与模型 ID",
        )
        .with_detail(detail),
        429 => AppError::provider(
            "E_PROVIDER_LIMIT",
            format!("请求过于频繁或额度已用尽（{}）", cfg.kind),
        )
        .with_detail(detail)
        .retryable(),
        s if s >= 500 => AppError::provider("E_PROVIDER_UNAVAILABLE", "模型服务暂时不可用")
            .with_detail(detail)
            .retryable(),
        _ => AppError::provider("E_PROVIDER_HTTP", format!("模型请求失败（HTTP {status}）"))
            .with_detail(detail),
    }
}

fn extract_error_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(msg) = value
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("message").and_then(|v| v.as_str()))
        {
            return msg.to_string();
        }
    }
    String::new()
}

#[derive(Debug, Default)]
struct SseParser {
    buffer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

impl SseParser {
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        self.buffer = self.buffer.replace("\r\n", "\n");
        let mut events = Vec::new();
        while let Some(pos) = self.buffer.find("\n\n") {
            let raw = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + 2);
            if raw.trim().is_empty() {
                continue;
            }
            let mut event = None;
            let mut data_lines: Vec<String> = Vec::new();
            for line in raw.lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    event = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim_start().to_string());
                }
            }
            if !data_lines.is_empty() {
                events.push(SseEvent {
                    event,
                    data: data_lines.join("\n"),
                });
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(kind: &str, base: &str) -> ProviderConfig {
        ProviderConfig {
            id: "p1".into(),
            kind: kind.into(),
            base_url: base.into(),
            api_key: "secret".into(),
            session_id: "ses_test".into(),
        }
    }

    #[test]
    fn builds_provider_urls() {
        let go = cfg("opencodego", "https://opencode.ai/inference/go/");
        assert_eq!(
            go.chat_url(),
            "https://opencode.ai/inference/go/openai/v1/chat/completions"
        );
        assert_eq!(
            go.models_url(),
            "https://opencode.ai/inference/go/v1/models"
        );
        assert_eq!(go.usage_url(), "https://opencode.ai/inference/go/v1/usage");
        let ds = cfg("deepseek", "https://api.deepseek.com");
        assert_eq!(ds.chat_url(), "https://api.deepseek.com/chat/completions");
        let an = cfg("anthropic", "https://api.anthropic.com/v1");
        assert_eq!(an.chat_url(), "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn parses_sse_chunks_split_anywhere() {
        let mut parser = SseParser::default();
        assert!(parser.push("data: {\"a\":1}\n").is_empty());
        let events = parser.push("\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"a\":1}");
    }

    #[test]
    fn parses_event_and_multiline_data() {
        let mut parser = SseParser::default();
        let events =
            parser.push("event: content_block_delta\ndata: {\"x\":1}\ndata: {\"y\":2}\n\n");
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(events[0].data, "{\"x\":1}\n{\"y\":2}");
    }

    #[test]
    fn maps_errors() {
        let go = cfg("opencodego", "https://opencode.ai/inference/go");
        let err = map_http_error(&go, 403, "{\"error\":{\"message\":\"region\"}}");
        assert_eq!(err.code, "E_PROVIDER_FORBIDDEN");
        assert!(err.message.contains("Global"));
    }
}
