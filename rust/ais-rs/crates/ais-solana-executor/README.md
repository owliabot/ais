# `ais-solana-executor`

Solana executor crate for AIS.

## Purpose and boundaries

- Own Solana chain executor behavior for `solana_read` and `solana_instruction`.
- Provide RPC endpoint/client abstraction for runner-driven chain config.
- Keep network-facing Solana logic out of `ais-engine` core scheduler.

## Public entry points

- `types` module:
  - `SolanaRpcEndpoint`
  - `SolanaProviderRegistry`
  - `SolanaRpcClient` / `SolanaRpcClientFactory`
  - `SolanaInstructionRequest`
  - `ProviderError`
- `executor` module:
  - `SolanaExecutor` (`ais_engine::Executor`)
  - `SolanaExecutor::supports(chain, execution_type)`
  - `solana_instruction` path requires configured signer, otherwise returns `need_user_confirm`
- `signer` module:
  - `SolanaTransactionSigner` (pluggable signer trait)
  - `LocalPrivateKeySigner` (dev local signer)
- `redact` module:
  - `redact_solana_value(payload, mode)` for `default|audit|off`

## Workspace dependencies

- Internal:
  - `ais-engine` (`Executor` trait integration)
- External:
  - `serde`, `serde_json`, `thiserror`

## Current status

- Implemented:
  - `AISRS-SOL-001` minimal RPC provider abstraction + chain config model.
  - `AISRS-SOL-010` supports() + `solana_read` + `solana_instruction` execution entry.
  - `AISRS-SOL-011` instruction signing/send/confirm flow (missing-signer -> need_user_confirm; v0 + lookup table enforcement).
  - `AISRS-SOL-020` solana payload redaction aligned with trace modes.
- Planned next:
  - runner/engine wiring with real solana client + signer config.
