use crate::{
    CompleteWithToolsRequest, CompleteWithToolsResponse, LlmProvider, LlmProviderError,
    MessageRole, ToolCall,
};
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    client: Client,
    model: String,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(
        model: String,
        api_key: String,
        api_base: Option<String>,
    ) -> Result<Self, LlmProviderError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .http1_only()
            .build()
            .map_err(|error| LlmProviderError::CallFailed {
                reason: format!("http client init failed: {error}"),
            })?;
        Ok(Self {
            client,
            model,
            api_key,
            base_url: normalize_base_url(api_base.as_deref().unwrap_or(DEFAULT_ANTHROPIC_BASE_URL)),
        })
    }
}

impl LlmProvider for AnthropicProvider {
    fn complete_with_tools(
        &mut self,
        request: CompleteWithToolsRequest,
    ) -> Result<CompleteWithToolsResponse, LlmProviderError> {
        let payload = to_anthropic_request(request, self.model.clone());
        let endpoint = format!("{}/v1/messages", self.base_url);
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
    payload: &AnthropicRequest,
) -> Result<CompleteWithToolsResponse, LlmProviderError> {
    let response = client
        .post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_API_VERSION)
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
    let decoded: AnthropicResponse = serde_json::from_str(body.as_str()).map_err(|error| {
        LlmProviderError::DecodeFailed {
            reason: format!(
                "anthropic response decode failed endpoint={endpoint} status={} content_type={}: {error}; body={}",
                status.as_u16(),
                content_type(&headers),
                truncate_body(&body)
            ),
        }
    })?;
    from_anthropic_response(decoded)
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

fn to_anthropic_request(request: CompleteWithToolsRequest, model: String) -> AnthropicRequest {
    let mut system_parts = Vec::<String>::new();
    let mut messages = Vec::<AnthropicMessage>::new();
    for message in request.messages {
        match message.role {
            MessageRole::System => {
                if let Some(content) = message.content {
                    if !content.trim().is_empty() {
                        system_parts.push(content);
                    }
                }
            }
            MessageRole::User => messages.push(AnthropicMessage {
                role: "user".to_string(),
                content: AnthropicContent::Text(message.content.unwrap_or_default()),
            }),
            MessageRole::Assistant => {
                if message.tool_calls.is_empty() {
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Text(message.content.unwrap_or_default()),
                    });
                } else {
                    let mut blocks = Vec::<AnthropicBlock>::new();
                    if let Some(content) = message.content {
                        if !content.trim().is_empty() {
                            blocks.push(AnthropicBlock::Text { text: content });
                        }
                    }
                    for call in message.tool_calls {
                        blocks.push(AnthropicBlock::ToolUse {
                            id: call.id,
                            name: call.name,
                            input: call.arguments,
                        });
                    }
                    messages.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Blocks(blocks),
                    });
                }
            }
            MessageRole::Tool => {
                messages.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Blocks(vec![AnthropicBlock::ToolResult {
                        tool_use_id: message.tool_call_id.unwrap_or_default(),
                        content: message.content.unwrap_or_default(),
                    }]),
                });
            }
        }
    }

    AnthropicRequest {
        model,
        max_tokens: 1024,
        system: if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        },
        messages,
        tools: request
            .tools
            .into_iter()
            .map(|tool| AnthropicTool {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            })
            .collect(),
    }
}

fn from_anthropic_response(
    response: AnthropicResponse,
) -> Result<CompleteWithToolsResponse, LlmProviderError> {
    let mut assistant_parts = Vec::<String>::new();
    let mut tool_calls = Vec::<ToolCall>::new();
    for block in response.content {
        match block {
            AnthropicBlock::Text { text } => assistant_parts.push(text),
            AnthropicBlock::ToolUse { id, name, input } => tool_calls.push(ToolCall {
                id,
                name,
                arguments: input,
            }),
            AnthropicBlock::ToolResult { .. } => {}
        }
    }
    let assistant_content = if assistant_parts.is_empty() {
        None
    } else {
        Some(assistant_parts.join("\n"))
    };
    Ok(CompleteWithToolsResponse {
        assistant_content,
        tool_calls,
    })
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicBlock>,
}

#[cfg(test)]
mod tests {
    use super::{from_anthropic_response, to_anthropic_request, truncate_body, AnthropicResponse};
    use crate::{CompleteWithToolsRequest, LlmMessage, MessageRole, ToolCall, ToolSpec};
    use serde_json::json;

    #[test]
    fn anthropic_request_maps_system_and_tool_result() {
        let request = CompleteWithToolsRequest {
            messages: vec![
                LlmMessage {
                    role: MessageRole::System,
                    content: Some("sys".to_string()),
                    tool_name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                },
                LlmMessage {
                    role: MessageRole::Assistant,
                    content: Some("need tool".to_string()),
                    tool_name: None,
                    tool_call_id: None,
                    tool_calls: vec![ToolCall {
                        id: "c1".to_string(),
                        name: "confirm".to_string(),
                        arguments: json!({"x":1}),
                    }],
                },
                LlmMessage {
                    role: MessageRole::Tool,
                    content: Some("{\"ok\":true}".to_string()),
                    tool_name: Some("confirm".to_string()),
                    tool_call_id: Some("c1".to_string()),
                    tool_calls: vec![],
                },
            ],
            tools: vec![ToolSpec {
                name: "confirm".to_string(),
                description: "confirm action".to_string(),
                input_schema: json!({"type":"object"}),
            }],
        };
        let payload = to_anthropic_request(request, "claude".to_string());
        assert_eq!(payload.system.as_deref(), Some("sys"));
        assert_eq!(payload.messages.len(), 2);
        assert_eq!(payload.tools.len(), 1);
    }

    #[test]
    fn anthropic_response_extracts_tool_calls() {
        let response: AnthropicResponse = serde_json::from_value(json!({
            "content":[
                {"type":"text","text":"working"},
                {"type":"tool_use","id":"t1","name":"confirm","input":{"decision":"approve"}}
            ]
        }))
        .expect("valid");
        let parsed = from_anthropic_response(response).expect("parse");
        assert_eq!(parsed.assistant_content.as_deref(), Some("working"));
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "confirm");
    }

    #[test]
    fn truncate_body_truncates_large_text() {
        let big = "b".repeat(4096);
        let truncated = truncate_body(big.as_str());
        assert!(truncated.len() < big.len());
        assert!(truncated.ends_with("…(truncated)"));
    }
}
