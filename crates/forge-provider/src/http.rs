use crate::{sse::SseParser, *};
use forge_protocol::{MessageRole, ProviderSettings, Usage};
use reqwest::{Client, Response, header::HeaderValue};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

pub const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com";
pub const OPENAI_ENDPOINT: &str = "https://api.openai.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

pub trait ProviderFactory: Send + Sync {
    fn create(
        &self,
        settings: &ProviderSettings,
        key: Option<String>,
    ) -> Result<Arc<dyn Provider>, ProviderError>;
}
pub struct HttpProviderFactory;
impl ProviderFactory for HttpProviderFactory {
    fn create(
        &self,
        settings: &ProviderSettings,
        key: Option<String>,
    ) -> Result<Arc<dyn Provider>, ProviderError> {
        match settings.id.0.as_str() {
            "anthropic" => Ok(Arc::new(AnthropicProvider::new(settings.clone(), key)?)),
            "openai" => Ok(Arc::new(OpenAiProvider::new(settings.clone(), key)?)),
            _ => Err(configuration("unknown provider")),
        }
    }
}

#[derive(Clone, Copy)]
enum Api {
    Anthropic,
    OpenAi,
}
impl Api {
    fn error_code(self, error: &Value) -> Option<String> {
        match self {
            Self::Anthropic => error["type"].as_str(),
            Self::OpenAi => error["code"].as_str().or_else(|| error["type"].as_str()),
        }
        .map(str::to_owned)
    }
}
struct HttpProvider {
    client: Client,
    settings: ProviderSettings,
    key: Option<String>,
    api: Api,
}
pub struct AnthropicProvider(HttpProvider);
pub struct OpenAiProvider(HttpProvider);
macro_rules! provider {
    ($name:ident,$api:ident) => {
        impl $name {
            pub fn new(
                settings: ProviderSettings,
                key: Option<String>,
            ) -> Result<Self, ProviderError> {
                Ok(Self(HttpProvider {
                    client: Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .map_err(|_| configuration("HTTP client initialization failed"))?,
                    settings,
                    key,
                    api: Api::$api,
                }))
            }
        }
        impl Provider for $name {
            fn request(
                &self,
                request: ProviderRequest,
            ) -> ProviderFuture<'_, Result<Box<dyn ProviderStream>, ProviderError>> {
                Box::pin(self.0.request(request))
            }
        }
    };
}
provider!(AnthropicProvider, Anthropic);
provider!(OpenAiProvider, OpenAi);

pub(crate) fn configuration(message: &str) -> ProviderError {
    ProviderError {
        code: None,
        kind: ErrorKind::Configuration,
        message: message.into(),
    }
}
fn invalid() -> ProviderError {
    ProviderError {
        code: None,
        kind: ErrorKind::InvalidResponse,
        message: "malformed provider payload".into(),
    }
}
fn string(value: &Value) -> Result<&str, ProviderError> {
    value.as_str().ok_or_else(invalid)
}
fn number(value: &Value) -> Result<u64, ProviderError> {
    value.as_u64().ok_or_else(invalid)
}
fn array(value: &Value) -> Result<&Vec<Value>, ProviderError> {
    value.as_array().ok_or_else(invalid)
}

impl HttpProvider {
    async fn request(
        &self,
        request: ProviderRequest,
    ) -> Result<Box<dyn ProviderStream>, ProviderError> {
        let key = self
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| configuration("provider key is missing"))?;
        let endpoint = self.settings.endpoint.as_deref().unwrap_or(match self.api {
            Api::Anthropic => ANTHROPIC_ENDPOINT,
            Api::OpenAi => OPENAI_ENDPOINT,
        });
        let path = match self.api {
            Api::Anthropic => "/v1/messages",
            Api::OpenAi => "/v1/chat/completions",
        };
        let url = reqwest::Url::parse(&format!("{}{path}", endpoint.trim_end_matches('/')))
            .map_err(|_| configuration("invalid provider endpoint"))?;
        let mut messages = encode_messages(self.api, &request.messages)?;
        let mut body = json!({"model":request.model.model,"stream":true});
        let mut builder = self.client.post(url);
        match self.api {
            Api::Anthropic => {
                let mut header = HeaderValue::from_str(key)
                    .map_err(|_| configuration("invalid provider key header"))?;
                header.set_sensitive(true);
                builder = builder
                    .header("x-api-key", header)
                    .header("anthropic-version", ANTHROPIC_VERSION);
                body["max_tokens"] = json!(
                    self.settings
                        .max_output_tokens
                        .ok_or_else(|| configuration("Anthropic requires max_output_tokens"))?
                );
                if let Some(system) = request.system {
                    body["system"] = json!(system);
                }
                if !request.tools.is_empty() {
                    body["tools"] = request.tools.iter().map(|tool| Ok(json!({"name":tool.name,"description":tool.description,"input_schema":serde_json::from_str::<Value>(&tool.parameters_json).map_err(|_| configuration("invalid tool schema"))?}))).collect::<Result<Vec<_>,ProviderError>>()?.into();
                }
            }
            Api::OpenAi => {
                builder = builder.bearer_auth(key);
                body["stream_options"] = json!({"include_usage":true});
                if let Some(tokens) = self.settings.max_output_tokens {
                    body["max_completion_tokens"] = json!(tokens);
                }
                if let Some(system) = request.system {
                    messages.insert(0, json!({"role":"system","content":system}));
                }
                if !request.tools.is_empty() {
                    body["tools"] = request.tools.iter().map(|tool| Ok(json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":serde_json::from_str::<Value>(&tool.parameters_json).map_err(|_| configuration("invalid tool schema"))?}}))).collect::<Result<Vec<_>,ProviderError>>()?.into();
                }
            }
        }
        body["messages"] = messages.into();
        let response = builder
            .json(&body)
            .send()
            .await
            .map_err(|error| ProviderError {
                code: None,
                kind: if error.is_builder() {
                    ErrorKind::Configuration
                } else {
                    ErrorKind::Network
                },
                message: "provider request failed".into(),
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    value
                        .parse::<u64>()
                        .ok()
                        .map(|seconds| RetryAfter::Delay(Duration::from_secs(seconds)))
                        .or_else(|| httpdate::parse_http_date(value).ok().map(RetryAfter::At))
                });
            let bytes = response.bytes().await.map_err(|_| ProviderError {
                code: None,
                kind: ErrorKind::Http {
                    status,
                    retry_after: retry_after.clone(),
                },
                message: "provider error body disconnected".into(),
            })?;
            let (code, message) = match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => (self.api.error_code(&value["error"]), value.to_string()),
                Err(_) => (None, String::from_utf8_lossy(&bytes).into_owned()),
            };
            return Err(ProviderError {
                code,
                kind: ErrorKind::Http {
                    status,
                    retry_after,
                },
                message,
            }
            .redacted(key));
        }
        Ok(Box::new(HttpStream {
            response,
            parser: SseParser::default(),
            decoder: Decoder::new(self.api),
            pending: VecDeque::new(),
            done: false,
            key: key.into(),
        }))
    }
}

/// Provider-facing message forms: Anthropic carries assistant tool calls as
/// `tool_use` content blocks and answers them with `tool_result` blocks inside
/// a user message; OpenAI carries `tool_calls` on the assistant message and
/// answers each with a `role: tool` message keyed by `tool_call_id`.
fn encode_messages(api: Api, messages: &[ProviderMessage]) -> Result<Vec<Value>, ProviderError> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        match api {
            Api::Anthropic => anthropic_message(&mut out, message)?,
            Api::OpenAi => openai_message(&mut out, message)?,
        }
    }
    Ok(out)
}
/// Append a user-role content block, merging into a trailing user message so
/// consecutive user content stays in one Anthropic message.
fn anthropic_user_block(out: &mut Vec<Value>, block: Value) {
    if let Some(last) = out.last_mut()
        && last["role"] == "user"
    {
        if let Some(blocks) = last["content"].as_array_mut() {
            blocks.push(block);
        } else {
            let text = last["content"].take();
            last["content"] = json!([{"type":"text","text":text}, block]);
        }
        return;
    }
    out.push(json!({"role":"user","content":[block]}));
}
fn anthropic_message(out: &mut Vec<Value>, message: &ProviderMessage) -> Result<(), ProviderError> {
    match message.role {
        MessageRole::User => {
            if matches!(out.last(), Some(last) if last["role"] == "user") {
                anthropic_user_block(out, json!({"type":"text","text":message.content.text}));
            } else {
                out.push(json!({"role":"user","content":message.content.text}));
            }
        }
        MessageRole::Assistant => {
            let mut content = Vec::new();
            if !message.content.text.is_empty() {
                content.push(json!({"type":"text","text":message.content.text}));
            }
            for call in &message.tool_calls {
                content.push(json!({
                    "type":"tool_use","id":call.id.0,"name":call.name,
                    "input":serde_json::from_str::<Value>(&call.arguments_json).unwrap_or_else(|_| json!({})),
                }));
            }
            match (content.len(), message.tool_calls.is_empty()) {
                (0, _) => {}
                (1, true) => out.push(json!({"role":"assistant","content":message.content.text})),
                _ => out.push(json!({"role":"assistant","content":content})),
            }
        }
        MessageRole::Tool => {
            let mut block = json!({
                "type":"tool_result",
                "tool_use_id":message.tool_call_id.as_ref().ok_or_else(|| configuration("tool result message is missing tool_call_id"))?.0,
                "content":message.content.text,
            });
            if message.is_error {
                block["is_error"] = true.into();
            }
            anthropic_user_block(out, block);
        }
    }
    Ok(())
}
fn openai_message(out: &mut Vec<Value>, message: &ProviderMessage) -> Result<(), ProviderError> {
    match message.role {
        MessageRole::Assistant if !message.tool_calls.is_empty() => out.push(json!({
            "role":"assistant",
            "content":message.content.text,
            "tool_calls":message.tool_calls.iter().map(|call| json!({
                "id":call.id.0,"type":"function",
                "function":{"name":call.name,"arguments":call.arguments_json}
            })).collect::<Vec<_>>(),
        })),
        MessageRole::Assistant if message.content.text.is_empty() => {}
        MessageRole::Tool => out.push(json!({
            "role":"tool",
            "tool_call_id":message.tool_call_id.as_ref().ok_or_else(|| configuration("tool result message is missing tool_call_id"))?.0,
            "content":message.content.text,
        })),
        _ => out.push(json!({"role":message.role,"content":message.content.text})),
    }
    Ok(())
}

struct HttpStream {
    response: Response,
    parser: SseParser,
    decoder: Decoder,
    pending: VecDeque<StreamEvent>,
    done: bool,
    key: String,
}
impl ProviderStream for HttpStream {
    fn next(&mut self) -> ProviderFuture<'_, Option<Result<StreamEvent, ProviderError>>> {
        Box::pin(async move {
            loop {
                if let Some(event) = self.pending.pop_front() {
                    return Some(Ok(event));
                }
                if self.done {
                    return None;
                }
                if let Some(data) = self.parser.events.pop_front() {
                    match self.decoder.decode(&data) {
                        Ok(events) => {
                            self.done = events.contains(&StreamEvent::Finished);
                            self.pending.extend(events);
                        }
                        Err(error) => {
                            self.done = true;
                            return Some(Err(error.redacted(&self.key)));
                        }
                    }
                } else {
                    match self.response.chunk().await {
                        Ok(Some(chunk)) => {
                            if let Err(error) = self.parser.push(&chunk) {
                                self.done = true;
                                return Some(Err(error));
                            }
                        }
                        Ok(None) | Err(_) => {
                            self.done = true;
                            return Some(Err(ProviderError {
                                code: None,
                                kind: ErrorKind::Disconnected,
                                message: "provider stream ended before its terminal event".into(),
                            }));
                        }
                    }
                }
            }
        })
    }
}

struct Decoder {
    api: Api,
    tools: BTreeMap<(u64, u64), ToolCallId>,
    usage: Usage,
}
impl Decoder {
    fn new(api: Api) -> Self {
        Self {
            api,
            tools: BTreeMap::new(),
            usage: Usage {
                requests: 1,
                ..Usage::default()
            },
        }
    }
    fn decode(&mut self, data: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        if matches!(self.api, Api::OpenAi) && data == "[DONE]" {
            return Ok(vec![StreamEvent::Finished]);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| invalid())?;
        if !value["error"].is_null() {
            return Err(stream_error(self.api, &value["error"]));
        }
        let mut events = Vec::new();
        match self.api {
            Api::Anthropic => match string(&value["type"])? {
                "message_start" => {
                    self.anthropic_usage(&value["message"]["usage"])?;
                    events.push(StreamEvent::Usage(self.usage.clone()));
                }
                "content_block_start" => {
                    let block = &value["content_block"];
                    match string(&block["type"])? {
                        "text" => {
                            events.push(StreamEvent::TextDelta(string(&block["text"])?.into()))
                        }
                        "thinking" => events.push(StreamEvent::ReasoningDelta(
                            string(&block["thinking"])?.into(),
                        )),
                        "tool_use" => {
                            let id = ToolCallId(string(&block["id"])?.into());
                            self.tools.insert((0, number(&value["index"])?), id.clone());
                            events.push(StreamEvent::ToolCallStarted {
                                id,
                                name: string(&block["name"])?.into(),
                            });
                        }
                        _ => {}
                    }
                }
                "content_block_delta" => {
                    let delta = &value["delta"];
                    match string(&delta["type"])? {
                        "text_delta" => {
                            events.push(StreamEvent::TextDelta(string(&delta["text"])?.into()))
                        }
                        "thinking_delta" => events.push(StreamEvent::ReasoningDelta(
                            string(&delta["thinking"])?.into(),
                        )),
                        "input_json_delta" => events.push(StreamEvent::ToolArgumentsDelta {
                            id: self
                                .tools
                                .get(&(0, number(&value["index"])?))
                                .ok_or_else(invalid)?
                                .clone(),
                            json: string(&delta["partial_json"])?.into(),
                        }),
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    self.tools.remove(&(0, number(&value["index"])?));
                }
                "message_delta" => {
                    self.anthropic_usage(&value["usage"])?;
                    events.push(StreamEvent::Usage(self.usage.clone()));
                }
                "message_stop" => events.push(StreamEvent::Finished),
                _ => {}
            },
            Api::OpenAi => {
                for choice in array(&value["choices"])? {
                    let index = number(&choice["index"])?;
                    let delta = choice["delta"].as_object().ok_or_else(invalid)?;
                    if let Some(text) = delta.get("content").filter(|v| !v.is_null()) {
                        events.push(StreamEvent::TextDelta(string(text)?.into()));
                    }
                    if let Some(calls) = delta.get("tool_calls").filter(|v| !v.is_null()) {
                        for call in array(calls)? {
                            let call_index = number(&call["index"])?;
                            if let Some(id) = call.get("id") {
                                let id = ToolCallId(string(id)?.into());
                                self.tools.insert((index, call_index), id.clone());
                                events.push(StreamEvent::ToolCallStarted {
                                    id,
                                    name: string(&call["function"]["name"])?.into(),
                                });
                            }
                            if let Some(arguments) = call["function"].get("arguments") {
                                events.push(StreamEvent::ToolArgumentsDelta {
                                    id: self
                                        .tools
                                        .get(&(index, call_index))
                                        .ok_or_else(invalid)?
                                        .clone(),
                                    json: string(arguments)?.into(),
                                });
                            }
                        }
                    }
                }
                if !value["usage"].is_null() {
                    let usage = &value["usage"];
                    self.usage.input_tokens = number(&usage["prompt_tokens"])?;
                    self.usage.output_tokens = number(&usage["completion_tokens"])?;
                    self.usage.cached_tokens =
                        match usage["prompt_tokens_details"].get("cached_tokens") {
                            Some(v) => number(v)?,
                            None => 0,
                        };
                    events.push(StreamEvent::Usage(self.usage.clone()));
                }
            }
        }
        Ok(events)
    }
    fn anthropic_usage(&mut self, value: &Value) -> Result<(), ProviderError> {
        let usage = value.as_object().ok_or_else(invalid)?;
        for (key, target) in [
            ("input_tokens", &mut self.usage.input_tokens),
            ("output_tokens", &mut self.usage.output_tokens),
            ("cache_read_input_tokens", &mut self.usage.cached_tokens),
        ] {
            if let Some(value) = usage.get(key) {
                *target = number(value)?;
            }
        }
        Ok(())
    }
}
fn stream_error(api: Api, value: &Value) -> ProviderError {
    if !value.is_object() || !value["message"].is_string() {
        return invalid();
    }
    ProviderError {
        code: api.error_code(value),
        kind: ErrorKind::Api,
        message: value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anthropic_usage_merges_cumulative_updates_and_new_stream_starts_empty() {
        let mut decoder = Decoder::new(Api::Anthropic);
        decoder.decode(r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":2}}}"#).unwrap();
        let events = decoder
            .decode(r#"{"type":"message_delta","usage":{"output_tokens":5}}"#)
            .unwrap();
        assert_eq!(
            events,
            vec![StreamEvent::Usage(Usage {
                input_tokens: 10,
                output_tokens: 5,
                cached_tokens: 2,
                requests: 1,
                estimated_cost_usd: None
            })]
        );
        let events=decoder.decode(r#"{"type":"message_delta","usage":{"input_tokens":11,"output_tokens":8,"cache_read_input_tokens":4}}"#).unwrap();
        let mut fresh = Decoder::new(Api::Anthropic);
        assert_eq!(events,fresh.decode(r#"{"type":"message_start","message":{"usage":{"input_tokens":11,"output_tokens":8,"cache_read_input_tokens":4}}}"#).unwrap());
        assert_eq!(
            Decoder::new(Api::Anthropic).usage,
            Usage {
                requests: 1,
                ..Usage::default()
            }
        );
    }
    #[test]
    fn anthropic_tool_index_expires_at_block_stop() {
        let mut decoder = Decoder::new(Api::Anthropic);
        decoder.decode(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"first","name":"read"}}"#).unwrap();
        let delta = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#;
        assert_eq!(
            decoder.decode(delta).unwrap(),
            vec![StreamEvent::ToolArgumentsDelta {
                id: "first".into(),
                json: "{}".into()
            }]
        );
        decoder
            .decode(r#"{"type":"content_block_stop","index":0}"#)
            .unwrap();
        assert_eq!(
            decoder.decode(delta).unwrap_err().kind,
            ErrorKind::InvalidResponse
        );
        decoder.decode(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"second","name":"list"}}"#).unwrap();
        assert_eq!(
            decoder.decode(delta).unwrap(),
            vec![StreamEvent::ToolArgumentsDelta {
                id: "second".into(),
                json: "{}".into()
            }]
        );
    }
    #[test]
    fn openai_tool_indices_are_scoped_to_choice_and_request() {
        let mut decoder = Decoder::new(Api::OpenAi);
        decoder.decode(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"first","function":{"name":"read"}}]}},{"index":1,"delta":{"tool_calls":[{"index":0,"id":"second","function":{"name":"list"}}]}}]}"#).unwrap();
        let delta = r#"{"choices":[{"index":1,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}},{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"[]"}}]}}]}"#;
        assert_eq!(
            decoder.decode(delta).unwrap(),
            vec![
                StreamEvent::ToolArgumentsDelta {
                    id: "second".into(),
                    json: "{}".into()
                },
                StreamEvent::ToolArgumentsDelta {
                    id: "first".into(),
                    json: "[]".into()
                }
            ]
        );
        assert_eq!(
            Decoder::new(Api::OpenAi).decode(delta).unwrap_err().kind,
            ErrorKind::InvalidResponse
        );
    }
}
