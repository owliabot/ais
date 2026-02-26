use crate::{LlmProvider, LlmProviderError};

use super::anthropic::AnthropicProvider;
use super::chain::{ProviderChain, ProviderChainPolicy};
use super::openai_compatible::OpenAiCompatibleProvider;
use super::registry::{provider_spec, ProviderBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChainConfig {
    pub providers: Vec<ProviderConfig>,
    pub policy: ProviderChainPolicy,
}

pub fn build_provider(config: ProviderConfig) -> Result<Box<dyn LlmProvider>, LlmProviderError> {
    if config.provider.trim().is_empty() {
        return Err(LlmProviderError::InvalidConfig {
            reason: "provider is required".to_string(),
        });
    }
    if config.model.trim().is_empty() {
        return Err(LlmProviderError::InvalidConfig {
            reason: "model is required".to_string(),
        });
    }
    if config.api_key.trim().is_empty() {
        return Err(LlmProviderError::InvalidConfig {
            reason: "api_key is required".to_string(),
        });
    }

    let spec =
        provider_spec(config.provider.as_str()).ok_or_else(|| LlmProviderError::InvalidConfig {
            reason: format!("unknown provider `{}`", config.provider),
        })?;
    let base_url = config
        .api_base
        .or_else(|| spec.default_base_url.map(str::to_string));
    match spec.backend {
        ProviderBackend::Anthropic => Ok(Box::new(AnthropicProvider::new(
            config.model,
            config.api_key,
            base_url,
        )?)),
        ProviderBackend::OpenAiCompatible => Ok(Box::new(OpenAiCompatibleProvider::new(
            config.model,
            config.api_key,
            base_url,
        )?)),
    }
}

pub fn build_provider_chain(
    config: ProviderChainConfig,
) -> Result<Box<dyn LlmProvider>, LlmProviderError> {
    if config.providers.is_empty() {
        return Err(LlmProviderError::InvalidConfig {
            reason: "provider chain must include at least one provider".to_string(),
        });
    }

    let mut providers = Vec::<Box<dyn LlmProvider>>::with_capacity(config.providers.len());
    let mut labels = Vec::<String>::with_capacity(config.providers.len());
    for provider in config.providers {
        labels.push(format!("{}/{}", provider.provider, provider.model));
        providers.push(build_provider(provider)?);
    }
    let chain = ProviderChain::new(providers, labels, config.policy)?;
    Ok(Box::new(chain))
}

#[cfg(test)]
mod tests {
    use super::{build_provider, build_provider_chain, ProviderChainConfig, ProviderConfig};
    use crate::providers::{ProviderChainPolicy, RotationMode};
    use crate::LlmProviderError;

    #[test]
    fn build_provider_rejects_unknown_provider() {
        let result = build_provider(ProviderConfig {
            provider: "foo".to_string(),
            model: "bar".to_string(),
            api_key: "key".to_string(),
            api_base: None,
        });
        let error = result.err().expect("must reject");
        assert!(matches!(error, LlmProviderError::InvalidConfig { .. }));
    }

    #[test]
    fn build_provider_accepts_openai() {
        let provider = build_provider(ProviderConfig {
            provider: "openai".to_string(),
            model: "gpt-4.1-mini".to_string(),
            api_key: "key".to_string(),
            api_base: None,
        });
        assert!(provider.is_ok());
    }

    #[test]
    fn build_provider_chain_rejects_empty_chain() {
        let result = build_provider_chain(ProviderChainConfig {
            providers: vec![],
            policy: ProviderChainPolicy::default(),
        });
        let error = result.err().expect("must reject");
        assert!(matches!(error, LlmProviderError::InvalidConfig { .. }));
    }

    #[test]
    fn build_provider_chain_accepts_primary_and_fallback() {
        let provider = build_provider_chain(ProviderChainConfig {
            providers: vec![
                ProviderConfig {
                    provider: "openai".to_string(),
                    model: "gpt-4.1-mini".to_string(),
                    api_key: "key".to_string(),
                    api_base: None,
                },
                ProviderConfig {
                    provider: "groq".to_string(),
                    model: "llama-3.3-70b-versatile".to_string(),
                    api_key: "key".to_string(),
                    api_base: None,
                },
            ],
            policy: ProviderChainPolicy {
                max_retries_per_provider: 1,
                rotation_mode: RotationMode::StickyPrimary,
            },
        });
        assert!(provider.is_ok());
    }
}
