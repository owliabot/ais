# AIS Rust Crates

This file documents which crates power which `ais-runner` CLI behaviors. Useful if you need to understand what happens internally when commands run or if you have source access.

| CLI command / flag | Crate(s) responsible |
|--------------------|----------------------|
| `run plan` (parse + validate) | `ais-sdk`, `ais-schema` |
| `run plan` (execute) | `ais-engine`, `ais-runner` |
| `run workflow` (compile → plan) | `ais-sdk` |
| `run workflow` (execute) | `ais-engine`, `ais-runner` |
| `plan diff` | `ais-runner` (CLI dispatch), `ais-engine` (diff logic) |
| `replay` | `ais-runner` (CLI dispatch), `ais-engine` (replay logic) |
| `agent` (LLM planning) | `ais-llm`, `ais-sdk` |
| `agent` (execution) | `ais-engine`, `ais-runner` |
| CEL expression evaluation | `ais-cel` |
| Schema validation | `ais-schema` |
| EVM actions (`evm_call`, `evm_read`) | `ais-evm-executor` |
| Solana actions (`solana_instruction`) | `ais-solana-executor` |
| Offchain queries (`offchain_apy_query`) | `ais-offchain-executor` |
| Shared primitives, hashing | `ais-core` |
