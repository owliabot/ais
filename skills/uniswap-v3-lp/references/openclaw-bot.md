# OpenClaw Bot 集成 — 自动调仓

## 架构

```
owlia bot
  ├── CLI  → ais-runner run workflow    # 组装并执行 tx（通过 evm_call）
  └── RPC  → clawlet                   # 签名 + 广播到链（signer 需实现）
```

---

## ⚠️ 当前实现状态与限制

在集成之前，需要了解以下两个尚未实现的功能：

### 1. composite 执行类型（需实现）

`mint-position` 和 `mint-position-atomic` 的 action 使用 `type: composite`（approve + mint 两步）。

当前 `ais-evm-executor` 只注册了 `evm_call`、`evm_read`、`evm_rpc`，**没有 `composite` handler**。
执行此类 action 时会报路由错误。

**修复路径**：在 `ais-evm-executor` 实现 composite 执行逻辑，按顺序执行 steps，每步评估 condition 后决定是否跳过。

### 2. clawlet signer（需实现）

当前 runner `SignerConfig` 只支持 `evm_private_key`。
clawlet 集成需要实现 `ClawletSigner` 并添加 `SignerConfig::Clawlet` variant。

---

## CLI 正确用法

### 运行 workflow 的正确参数

`run workflow` **没有** `--inputs` 参数。inputs 通过 `--runtime <file>` 传入：

```bash
# 1. 创建 runtime inputs 文件
cat > /tmp/rebalance-inputs.json << 'EOF'
{
  "inputs": {
    "token_id": "12345",
    "range_width_ticks": "600",
    "slippage_bps": "50"
  },
  "ctx": {
    "wallet_address": "0xYOUR_WALLET_ADDRESS"
  }
}
EOF
# Note: ABI field names are camelCase in node outputs (tickLower, tokensOwed0 etc.)
# Workflow refs use these directly, e.g. nodes.q_position.outputs.tickLower

# 2. 运行 workflow
ais-runner run workflow \
  --workflow /path/to/workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config /path/to/config/runner.sepolia.yaml \
  --runtime /tmp/rebalance-inputs.json \
  --format json
```

### `--dry-run` 的实际行为

`--dry-run` 输出的是**编译预览报告**（`DryRunJsonReport`），不是 unsigned tx。
它只告诉你 workflow 能否被编译成 plan，不会执行任何 evm_read 或 tx 构造。

```bash
# dry-run 只做编译检查，不执行
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.sepolia.yaml \
  --dry-run \
  --format json
# 输出: {schema, summary, plan_hash, nodes, issues} — 无 tx 数据
```

---

## Bot 集成方案（待 composite + clawlet 实现后）

owlia bot 的调用路径：

```
Step 1: CLI → ais-runner run workflow
  - runner 执行 evm_read 读链上数据
  - runner 调用 clawlet signer 完成 evm_call（签名+广播）
  - runner 返回执行结果（outputs）

Step 2: bot 读取 outputs 判断是否调仓成功
```

实现 `ClawletSigner` 后，runner config 配置：

```yaml
chains:
  "eip155:11155111":
    rpc_url: "https://rpc.sepolia.org"
    signer:
      type: "clawlet"
      endpoint: "http://localhost:7777"
      account: "sepolia-test"
```

然后 bot 只需调用一次 CLI：

```python
import subprocess, json

def rebalance(token_id: str, range_width: str = "600"):
    # 写 runtime inputs 文件
    inputs = {
        "inputs": {
            "token_id": token_id,
            "range_width_ticks": range_width,
            "slippage_bps": "50"
        },
        "ctx": {"wallet_address": "0xYOUR_WALLET_ADDRESS"}
    }
    with open("/tmp/inputs.json", "w") as f:
        json.dump(inputs, f)

    result = subprocess.run([
        "ais-runner", "run", "workflow",
        "--workflow", "workspace/uniswap-v3-lp-rebalance.ais-flow.yaml",
        "--config", "config/runner.sepolia.yaml",
        "--runtime", "/tmp/inputs.json",
        "--format", "json"
    ], capture_output=True, text=True, check=True)

    output = json.loads(result.stdout)
    print(output.get("outputs", {}))
```

clawlet signer 在 runner 内部被调用，bot 不直接调 clawlet RPC。
（runner 自己处理 approve 和 mint 的顺序执行）

---

## OpenClaw Cron 配置

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "Rebalance Uniswap V3 LP position token_id=12345 on Sepolia. Write runtime inputs to a temp file with token_id, range_width_ticks=600, slippage_bps=50 and wallet_address. Run ais-runner run workflow with --runtime pointing to that file. Report the outputs.",
    "timeoutSeconds": 180
  },
  "delivery": { "mode": "announce" }
}
```

---

## 初始化步骤

1. **准备 workspace**
   ```bash
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais.yaml         workspace/
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp-rebalance.ais-flow.yaml workspace/
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais-pack.yaml    workspace/
   cp skills/uniswap-v3-lp/assets/runner.sepolia.yaml             config/
   # 填写 rpc_url 和 wallet_address
   ```

2. **等待 composite + clawlet signer 实现后配置 signer**

3. **验证 dry-run 编译**
   ```bash
   ais-runner run workflow \
     --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
     --config config/runner.sepolia.yaml \
     --dry-run --format json
   # 应输出 issues: [] 表示编译通过
   ```

---

## 错误处理

| 错误 | 原因 | 处理 |
|------|------|------|
| `assert failed: liquidity > 0` | position 已无流动性 | 跳过 |
| `assert failed: pool_address != 0x0` | pool 不存在 | 检查 token pair 和 fee |
| `assert failed: position is still in range` | 价格在范围内 | 正常，无需操作 |
| `unregistered execution type: composite` | composite handler 未实现 | 等待实现 |
| clawlet connection refused | daemon 未运行 | `clawlet start` |
| tx reverted | 链上执行失败 | 检查余额和授权 |

---

## 待实现任务

| 任务 | 位置 | 优先级 |
|------|------|--------|
| `composite` execution handler | `ais-evm-executor/src/executor.rs` | 高 |
| `ClawletSigner` + `SignerConfig::Clawlet` | `ais-evm-executor/src/signer.rs` + `ais-runner/src/config.rs` | 高 |
