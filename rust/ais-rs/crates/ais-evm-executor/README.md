# `ais-evm-executor`

EVM executor crate for AIS, backed by the Alloy ecosystem.

## Purpose and boundaries

- Own EVM chain executor capabilities (`evm_read` / `evm_call` / `evm_rpc`) for `eip155:*`.
- Provide provider/signer/redaction modules used by `ais-runner` and `ais-engine` integration.
- Keep network-facing concerns isolated from `ais-engine` core scheduling logic.

## Public entry points

- `provider` module:
  - `EvmRpcEndpoint`
  - `EvmProviderRegistry`
  - `ProviderError`
- `types` module:
  - shared request/response and trait types (`EvmReadRequest`, `EvmRpcRequest`, `EvmCallSendRequest`, sender traits)
  - `EvmCallExecutionConfig`
- `executor` module:
  - `EvmExecutor` (`ais_engine::Executor`)
  - `EvmExecutor::supports(chain, execution_type)`
  - `EvmCallSender` / `AlloyEvmCallSender` (`evm_call` adapter)
  - `EvmReadRpcSender` / `AlloyEvmReadRpcSender` (`evm_read` + `evm_rpc` adapter)
- `signer` module:
  - `EvmTransactionSigner` (signer identity trait for address + private key source)
  - `LocalPrivateKeySigner` (dev local private-key signer)
- `redact` module:
  - `redact_evm_value(payload, mode)` redacts rpc/tx payloads for `default|audit|off`

## Workspace dependencies

- External:
  - `alloy`
  - `alloy-primitives`
  - `alloy-json-abi`
  - `alloy-dyn-abi`
  - `alloy-rpc-types-eth`
  - `k256`
- Internal:
  - `ais-engine` (implements `Executor` trait integration)

## Current status

- Implemented:
  - `AISRS-EVM-001` baseline provider abstraction and chain->RPC registry.
  - `AISRS-EVM-010` supports() + exact chain matching for `eip155:*`.
  - `AISRS-EVM-011` `evm_read` path (`eth_call` + ABI static decode for `address/bool/bytes32/uint*`).
  - ABI handling uses Alloy typed `Function` + `alloy-dyn-abi` (`JsonAbiExt`/`FunctionExt`) for input/output ABI encode/decode, removing local selector/signature assembly and static-word decode path.
  - Input coercion now supports more ABI shapes (dynamic string/bytes, arrays, tuple object/array forms) before passing values into Alloy ABI encoder.
  - `evm_read` result keeps ABI-decoded data under `outputs` (generic per-ABI behavior, no protocol-specific field special-casing).
  - `AISRS-EVM-012` `evm_call` path now uses Alloy provider wallet + recommended fillers flow (nonce/gas/fee/chain-id), optional receipt wait, missing-signer -> need_user_confirm error.
  - `LocalPrivateKeySigner` now focuses on validated local key/address source for Alloy wallet filling/signing flow.
  - `evm_call` supports optional tx override fields: `nonce`, `gas_limit`, `max_fee_per_gas`, `max_priority_fee_per_gas` (defaults are used when omitted).
  - `evm_call` result includes filled tx fields (`nonce/gas_limit/max_fee_per_gas/max_priority_fee_per_gas`) fetched from pending tx for workflow traceability.
  - `AISRS-EVM-013` `evm_rpc` read-only allowlist gate.
  - `evm_rpc` now normalizes object-style params into positional JSON-RPC arrays for allowlisted methods (e.g. `eth_getBalance`) to avoid RPC tuple decode errors, including protocol `params.array` wrapper form.
  - `eth_getBalance` `evm_rpc` result additionally exposes decimal `balance` field (alongside raw `result`) for query assertions using `outputs.balance`.
  - `AISRS-EVM-020` rpc/tx redaction aligned to trace modes.
  - Internal module split: shared executor types moved to `types.rs`, value/param parsing helpers moved to `utils.rs`, ABI encode/decode logic moved to `abi.rs`, and Alloy RPC client pool moved to `client_pool.rs` to keep `executor.rs` focused on execution flow.
  - chain-scoped long-lived Alloy RPC client pool for `evm_read` / `evm_call` / `evm_rpc` (reuses transport/session instead of reconnecting each request).
  - built-in transport URL support extended to `http(s)` and `ws(s)` endpoints.
  - `timeout_ms` is applied to RPC connect/read/call/send/receipt awaits (HTTP + WS) via Tokio timeout guards.
  - Minor cleanup: remove needless borrows in executor dispatch path, keeping clippy output focused on substantive warnings.
- Executor uses Alloy providers for read/call/rpc paths; no crate-local legacy HTTP RPC client/factory layer.
- Executor expects execution payload to be materialized by engine (literal values), and focuses on EVM request building/sign/send/receipt only.
- Planned next:
  - engine/runner wiring for real provider + signer config.
