# Runner local fixture: intent native + ERC20 transfer

`AISINT-TEST-004` / `AISINT-DOC-002` 端到端 fixture：
- 输入自然语言 intent
- LLM tool-calling 产出 plan
- Runner 加载 workspace/pack/config 后执行
- 在 `need_user_confirm` 下支持人工确认或 yolo 自动确认
- 支持“执行失败 -> 自动 revise_plan -> replace_plan -> 继续执行”演示

## Layout

- `intent/intent.txt`: 示例意图文本
- `workspace/evm-native-utils.ais.yaml`
- `workspace/erc20.ais.yaml`
- `workspace/safe-defi.ais-pack.yaml`
- `config/runner.local.yaml`: 本地链运行配置（含 demo 私钥）
- `runtime/runtime.local.json`: 示例输入参数
- `plan/intent-native-erc20.plan.json`: 目标计划（供审阅/回归）
- `llm/intent-native-erc20.success.jsonl`: scripted LLM 响应（`propose_plan`）
- `llm/intent-native-erc20.repair.jsonl`: scripted LLM 响应（`propose_plan` 失败后 `revise_plan` 修复）

## Prerequisites

- 在本机启动 `anvil`：`http://127.0.0.1:8545`
- `runtime.runtime.local.json` 中的 ERC20 地址应为本地已部署 token
- 当前 demo 私钥仅用于本地演示，勿用于真实资金

## Scenario A: 成功路径（YOLO 自动确认）

在 `rust/ais-rs` 下运行：

```bash
cargo run -p ais-runner -- agent \
  --intent-file fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --runtime fixtures/runner-local/intent-native-erc20-transfer/runtime/runtime.local.json \
  --profile demo-scripted \
  --llm-script-jsonl fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.success.jsonl \
  --approvals-mode yolo \
  --format json
```

## Scenario B: 手动确认路径（safe）

```bash
cargo run -p ais-runner -- agent \
  --intent-file fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --runtime fixtures/runner-local/intent-native-erc20-transfer/runtime/runtime.local.json \
  --profile demo-scripted \
  --llm-script-jsonl fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.success.jsonl \
  --approvals-mode safe \
  --format text
```

当 CLI 提示确认时可输入：
- `approve|a`
- `deny|d`
- `always_approve_this_run|aa`
- `cancel|c`

## Scenario C: 失败后自动修订路径（RS-004）

首轮 `propose_plan` 会返回一个包含错误 condition 的 plan，执行暂停后 runner 会触发 `revise_plan`，并自动走 `replace_plan` 继续执行。

```bash
cargo run -p ais-runner -- agent \
  --intent-file fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --runtime fixtures/runner-local/intent-native-erc20-transfer/runtime/runtime.local.json \
  --profile demo-scripted \
  --llm-script-jsonl fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.repair.jsonl \
  --approvals-mode yolo \
  --max-planner-rounds 3 \
  --format text
```

可在输出中观察：
- `intent planner revise round=...`
- `plan_replaced` event
- 最终 `status: completed`

## Troubleshooting

- 本 fixture 聚焦 `intent -> plan -> execute/confirm` 链路演示。
- 如余额条件不满足，`a_transfer_native_5` 节点会被 `condition` 跳过，后续 transfer 节点不会执行。
- 若报 `connection refused`，确认本地链 `anvil` 已启动并监听 `127.0.0.1:8545`。
- 若报 `execution.type ... has no registered handler`，确认 `--workspace` 指向本目录 `workspace/`。
- 若报 token 调用失败，确认 `runtime/runtime.local.json` 中 ERC20 地址在本地链已部署、且钱包有余额。
- 若卡在 `need_user_confirm`，可输入 `aa` 在本次 run 内自动批准后续确认节点。
