use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod providers;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: MessageRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompleteWithToolsRequest {
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompleteWithToolsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmProviderError {
    #[error("llm provider call failed: {reason}")]
    CallFailed { reason: String },
    #[error("llm provider config invalid: {reason}")]
    InvalidConfig { reason: String },
    #[error("llm provider http status {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("llm provider decode failed: {reason}")]
    DecodeFailed { reason: String },
    #[error("llm provider exhausted provider chain: {reason}")]
    ChainExhausted { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorClass {
    Retryable,
    Fallbackable,
    Fatal,
}

impl LlmProviderError {
    pub fn classify(&self) -> LlmErrorClass {
        match self {
            LlmProviderError::InvalidConfig { .. } => LlmErrorClass::Fatal,
            LlmProviderError::ChainExhausted { .. } => LlmErrorClass::Fatal,
            LlmProviderError::DecodeFailed { .. } => LlmErrorClass::Fallbackable,
            LlmProviderError::HttpStatus { status, .. } => {
                if *status == 408 || *status == 429 || *status >= 500 {
                    LlmErrorClass::Retryable
                } else {
                    LlmErrorClass::Fallbackable
                }
            }
            LlmProviderError::CallFailed { reason } => {
                let reason_lower = reason.to_lowercase();
                if reason_lower.contains("timeout")
                    || reason_lower.contains("timed out")
                    || reason_lower.contains("connection")
                    || reason_lower.contains("eof")
                    || reason_lower.contains("broken pipe")
                    || reason_lower.contains("temporar")
                    || reason_lower.contains("429")
                    || reason_lower.contains("5xx")
                {
                    LlmErrorClass::Retryable
                } else {
                    LlmErrorClass::Fallbackable
                }
            }
        }
    }

    pub fn is_retryable(&self) -> bool {
        self.classify() == LlmErrorClass::Retryable
    }

    pub fn is_fallbackable(&self) -> bool {
        matches!(
            self.classify(),
            LlmErrorClass::Retryable | LlmErrorClass::Fallbackable
        )
    }
}

pub trait LlmProvider {
    fn complete_with_tools(
        &mut self,
        request: CompleteWithToolsRequest,
    ) -> Result<CompleteWithToolsResponse, LlmProviderError>;
}

impl<T> LlmProvider for Box<T>
where
    T: LlmProvider + ?Sized,
{
    fn complete_with_tools(
        &mut self,
        request: CompleteWithToolsRequest,
    ) -> Result<CompleteWithToolsResponse, LlmProviderError> {
        (**self).complete_with_tools(request)
    }
}

#[derive(Debug, Clone)]
pub struct ScriptedLlmProvider {
    responses: std::collections::VecDeque<Result<CompleteWithToolsResponse, LlmProviderError>>,
}

impl ScriptedLlmProvider {
    pub fn from_responses(
        responses: impl IntoIterator<Item = Result<CompleteWithToolsResponse, LlmProviderError>>,
    ) -> Self {
        Self {
            responses: responses.into_iter().collect(),
        }
    }
}

impl LlmProvider for ScriptedLlmProvider {
    fn complete_with_tools(
        &mut self,
        _request: CompleteWithToolsRequest,
    ) -> Result<CompleteWithToolsResponse, LlmProviderError> {
        self.responses.pop_front().unwrap_or_else(|| {
            Err(LlmProviderError::CallFailed {
                reason: "no scripted response available".to_string(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scripted_provider_returns_responses_in_order() {
        let mut provider = ScriptedLlmProvider::from_responses(vec![
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("first".to_string()),
                tool_calls: vec![],
            }),
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("second".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-1".to_string(),
                    name: "confirm".to_string(),
                    arguments: json!({"node_id":"n1"}),
                }],
            }),
        ]);

        let request = CompleteWithToolsRequest {
            messages: vec![],
            tools: vec![],
        };
        let first = provider
            .complete_with_tools(request.clone())
            .expect("first call");
        assert_eq!(first.assistant_content.as_deref(), Some("first"));
        let second = provider.complete_with_tools(request).expect("second call");
        assert_eq!(second.tool_calls.len(), 1);
        assert_eq!(second.tool_calls[0].name, "confirm");
    }

    #[test]
    fn provider_error_classification_is_stable() {
        let retryable = LlmProviderError::HttpStatus {
            status: 429,
            body: "rate_limit".to_string(),
        };
        assert_eq!(retryable.classify(), LlmErrorClass::Retryable);
        assert!(retryable.is_fallbackable());

        let fallbackable = LlmProviderError::DecodeFailed {
            reason: "invalid json".to_string(),
        };
        assert_eq!(fallbackable.classify(), LlmErrorClass::Fallbackable);
        assert!(!fallbackable.is_retryable());

        let fatal = LlmProviderError::InvalidConfig {
            reason: "missing key".to_string(),
        };
        assert_eq!(fatal.classify(), LlmErrorClass::Fatal);
        assert!(!fatal.is_fallbackable());
    }
}
