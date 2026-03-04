---
name: ais
description: Use AIS through the Rust toolchain and `ais-runner`. Use when building AIS Rust crates, running plans/workflows/replay/agent flows, or authoring AIS YAML documents (protocol/pack/workflow/plan/plan-sketch/catalog).
---

# AIS (Rust-only)

Use this skill for AIS work in the Rust workspace.

Rust workspace:
`/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/`

## Build from source

Prerequisite: Rust toolchain via `rustup`.

```bash
cd /home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs
cargo build --release -p ais-runner
cargo install --path crates/ais-runner
cargo test --workspace
```

## Main CLI: `ais-runner`

Primary command families:

```bash
ais-runner run plan --plan <file> [--config <file>] [--dry-run] [--format json|text]
ais-runner run workflow --workflow <file> [--workspace <dir>] [--config <file>] [--dry-run] [--outputs <json-file>]
ais-runner plan diff --before <plan> --after <plan>
ais-runner replay [--trace-jsonl <file> | --checkpoint <file> --plan <file> --config <file>] [--until-node <id>]
ais-runner agent --intent "<text>" --config <runner-config> [--workspace <dir>] [--approvals-mode safe|assist|yolo]
```

Output and stream contracts:
- `--format text` (default) or `--format json`
- JSONL boundaries:
- `--events-jsonl <path|->`
- `--trace <path>`
- `--commands-stdin-jsonl`

Read full command/flag/output/error details in [references/ais-runner-cli.md](references/ais-runner-cli.md).

## AIS YAML document types

AIS inputs are YAML-authored documents with a required `schema:` field identifying the contract type:
- `protocol`
- `pack`
- `workflow`
- `plan`
- `plan-sketch`
- `catalog`

Read schema IDs, key fields, and examples in [references/ais-document-types.md](references/ais-document-types.md).

## Rust crates for custom tooling

Core crates:
- `ais-runner`
- `ais-sdk`
- `ais-engine`
- `ais-core`
- `ais-schema`
- `ais-cel`
- `ais-llm`

Executors:
- `ais-evm-executor`
- `ais-solana-executor`
- `ais-offchain-executor`

Read responsibilities, entry points, and status in [references/rust-crates.md](references/rust-crates.md).

## References

- CLI reference: [references/ais-runner-cli.md](references/ais-runner-cli.md)
- Document contracts: [references/ais-document-types.md](references/ais-document-types.md)
- Rust crate map: [references/rust-crates.md](references/rust-crates.md)
