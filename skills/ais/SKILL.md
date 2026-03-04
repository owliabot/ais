---
name: ais
description: Install and use AIS (Agent Interaction Spec) for DeFi protocol integration. Use when building AIS-compatible agents, parsing or validating AIS documents (protocol, pack, workflow, plan), using the `ais` CLI, integrating with on-chain protocols via AIS, or building AIS components from source.
---

# AIS

Use this skill to install, validate, and integrate AIS artifacts for agent-driven DeFi workflows.

## Quick Start

Install from npm:

```bash
npm install @owliabot/ais-ts-sdk
```

Install globally to get the `ais` CLI on PATH:

```bash
npm install -g @owliabot/ais-ts-sdk
```

Build SDK from source:

```bash
cd /home/ocbot/.openclaw/workspace/repos/ais/ts-sdk
npm install
npm run build
```

## CLI

Run AIS checks with the `ais` binary:

```bash
ais validate <file-or-dir>
ais lint <file-or-dir>
ais check <path>
ais version
```

Common examples:

```bash
ais validate ./protocols/
ais lint ./specs/
ais check . --recursive
ais check . --quiet
ais validate ./specs/ --json
```

## TypeScript SDK

Core parse and validation calls:

```typescript
import {
  parseAIS,
  parseProtocolSpec,
  parsePack,
  parseWorkflow,
  validate,
  createContext,
  registerProtocol,
  resolveAction,
} from '@owliabot/ais-ts-sdk';

const doc = parseAIS(yamlText);
const protocol = parseProtocolSpec(protocolYaml);
const pack = parsePack(packYaml);
const workflow = parseWorkflow(workflowYaml);

const result = validate(yamlText);

const ctx = createContext();
registerProtocol(ctx, protocol);
const action = resolveAction(ctx, 'uniswap-v3/swap_exact_in');
```

For full exported API coverage (resolution helpers, loaders, execution builder, CEL evaluator, schemas), read [references/ts-sdk-api.md](references/ts-sdk-api.md).

## Rust Workspace

Workspace location:

`/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/`

Build all crates:

```bash
cd /home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs
cargo build --workspace
```

Run all tests:

```bash
cargo test --workspace
```

Key crates and roles:
- `ais-sdk`: parsing, typed documents, resolver, planning compile paths.
- `ais-core`: shared issues, field paths, stable hashing, runtime patch primitives.
- `ais-schema`: embedded JSON schemas + validation adapter.
- `ais-engine`: execution loop, event/command/checkpoint contracts, policy gate.
- `ais-llm`: provider-agnostic LLM tool-calling boundary.
- `ais-cel`: CEL lexer/parser/evaluator and numeric model.
- Executors: `ais-evm-executor`, `ais-solana-executor`, `ais-offchain-executor`.
- `ais-runner`: CLI and orchestrated workflow/plan/intent execution.

Detailed crate summaries are in [references/rust-crates.md](references/rust-crates.md).

## AIS Document Structure

AIS commonly uses six document families:
- `protocol`: protocol interfaces (actions, queries, execution mappings)
- `pack`: protocol bundle + policy/constraints
- `workflow`: DAG orchestration across protocol actions/queries
- `plan`: compiled execution contract for runners/engines
- `plan-sketch`: LLM-facing segmented planning IR
- `catalog`: index cards for discovery/search over actions, queries, and packs

See [references/ais-document-types.md](references/ais-document-types.md) for field-level summaries and schema IDs.

## References

- TypeScript SDK API: [references/ts-sdk-api.md](references/ts-sdk-api.md)
- AIS document types: [references/ais-document-types.md](references/ais-document-types.md)
- Rust crates overview: [references/rust-crates.md](references/rust-crates.md)
