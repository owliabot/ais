# `tools/ais-runner` (Internal AIS Runner)

Internal CLI tool for verifying `ts-sdk` end-to-end:
- load an AIS workspace (protocols/packs/workflows)
- build an `ExecutionPlan` from a workflow, action, or query
- run the engine loop (concurrency, checkpoint/resume, trace)
- (optionally) broadcast transactions and wait for success

This runner is for internal SDK validation. It is not meant to be a production wallet/app.

## Requirements

- Node.js `>=18`
- This repo checked out with `ts-sdk/` built (`ts-sdk/dist` exists)

## Install / Build

From repo root:

```bash
npm -C tools/ais-runner install
npm -C tools/ais-runner run -s build
```

Run help:

```bash
node tools/ais-runner/dist/main.js --help
```

Run tests:

```bash
npm -C tools/ais-runner test
```

## Quickstart (No Network, Dry-Run)

Dry-run compiles nodes and prints readiness/missing refs without doing any RPC calls.

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --dry-run
```

What you should see:
- `== dry-run (compile only) ==`
- a plan summary with node order
- per-node `state=blocked` with missing refs (until you provide inputs/ctx)

## Runner Config (YAML)

Execution mode (non-`--dry-run`) requires `--config <yaml>`.

Minimal EVM config template:

```yaml
schema: "ais-runner/0.0.1"

engine:
  max_concurrency: 8
  per_chain:
    "eip155:1": { max_read_concurrency: 8, max_write_concurrency: 1 }

chains:
  "eip155:1":
    rpc_url: "${EVM_RPC_MAINNET}"
    wait_for_receipt: true
    receipt_poll: { interval_ms: 1000, max_attempts: 120 }
    signer:
      type: "evm_private_key"
      private_key_env: "EVM_PRIVATE_KEY"

runtime:
  ctx:
    # optional defaults (CLI --ctx overrides this)
    # wallet_address: "0x..."
    capabilities: []
```

Minimal Solana config template:

```yaml
schema: "ais-runner/0.0.1"

engine:
  per_chain:
    "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp": { max_read_concurrency: 16, max_write_concurrency: 1 }

chains:
  "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp":
    rpc_url: "${SOLANA_RPC_MAINNET}"
    commitment: "confirmed"
    wait_for_confirmation: true
    signer:
      type: "solana_keypair_file"
      keypair_path: "~/.config/solana/id.json"
```

Notes:
- `chains` keys must be CAIP-2 chain IDs and must exactly match workflow node `chain` values (e.g. `eip155:8453`).
- `${ENV}` placeholders are expanded at load time.
- Config is validated (fails fast with pinpointed paths).

## Modes

### 1) Run a workflow

Dry-run:

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/swap-to-token.ais-flow.yaml \
  --workspace . \
  --inputs '{"slippage_bps":"50"}' \
  --dry-run
```

Execution (RPC required):

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --inputs '{"evm_amount":"1000000","solana_address":"BPFLoaderUpgradeab1e11111111111111111111111"}' \
  --ctx '{"wallet_address":"0x2222222222222222222222222222222222222222"}'
```

Note:
- `examples/swap-to-token.ais-flow.yaml` references `examples/safe-defi-pack.ais-pack.yaml`, which currently scopes Uniswap V3 to `eip155:8453`. Running it on `eip155:1` will fail workspace validation unless you adjust the workflow/pack.

### 2) Run an action (synthetic workflow)

Action mode always creates a 1-node synthetic workflow and runs it via the same engine.

```bash
node tools/ais-runner/dist/main.js run action \
  --workspace . \
  --ref uniswap-v3@0.0.2/swap-exact-in \
  --chain eip155:1 \
  --args '{"token_in":{"chain_id":"eip155:1","symbol":"WETH","address":"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2","decimals":18},"token_out":{"chain_id":"eip155:1","symbol":"USDC","address":"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48","decimals":6},"amount_in":"0.01","slippage_bps":"50"}' \
  --dry-run
```

Note:
- Action/query modes currently require explicit `--chain`.

### 3) Run a query (synthetic workflow)

```bash
node tools/ais-runner/dist/main.js run query \
  --workspace . \
  --ref erc20@0.0.2/balance-of \
  --chain eip155:1 \
  --args '{"token":{"chain_id":"eip155:1","symbol":"WETH","address":"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2","decimals":18},"owner":"0x1111111111111111111111111111111111111111"}' \
  --dry-run
```

Polling options for read nodes:
- `--until <cel>`: stop when condition becomes true
- `--retry <json>`: retry policy like `{"interval_ms":1000,"max_attempts":60}`
- `--timeout-ms <n>`

## Checkpoint / Resume

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --checkpoint /tmp/ais-checkpoint.json

node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --checkpoint /tmp/ais-checkpoint.json \
  --resume
```

## Trace (JSONL)

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --trace /tmp/ais-trace.jsonl
```

## Outputs File

If a workflow completes without pausing/error, the runner evaluates `workflow.outputs` and prints them.
You can also write them to JSON:

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --out /tmp/workflow-outputs.json
```

## Safety Flags

The runner is intentionally conservative:

- `--dry-run`
  - compiles/prints only, no RPC
  - does not require `--config`
- `--broadcast`
  - required to execute any write node (EVM `evm_call`, Solana `solana_instruction`)
  - without it, write nodes pause with `need_user_confirm` (and include a compiled tx preview when possible)
- `--yes`
  - auto-approves pack policy gates (risk-level approvals)
  - does not bypass `--broadcast`

## Policy Gate Fixture (No Real RPC Needed)

These files exist to validate safety gates deterministically:
- `tools/ais-runner/fixtures/policy-gate-pack.ais-pack.yaml`
- `tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml`
- `tools/ais-runner/fixtures/policy-gate.config.yaml`

Run from repo root:

```bash
# 1) default safety: broadcast gate triggers
node tools/ais-runner/dist/main.js run workflow \
  --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml \
  --workspace . \
  --config tools/ais-runner/fixtures/policy-gate.config.yaml

# 2) broadcast enabled: policy gate triggers (requires approval)
node tools/ais-runner/dist/main.js run workflow \
  --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml \
  --workspace . \
  --config tools/ais-runner/fixtures/policy-gate.config.yaml \
  --broadcast

# 3) broadcast + auto-approve: policy gate does not pause; subsequent failure may occur if localhost RPC is not running
node tools/ais-runner/dist/main.js run workflow \
  --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml \
  --workspace . \
  --config tools/ais-runner/fixtures/policy-gate.config.yaml \
  --broadcast --yes
```

## Troubleshooting

- `workspace_errors` / `workflow_errors`
  - workspace validation failed; fix spec references (protocol@version, pack requires_pack, etc.)
- `Missing --config for execution mode`
  - you ran without `--dry-run` but didn’t pass `--config`
- `Missing signer config for broadcast on chains: ...`
  - you passed `--broadcast` but some chains with write nodes have no `chains[chain].signer` config
- `event: node_blocked missing=[...]`
  - runtime is missing required refs (usually `inputs.*`, `ctx.*`, or auto-filled `contracts.*`)
- Workflows using `{ detect: ... }` may still block on detect
  - dynamic detect provider IO is tracked separately (see `RUN-019` in `docs/TODO-internal-runner-main.md`)
