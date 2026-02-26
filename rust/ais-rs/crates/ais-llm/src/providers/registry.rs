#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderBackend {
    OpenAiCompatible,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSpec {
    pub name: &'static str,
    pub backend: ProviderBackend,
    pub default_base_url: Option<&'static str>,
}

pub const PROVIDER_REGISTRY: &[ProviderSpec] = &[
    ProviderSpec {
        name: "anthropic",
        backend: ProviderBackend::Anthropic,
        default_base_url: None,
    },
    ProviderSpec {
        name: "openai",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: None,
    },
    ProviderSpec {
        name: "openrouter",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("https://openrouter.ai/api/v1"),
    },
    ProviderSpec {
        name: "groq",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("https://api.groq.com/openai/v1"),
    },
    ProviderSpec {
        name: "zhipu",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("https://open.bigmodel.cn/api/paas/v4"),
    },
    ProviderSpec {
        name: "vllm",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("http://localhost:8000/v1"),
    },
    ProviderSpec {
        name: "gemini",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
    },
    ProviderSpec {
        name: "ollama",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("http://localhost:11434/v1"),
    },
    ProviderSpec {
        name: "nvidia",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("https://integrate.api.nvidia.com/v1"),
    },
    ProviderSpec {
        name: "deepseek",
        backend: ProviderBackend::OpenAiCompatible,
        default_base_url: Some("https://api.deepseek.com/v1"),
    },
];

pub fn provider_spec(name: &str) -> Option<ProviderSpec> {
    PROVIDER_REGISTRY
        .iter()
        .find(|spec| spec.name == name)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::{provider_spec, ProviderBackend};

    #[test]
    fn registry_contains_expected_backends() {
        let anthropic = provider_spec("anthropic").expect("anthropic exists");
        assert_eq!(anthropic.backend, ProviderBackend::Anthropic);
        let openrouter = provider_spec("openrouter").expect("openrouter exists");
        assert_eq!(openrouter.backend, ProviderBackend::OpenAiCompatible);
        assert!(provider_spec("unknown").is_none());
    }
}
