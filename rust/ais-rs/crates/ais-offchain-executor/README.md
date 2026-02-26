# `ais-offchain-executor`

Offchain execution plugin crate for HTTP-based protocol queries.

## Responsibility

- Provide plugin execution handler for `execution.type = offchain_apy_query`
- Enforce endpoint domain allowlist before network IO
- Execute HTTP GET/POST with timeout + bounded retry
- Return normalized `outputs` payload for AIS query node projection
- Map response `outputs.side_effects[]` into canonical `ExecutorOutput.side_effects` records when provided
- Normalize response-carried side-effect status values to canonical lifecycle enum (`prepared|sent|confirmed|reverted|unknown`)
- Provide executor-side side-effect reconcile hook for `offchain_apy_query` (`sent` records are currently marked as reconcile-not-supported)

## Public entry points

- `OffchainApyExecutor`
- `OffchainApyExecutorConfig`
- `OffchainApyHttpRequest`
- `OffchainHttpClient`
- `ReqwestOffchainHttpClient`

## Dependencies

- `ais-engine`: `Executor` trait boundary and `ExecutorOutput`
- `reqwest` (blocking + rustls): HTTP client transport
- `url`: endpoint host parsing for allowlist enforcement
- `serde` / `serde_json`: config and payload serialization

## Current status

- Implemented:
  - `AISRS-PLUG-002` minimal executor path for `offchain_apy_query`
  - exact/wildcard domain allowlist enforcement (`api.example.com`, `*.trusted.org`)
  - retry loop with deterministic max-attempt behavior
  - response-carried side-effects (`outputs.side_effects[]`) are normalized to engine side-effect records at executor boundary
  - executor exposes reconcile hook for offchain side-effects; current behavior is conservative (`sent` remains pending with `side_effect_reconcile_not_supported`)
  - unit tests for allowlist rejection, wildcard allow, retry-success flow, and `outputs.side_effects[]` 映射（含 tx_hash 衍生 key / 自定义 key）
- Known gaps:
  - no circuit breaker / jittered backoff
  - no auth/header policy model yet
  - no dedicated response schema binding beyond generic `outputs` normalization
