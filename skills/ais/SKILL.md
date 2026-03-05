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
ais-runner plan diff --before <a.yaml> --after <b.yaml> [--format text|json]

# Text output example:
# plan diff: added=0 removed=0 changed=1
# changed:
#   - id=swap chain=eip155:1 exec=evm_call changes=execution_type

# JSON output shape: { summary:{added,removed,changed}, added:[], removed:[], changed:[{id,changes[],before,after}] }

# Replay from trace or checkpoint
ais-runner replay --trace-jsonl <file> [--until-node <id>]
ais-runner replay --checkpoint <file> --plan <file> --config <runner.yaml>

# Agent — two distinct modes:

# Intent mode (LLM planning): requires --workspace + llm config
ais-runner agent --intent "swap 1 ETH to USDC" --config <runner.yaml> --workspace <dir> \
  [--pack <pack-file>] [--approvals-mode safe|assist|yolo]
ais-runner agent --intent-file <file.txt> --config <runner.yaml> --workspace <dir>

# Plan execution mode (no planning): executes an existing plan directly
ais-runner agent --plan <file.yaml> --config <runner.yaml> \
  [--approvals-mode safe|assist|yolo]
```

> **Agent modes:**
> - `--intent / --intent-file`: segmented LLM planner -> compiles plan-sketch -> executes. Requires `--workspace` and `llm` in runner config.
> - `--plan`: execution-only. No LLM, no plan-sketch, no workspace needed. Adds a pause/resume decision loop for `need_user_confirm` events (manual/assist/yolo), pack-policy integration via `--pack`, and outputs `ais-runner-agent/0.0.1` schema (includes `iterations`, `llm_usage`).

## Minimal runner config (`runner.yaml`)

Every non-dry-run command requires `--config`. Minimal structure for EVM plan/workflow:

```yaml
schema: "ais-runner/0.0.1"
chains:
  "eip155:1":                         # Ethereum mainnet
    rpc_url: ${EVM_RPC_URL}
    signer:
      type: evm_private_key
      private_key: ${PRIVATE_KEY}     # 0x-prefixed hex key
```

For `ais-runner agent` (intent mode), also add an `llm` block:

```yaml
schema: "ais-runner/0.0.1"
chains:
  "eip155:1":
    rpc_url: ${EVM_RPC_URL}
    signer:
      type: evm_private_key
      private_key: ${PRIVATE_KEY}
llm:
  provider: openai                    # valid: openai, anthropic, openrouter, groq, zhipu, vllm, gemini, ollama, nvidia, deepseek
  model: gpt-4o
  api_key: ${OPENAI_API_KEY}
  api_base: https://api.openai.com/v1 # optional; use for OpenAI-compatible endpoints
```

For Sepolia testnet substitute `eip155:11155111` and a Sepolia RPC URL.

> **`llm.api_key` is required** — no `api_key_env` field exists. Use `${ENV_VAR}` placeholder syntax (missing env vars fail config parse immediately).

## I/O streams (advanced)

Output streams (optional flags on run plan / run workflow / agent):
- `--events-jsonl <path|->` — one `ais-engine-event/0.0.3` JSON object per line; pass `-` to stream to stdout
- `--trace <path>` — redacted event trace, one JSON object per line

Input stream:
- `--commands-stdin-jsonl` — send `ais-engine-command/0.0.1` envelopes via stdin, one per line (**`run plan` and `run workflow` only** — not available for `agent`)

## Approvals

In `safe` mode the runner pauses before risk-level ≥ 3 actions.

**For `run plan` / `run workflow`:** commands are read **once** from stdin before the engine loop starts. Pre-load approval commands via `--commands-stdin-jsonl`:
```json
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-1","type":"user_confirm","data":{"node_id":"n1","decision":"approve"}}}
{"schema":"ais-engine-command/0.0.1","command":{"id":"cmd-2","type":"user_confirm","data":{"node_id":"n2","decision":"deny"}}}
```

> ⚠️ If a node requires approval but no matching command was pre-loaded, the run exits with `status: paused` and `paused_reason: need_user_confirm:<node_id>`. Resume with `--checkpoint` after loading additional commands.

**For `agent`:** approvals are handled interactively or via checkpoint resume. Use `--approvals-mode assist` for semi-automatic or `yolo` to skip all gates (unsafe).

⚠️ `--approvals-mode yolo` skips all approval gates — use only in trusted test environments.

Paused runs can be resumed by re-running `ais-runner agent` with `--checkpoint <file>`:

```bash
# Intent mode resume
ais-runner agent --intent "swap 1 ETH for USDC" --config runner.yaml --workspace ./workspace --checkpoint ./run.ckpt

# Plan mode resume
ais-runner agent --plan plan.yaml --config runner.yaml --checkpoint ./run.ckpt
```

> ⚠️ `replay --checkpoint` is a debug/trace replay path — it does NOT continue agent execution. Always use `agent ... --checkpoint` to resume a paused run.

## AIS document families

AIS documents are strict **JSON or YAML** files. Each must set a versioned `schema` field:

| Family | schema value | Used as input to |
|--------|-------------|-----------------|
| protocol | `ais/0.0.2` | pack includes |
| pack | `ais-pack/0.0.2` | auto-discovered in workspace (schema scan); policy only activates via `--pack <file>` (agent only) |
| workflow | `ais-flow/0.0.3` | `run workflow --workflow` |
| plan | `ais-plan/0.0.3` | `run plan --plan` / `agent --plan` |
| plan-sketch | `ais-plan-sketch/0.1.0` | internal agent planning IR — compiled automatically by `ais-runner agent`; not a direct CLI input |
| catalog | `ais-catalog/0.0.1` | agent index (auto-built from workspace) |

## Workspace layout

`ais-runner agent --workspace <dir>` and `run workflow --workspace <dir>` scan the directory recursively. Files are classified by their `schema:` field:

- **Discovered automatically:** protocol, pack, workflow, plan
- **Ignored during scan:** catalog, plan-sketch (they are never auto-loaded from workspace)
- **Pack policy** is only activated when passed explicitly via `--pack <file>` (agent only — `run workflow` has no `--pack` flag; pack is only used for workspace validation in workflow execution)

Typical structure:
```text
workspace/
  myprotocol.ais.yaml          # protocol (ais/0.0.2)
  mypack.ais-pack.yaml         # pack (ais-pack/0.0.2)  -> agent only: pass via --pack to activate policy
  swap-workflow.ais-flow.yaml  # workflow (ais-flow/0.0.3)
```

Full field reference: [references/ais-document-types.md](references/ais-document-types.md)
Full CLI reference: [references/ais-runner-cli.md](references/ais-runner-cli.md)
