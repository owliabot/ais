# `tools/ais-runner` (内部 AIS Runner)

这是一个用于**内部验证** `ts-sdk` 端到端能力的 CLI 工具：
- 加载 AIS workspace（protocols / packs / workflows）
- 从 workflow / action / query 构造 `ExecutionPlan`
- 运行 engine（并发调度、checkpoint/resume、trace）
- （可选）广播交易并等待成功

Runner 的定位是 SDK 自测/验收工具，不是生产钱包或面向用户的应用。

## 环境要求

- Node.js `>=18`
- 本仓库已包含 `ts-sdk/dist`（即 `ts-sdk` 已 build）

## 安装与构建

在仓库根目录执行：

```bash
npm -C tools/ais-runner install
npm -C tools/ais-runner run -s build
```

查看帮助：

```bash
node tools/ais-runner/dist/main.js --help
```

运行单测：

```bash
npm -C tools/ais-runner test
```

## 最快上手（无网络，Dry-Run）

`--dry-run` 模式只做：
- workspace 加载与校验
- plan 构建与打印
- readiness/缺失引用检查
- execution 编译预览（不会做 RPC，也不会发送交易）

示例：

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --dry-run
```

你会看到：
- `== dry-run (compile only) ==`
- plan 的节点顺序与依赖
- 每个节点的 `state=blocked` / `missing_refs=...`（直到你提供 inputs/ctx/合约地址等）

## Runner 配置文件（YAML）

非 `--dry-run` 的执行模式必须提供 `--config <yaml>`。

### EVM 最小配置模板

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
    # 可选默认值（CLI --ctx 会覆盖）
    # wallet_address: "0x..."
    capabilities: []
```

### Solana 最小配置模板

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

### 关键注意事项

- `chains` 的 key 必须是 CAIP-2 chain id，并且需要与 workflow node 的 `chain` **精确一致**（例如 `eip155:1` / `eip155:8453`）。
- `${ENV}` 占位符会在读取配置时展开。
- 配置会进行校验（失败会给出带路径的报错，例如 `chains."eip155:1".rpc_url`）。

## 三种运行模式

### 1) 运行 workflow

Dry-run：

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --dry-run
```

执行（需要 RPC；默认不会广播交易）：

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --inputs '{"evm_amount":"1000000","solana_address":"BPFLoaderUpgradeab1e11111111111111111111111"}' \
  --ctx '{"wallet_address":"0x2222222222222222222222222222222222222222"}'
```

说明：
- 默认不广播写交易；如需真正上链请看下文 `--broadcast`。

### 2) 运行 action（合成 workflow）

action 模式会合成一个只包含单个 action 的 workflow，然后走同一套 engine。

```bash
node tools/ais-runner/dist/main.js run action \
  --workspace . \
  --ref uniswap-v3@0.0.2/swap-exact-in \
  --chain eip155:1 \
  --args '{"token_in":{"chain_id":"eip155:1","symbol":"WETH","address":"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2","decimals":18},"token_out":{"chain_id":"eip155:1","symbol":"USDC","address":"0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48","decimals":6},"amount_in":"0.01","slippage_bps":"50"}' \
  --dry-run
```

注意：
- action/query 模式目前要求显式传 `--chain`。

### 3) 运行 query（合成 workflow）

```bash
node tools/ais-runner/dist/main.js run query \
  --workspace . \
  --ref erc20@0.0.2/balance-of \
  --chain eip155:1 \
  --args '{"token":{"chain_id":"eip155:1","symbol":"WETH","address":"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2","decimals":18},"owner":"0x1111111111111111111111111111111111111111"}' \
  --dry-run
```

轮询参数（仅 read 节点支持）：
- `--until <cel>`：CEL 条件为 true 才结束
- `--retry <json>`：例如 `{"interval_ms":1000,"max_attempts":60}`
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

## Trace（JSONL）

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --trace /tmp/ais-trace.jsonl
```

## 输出写文件（workflow outputs）

workflow 正常完成（未 pause/未 error）时，runner 会计算 `workflow.outputs` 并打印。
也可以写到 JSON：

```bash
node tools/ais-runner/dist/main.js run workflow \
  --file examples/bridge-send-wait-deposit.ais-flow.yaml \
  --workspace . \
  --config /path/to/ais-runner.config.yaml \
  --out /tmp/workflow-outputs.json
```

## 安全开关说明

Runner 默认非常保守：

- `--dry-run`
  - 只编译与打印，不做 RPC
  - 不需要 `--config`
- `--broadcast`
  - 允许执行 write 节点（EVM `evm_call`、Solana `solana_instruction`）
  - 不带该参数时，write 节点会 `need_user_confirm` 并暂停（并尽量输出已编译的 tx 预览）
- `--yes`
  - 自动通过 pack policy gate（风险等级审批）
  - 不会绕过 `--broadcast`

## Policy Gate 固定夹具（无需真实 RPC）

这些文件用于确定性验证安全/审批 gate：
- `tools/ais-runner/fixtures/policy-gate-pack.ais-pack.yaml`
- `tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml`
- `tools/ais-runner/fixtures/policy-gate.config.yaml`

在仓库根目录执行：

```bash
# 1) 默认安全：broadcast gate 触发
node tools/ais-runner/dist/main.js run workflow \
  --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml \
  --workspace . \
  --config tools/ais-runner/fixtures/policy-gate.config.yaml

# 2) 开启 broadcast：policy gate 触发（需要审批）
node tools/ais-runner/dist/main.js run workflow \
  --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml \
  --workspace . \
  --config tools/ais-runner/fixtures/policy-gate.config.yaml \
  --broadcast

# 3) broadcast + auto-approve：policy gate 不再暂停；后续可能因为 localhost RPC 未启动而失败
node tools/ais-runner/dist/main.js run workflow \
  --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml \
  --workspace . \
  --config tools/ais-runner/fixtures/policy-gate.config.yaml \
  --broadcast --yes
```

## 常见问题排查

- 输出 `workspace_errors` / `workflow_errors`
  - 说明 workspace 或 workflow 校验失败（protocol@version、pack requires_pack、chain_scope 等不一致）
- 报错 `Missing --config for execution mode`
  - 你没有 `--dry-run`，但也没传 `--config`
- 报错 `Missing signer config for broadcast on chains: ...`
  - 你传了 `--broadcast`，但某些包含 write 节点的链未配置 `chains[chain].signer`
- 事件 `event: node_blocked missing=[...]`
  - runtime 缺失引用，常见是 `inputs.*` / `ctx.*` / `contracts.*`
- 使用 `{ detect: ... }` 的 workflow/action 可能仍会被 detect 阻塞
  - 动态 detect provider IO 尚未实现，单独跟踪（见 `docs/TODO-internal-runner-main.md` 的 `RUN-019`）

