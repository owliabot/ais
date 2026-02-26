use crate::{
    CompleteWithToolsRequest, CompleteWithToolsResponse, LlmMessage, LlmProvider, LlmProviderError,
    MessageRole, ToolCall, ToolSpec,
};
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    client: Client,
    model: String,
    api_key: String,
    base_url: String,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        model: String,
        api_key: String,
        api_base: Option<String>,
    ) -> Result<Self, LlmProviderError> {
        let base_url = normalize_base_url(api_base.as_deref().unwrap_or(DEFAULT_OPENAI_BASE_URL));
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            // Some gateways/proxies have flaky HTTP/2 behavior (body decode errors).
            // Force HTTP/1.1 for robustness in this synchronous client.
            .http1_only()
            .build()
            .map_err(|error| LlmProviderError::CallFailed {
                reason: format!("http client init failed: {error}"),
            })?;
        Ok(Self {
            client,
            model,
            api_key,
            base_url,
        })
    }
}

impl LlmProvider for OpenAiCompatibleProvider {
    fn complete_with_tools(
        &mut self,
        request: CompleteWithToolsRequest,
    ) -> Result<CompleteWithToolsResponse, LlmProviderError> {
        let payload = to_openai_request(request, self.model.clone());
        let endpoint = format!("{}/chat/completions", self.base_url);
        complete_once(
            &self.client,
            endpoint.as_str(),
            self.api_key.as_str(),
            &payload,
        )
    }
}

fn complete_once(
    client: &Client,
    endpoint: &str,
    api_key: &str,
    payload: &OpenAiChatRequest,
) -> Result<CompleteWithToolsResponse, LlmProviderError> {
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(payload)
        .send()
        .map_err(|error| LlmProviderError::CallFailed {
            reason: format!("request failed endpoint={endpoint}: {error}"),
        })?;

    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .text()
        .map_err(|error| LlmProviderError::CallFailed {
            reason: format!(
                "read body failed endpoint={endpoint} status={} headers={}: {error}",
                status.as_u16(),
                summarize_headers(&headers)
            ),
        })?;
    if !status.is_success() {
        return Err(LlmProviderError::HttpStatus {
            status: status.as_u16(),
            body: truncate_body(&body),
        });
    }
    let decoded: OpenAiChatResponse = serde_json::from_str(body.as_str()).map_err(|error| {
        LlmProviderError::DecodeFailed {
            reason: format!(
                "openai response decode failed endpoint={endpoint} status={} content_type={}: {error}; body={}",
                status.as_u16(),
                content_type(&headers),
                truncate_body(&body)
            ),
        }
    })?;
    from_openai_response(decoded)
}

fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

fn truncate_body(body: &str) -> String {
    const LIMIT: usize = 2048;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    let mut truncated = body[..LIMIT].to_string();
    truncated.push_str("…(truncated)");
    truncated
}

fn content_type(headers: &HeaderMap) -> &str {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
}

fn summarize_headers(headers: &HeaderMap) -> String {
    let mut parts = Vec::new();
    for key in [
        reqwest::header::CONTENT_TYPE,
        reqwest::header::CONTENT_ENCODING,
        reqwest::header::CONTENT_LENGTH,
    ] {
        let name = key.as_str();
        if let Some(value) = headers.get(&key).and_then(|v| v.to_str().ok()) {
            parts.push(format!("{name}={value}"));
        }
    }
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(",")
    }
}

fn to_openai_request(request: CompleteWithToolsRequest, model: String) -> OpenAiChatRequest {
    OpenAiChatRequest {
        model,
        messages: request
            .messages
            .into_iter()
            .map(openai_message_from_llm_message)
            .collect(),
        tools: request
            .tools
            .into_iter()
            .map(openai_tool_from_spec)
            .collect(),
        tool_choice: "auto".to_string(),
    }
}

fn openai_tool_from_spec(spec: ToolSpec) -> OpenAiTool {
    OpenAiTool {
        r#type: "function".to_string(),
        function: OpenAiToolFunction {
            name: spec.name,
            description: spec.description,
            parameters: spec.input_schema,
        },
    }
}

fn openai_message_from_llm_message(message: LlmMessage) -> OpenAiMessage {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    }
    .to_string();

    let content = message.content.unwrap_or_default();
    let tool_calls = if message.tool_calls.is_empty() {
        None
    } else {
        Some(
            message
                .tool_calls
                .into_iter()
                .map(|call| OpenAiToolCall {
                    id: call.id,
                    r#type: "function".to_string(),
                    function: OpenAiToolFunctionCall {
                        name: call.name,
                        arguments: call.arguments.to_string(),
                    },
                })
                .collect(),
        )
    };
    OpenAiMessage {
        role,
        content: if content.is_empty() && tool_calls.is_some() {
            None
        } else {
            Some(content)
        },
        tool_call_id: message.tool_call_id,
        tool_calls,
    }
}

fn from_openai_response(
    response: OpenAiChatResponse,
) -> Result<CompleteWithToolsResponse, LlmProviderError> {
    let choice =
        response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmProviderError::DecodeFailed {
                reason: "openai response has no choices".to_string(),
            })?;
    let message = choice.message;
    let assistant_content = message.content.and_then(|content| {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(content)
        }
    });
    let tool_calls = message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(decode_openai_tool_call)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompleteWithToolsResponse {
        assistant_content,
        tool_calls,
    })
}

fn decode_openai_tool_call(call: OpenAiToolCall) -> Result<ToolCall, LlmProviderError> {
    let arguments = parse_json_or_wrap_string(call.function.arguments)?;
    Ok(ToolCall {
        id: call.id,
        name: call.function.name,
        arguments,
    })
}

fn parse_json_or_wrap_string(raw: String) -> Result<Value, LlmProviderError> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw.as_str()).or_else(|_| Ok(json!({ "raw": raw })))
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    tools: Vec<OpenAiTool>,
    tool_choice: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    r#type: String,
    function: OpenAiToolFunction,
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    r#type: String,
    function: OpenAiToolFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiToolFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[cfg(test)]
mod tests {
    use super::{
        from_openai_response, normalize_base_url, to_openai_request, truncate_body,
        OpenAiChatResponse,
    };
    use crate::{CompleteWithToolsRequest, LlmMessage, MessageRole, ToolSpec};
    use serde_json::json;

    #[test]
    fn openai_request_maps_tools_and_messages() {
        let request = CompleteWithToolsRequest {
            messages: vec![LlmMessage {
                role: MessageRole::User,
                content: Some("hello".to_string()),
                tool_name: None,
                tool_call_id: None,
                tool_calls: vec![],
            }],
            tools: vec![ToolSpec {
                name: "confirm".to_string(),
                description: "confirm action".to_string(),
                input_schema: json!({"type":"object"}),
            }],
        };
        let payload = to_openai_request(request, "gpt-4.1-mini".to_string());
        assert_eq!(payload.model, "gpt-4.1-mini");
        assert_eq!(payload.messages.len(), 1);
        assert_eq!(payload.tools.len(), 1);
    }

    #[test]
    fn openai_response_extracts_tool_calls() {
        let response: OpenAiChatResponse = serde_json::from_value(json!({
            "choices":[
                {
                    "message":{
                        "role":"assistant",
                        "content":"ok",
                        "tool_calls":[
                            {
                                "id":"call-1",
                                "type":"function",
                                "function":{"name":"confirm","arguments":"{\"decision\":\"approve\"}"}
                            }
                        ]
                    }
                }
            ]
        }))
        .expect("valid");
        let parsed = from_openai_response(response).expect("must parse");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "confirm");
        assert_eq!(
            parsed.tool_calls[0].arguments,
            json!({"decision":"approve"})
        );
    }

    #[test]
    fn normalize_base_url_trims_trailing_slash() {
        assert_eq!(
            normalize_base_url("https://openrouter.ai/api/v1/"),
            "https://openrouter.ai/api/v1"
        );
    }

    #[test]
    fn truncate_body_truncates_large_text() {
        let big = "a".repeat(4096);
        let truncated = truncate_body(big.as_str());
        assert!(truncated.len() < big.len());
        assert!(truncated.ends_with("…(truncated)"));
    }
}
