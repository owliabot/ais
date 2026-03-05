---
name: ais
description: Use ais-runner to execute AIS plans, workflows, agent flows, or author AIS YAML documents. Use when: running plans/workflows/replay/agent, validating documents with --dry-run, or authoring protocol/pack/workflow/plan/plan-sketch/catalog files.
---

# AIS

## Preflight

```bash
command -v ais-runner          # verify binary is on PATH
ais-runner --help              # confirm version and commands
```

## Commands

```bash
# Dry-run (no config required)
ais-runner run plan --plan <file.yaml> --dry-run [--format json|text]
ais-runner run workflow --workflow <file.yaml> [--workspace <dir>] --dry-run

# Execute (requires --config)
ais-runner run plan --plan <file.yaml> --config <runner.yaml> [--format json|text]
ais-runner run workflow --workflow <file.yaml> --config <runner.yaml> [--outputs <out.json>]

# Diff two plans
ais-runner plan diff --before <a.yaml> --after <b.yaml>

# Replay from trace or checkpoint
ais-runner replay --trace-jsonl <file> [--until-node <id>]
ais-runner replay --checkpoint <file> --plan <file> --config <runner.yaml>

# Agent — run from natural language intent or plan
ais-runner agent --intent "swap 1 ETH to USDC" --config <runner.yaml> [--workspace <dir>] \
  [--approvals-mode safe|assist|yolo]
ais-runner agent --plan <file.yaml> --config <runner.yaml>
ais-runner agent --intent-file <file.txt> --config <runner.yaml>
```

## Minimal runner config (`runner.yaml`)

Every non-dry-run command requires `--config`. Minimal structure:

```yaml
schema: "ais-runner/0.0.1"
chain_providers:
  "eip155:1":                         # Ethereum mainnet
    type: http
    url: "https://eth.llamarpc.com"   # replace with your RPC endpoint
wallet:
  type: local
  private_key_env: "PRIVATE_KEY"     # export PRIVATE_KEY=0x...
```

For Sepolia testnet substitute `eip155:11155111` and a Sepolia RPC URL. Solana uses `solana:<genesis-hash>`.

## I/O streams (advanced)

Output streams (optional flags on run plan / run workflow / agent):
- `--events-jsonl <path|->` — one `ais-engine-event/0.0.3` JSON object per line; pass `-` to stream to stdout
- `--trace <path>` — redacted event trace, one JSON object per line

Input stream:
- `--commands-stdin-jsonl` — send `ais-engine-command/0.0.1` envelopes via stdin, one per line (**`run plan` and `run workflow` only** — not available for `agent`)

## Approvals

In `safe` mode the runner pauses before risk-level ≥ 3 actions.

**For `run plan` / `run workflow`:** pipe approval commands via `--commands-stdin-jsonl`:
```json
{"kind":"approve","node_id":"n1"}
{"kind":"reject","node_id":"n2","reason":"Too much slippage"}
```

**For `agent`:** approvals are handled interactively or via checkpoint resume. Use `--approvals-mode assist` for semi-automatic or `yolo` to skip all gates (unsafe).

⚠️ `--approvals-mode yolo` skips all approval gates — use only in trusted test environments.

Paused runs can be resumed from checkpoint (`--checkpoint`).

## AIS document families

AIS documents are strict **JSON or YAML** files. Each must set a versioned `schema` field:

| Family | schema value | Used as input to |
|--------|-------------|-----------------|
| protocol | `ais/0.0.2` | pack includes |
| pack | `ais-pack/0.0.2` | `--pack` flag / workspace |
| workflow | `ais-flow/0.0.3` | `run workflow --workflow` |
| plan | `ais-plan/0.0.3` | `run plan --plan` / `agent --plan` |
| plan-sketch | `ais-plan-sketch/0.1.0` | internal agent planning IR — compiled automatically by `ais-runner agent`; not a direct CLI input |
| catalog | `ais-catalog/0.0.1` | agent index (auto-built from workspace) |

Full field reference: [references/ais-document-types.md](references/ais-document-types.md)
Full CLI reference: [references/ais-runner-cli.md](references/ais-runner-cli.md)
