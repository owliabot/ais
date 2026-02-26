use crate::{CompleteWithToolsRequest, CompleteWithToolsResponse, LlmProvider, LlmProviderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationMode {
    StickyPrimary,
    RoundRobin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderChainPolicy {
    pub max_retries_per_provider: u8,
    pub rotation_mode: RotationMode,
}

impl Default for ProviderChainPolicy {
    fn default() -> Self {
        Self {
            max_retries_per_provider: 1,
            rotation_mode: RotationMode::StickyPrimary,
        }
    }
}

pub struct ProviderChain {
    providers: Vec<Box<dyn LlmProvider>>,
    labels: Vec<String>,
    policy: ProviderChainPolicy,
    cursor: usize,
}

impl ProviderChain {
    pub fn new(
        providers: Vec<Box<dyn LlmProvider>>,
        labels: Vec<String>,
        policy: ProviderChainPolicy,
    ) -> Result<Self, LlmProviderError> {
        if providers.is_empty() {
            return Err(LlmProviderError::InvalidConfig {
                reason: "provider chain requires at least one provider".to_string(),
            });
        }
        if providers.len() != labels.len() {
            return Err(LlmProviderError::InvalidConfig {
                reason: "provider chain labels count must match providers count".to_string(),
            });
        }
        Ok(Self {
            providers,
            labels,
            policy,
            cursor: 0,
        })
    }
}

impl LlmProvider for ProviderChain {
    fn complete_with_tools(
        &mut self,
        request: CompleteWithToolsRequest,
    ) -> Result<CompleteWithToolsResponse, LlmProviderError> {
        let len = self.providers.len();
        let start = match self.policy.rotation_mode {
            RotationMode::StickyPrimary => 0,
            RotationMode::RoundRobin => self.cursor % len,
        };
        let mut last_error: Option<LlmProviderError> = None;
        let mut trail = Vec::<String>::new();

        for offset in 0..len {
            let index = (start + offset) % len;
            let provider_label = self
                .labels
                .get(index)
                .cloned()
                .unwrap_or_else(|| format!("provider#{index}"));
            let max_attempts = usize::from(self.policy.max_retries_per_provider) + 1;

            for attempt in 0..max_attempts {
                match self.providers[index].complete_with_tools(request.clone()) {
                    Ok(response) => {
                        if self.policy.rotation_mode == RotationMode::RoundRobin {
                            self.cursor = (index + 1) % len;
                        }
                        return Ok(response);
                    }
                    Err(error) => {
                        let retryable = error.is_retryable();
                        let fallbackable = error.is_fallbackable();
                        let error_text = error.to_string();
                        trail.push(format!(
                            "{provider_label}:attempt={} retryable={} fallbackable={} error={error_text}",
                            attempt + 1,
                            retryable,
                            fallbackable
                        ));
                        if retryable && attempt + 1 < max_attempts {
                            continue;
                        }
                        if fallbackable {
                            last_error = Some(error);
                            break;
                        }
                        return Err(error);
                    }
                }
            }
        }

        let summary = if trail.is_empty() {
            "provider chain exhausted with no attempts".to_string()
        } else {
            trail.join(" | ")
        };
        Err(LlmProviderError::ChainExhausted {
            reason: match last_error {
                Some(error) => format!("{summary}; last_error={error}"),
                None => summary,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderChain, ProviderChainPolicy, RotationMode};
    use crate::{CompleteWithToolsResponse, LlmProvider, LlmProviderError, ScriptedLlmProvider};

    fn ok_provider(label: &str) -> ScriptedLlmProvider {
        ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
            assistant_content: Some(label.to_string()),
            tool_calls: vec![],
        })])
    }

    #[test]
    fn chain_falls_back_to_secondary_provider() {
        let primary =
            ScriptedLlmProvider::from_responses(vec![Err(LlmProviderError::HttpStatus {
                status: 429,
                body: "rate_limit".to_string(),
            })]);
        let secondary = ok_provider("secondary-ok");
        let mut chain = ProviderChain::new(
            vec![Box::new(primary), Box::new(secondary)],
            vec!["primary".to_string(), "secondary".to_string()],
            ProviderChainPolicy::default(),
        )
        .expect("chain");

        let response = chain
            .complete_with_tools(crate::CompleteWithToolsRequest {
                messages: vec![],
                tools: vec![],
            })
            .expect("fallback succeeds");
        assert_eq!(response.assistant_content.as_deref(), Some("secondary-ok"));
    }

    #[test]
    fn chain_retries_same_provider_before_fallback() {
        let primary = ScriptedLlmProvider::from_responses(vec![
            Err(LlmProviderError::HttpStatus {
                status: 500,
                body: "transient".to_string(),
            }),
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("primary-after-retry".to_string()),
                tool_calls: vec![],
            }),
        ]);
        let secondary = ok_provider("secondary-unused");
        let mut chain = ProviderChain::new(
            vec![Box::new(primary), Box::new(secondary)],
            vec!["primary".to_string(), "secondary".to_string()],
            ProviderChainPolicy {
                max_retries_per_provider: 1,
                rotation_mode: RotationMode::StickyPrimary,
            },
        )
        .expect("chain");

        let response = chain
            .complete_with_tools(crate::CompleteWithToolsRequest {
                messages: vec![],
                tools: vec![],
            })
            .expect("retry succeeds");
        assert_eq!(
            response.assistant_content.as_deref(),
            Some("primary-after-retry")
        );
    }

    #[test]
    fn round_robin_rotates_start_provider() {
        let first = ok_provider("first");
        let second = ok_provider("second");
        let mut chain = ProviderChain::new(
            vec![Box::new(first), Box::new(second)],
            vec!["first".to_string(), "second".to_string()],
            ProviderChainPolicy {
                max_retries_per_provider: 0,
                rotation_mode: RotationMode::RoundRobin,
            },
        )
        .expect("chain");

        let first_response = chain
            .complete_with_tools(crate::CompleteWithToolsRequest {
                messages: vec![],
                tools: vec![],
            })
            .expect("first response");
        assert_eq!(first_response.assistant_content.as_deref(), Some("first"));

        let second_response = chain
            .complete_with_tools(crate::CompleteWithToolsRequest {
                messages: vec![],
                tools: vec![],
            })
            .expect("second response");
        assert_eq!(second_response.assistant_content.as_deref(), Some("second"));
    }
}
