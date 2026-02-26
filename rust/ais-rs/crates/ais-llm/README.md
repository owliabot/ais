# `ais-llm`

Typed LLM provider boundary for AIS agent tool-calling.

## Purpose and boundaries

- Define provider-agnostic request/response types for chat + tool calls.
- Expose a small `LlmProvider` trait for runner/agent integration.
- Provide a deterministic scripted provider for tests.
- Provide minimal production-ready provider adapters and a provider factory.

## Public API entry points

- `LlmProvider`
- `LlmProviderError`
- `MessageRole`
- `LlmMessage`
- `ToolSpec`
- `ToolCall`
- `CompleteWithToolsRequest`
- `CompleteWithToolsResponse`
- `ScriptedLlmProvider`
- `providers::ProviderConfig`
- `providers::ProviderChainConfig`
- `providers::ProviderChainPolicy`
- `providers::RotationMode`
- `providers::build_provider`
- `providers::build_provider_chain`
- `providers::OpenAiCompatibleProvider`
- `providers::AnthropicProvider`
- `providers::PROVIDER_REGISTRY`

## Dependencies on other workspace crates

- None.

## Current implementation status and gaps

- Implemented:
  - sync provider trait and typed tool-calling model
  - scripted provider for unit/integration testing
  - provider registry + factory
  - OpenAI-compatible adapter (`openai/openrouter/groq/zhipu/vllm/gemini/ollama/nvidia/deepseek`)
  - Anthropic adapter (`/v1/messages`)
  - provider chain orchestration:
    - unified error classification (`retryable|fallbackable|fatal`)
    - per-provider retry budget
    - multi-provider fallback
    - optional round-robin rotation across providers
  - resilient HTTP handling:
    - enable response decompression (gzip/deflate/brotli) while keeping `reqwest default-features = false`
    - include endpoint + status + selected headers + truncated body in provider errors (helps debug 401/403/429/HTML error pages)
    - retries are now handled by chain policy rather than individual provider adapters
- Known gaps:
  - no streaming/async provider API yet
  - advanced request tuning (timeouts/retries/proxy/custom headers) is minimal
