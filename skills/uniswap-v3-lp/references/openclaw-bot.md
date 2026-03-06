# OpenClaw Bot 集成 — 自动调仓

## 架构

```
owlia bot
  ├── CLI  → ais-runner run workflow    # 组装 unsigned tx
  └── RPC  → clawlet                   # 签名 + 广播到链
```

owlia bot 的职责：
1. 调用 `ais-runner` CLI 获取 unsigned tx（calldata + to + value + chain）
2. 通过 clawlet RPC 将 unsigned tx 提交，clawlet 签名并广播

AIS 不持有私钥，不调用 clawlet。owlia bot 是两者之间的桥梁。

---

## 集成流程

### Step 1 — ais-runner CLI 生成 unsigned tx

```bash
ais-runner run workflow \
  --workflow /path/to/workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config /path/to/config/runner.sepolia.yaml \
  --inputs '{"token_id": "12345", "range_width_ticks": "600", "slippage_bps": "50"}' \
  --dry-run \
  --format json
```

`--dry-run`: AIS 执行所有 evm_read 查询、计算、验证，但不广播 — 输出 unsigned tx JSON。

输出示例：
```json
{
  "status": "ok",
  "pending_transactions": [
    {
      "step": "a_decrease",
      "chain": "eip155:11155111",
      "to": "0x1238536071E1c677A632429e3655c799b22cDA52",
      "calldata": "0x0c49ccbe...",
      "value": "0x0"
    },
    {
      "step": "a_collect",
      "chain": "eip155:11155111",
      "to": "0x1238536071E1c677A632429e3655c799b22cDA52",
      "calldata": "0xfc6f7865...",
      "value": "0x0"
    },
    {
      "step": "a_mint",
      "chain": "eip155:11155111",
      "to": "0x1238536071E1c677A632429e3655c799b22cDA52",
      "calldata": "0x88316456...",
      "value": "0x0"
    }
  ]
}
```

如果 `pending_transactions` 为空（in-range assert 触发）→ 无需调仓，bot 跳过 clawlet 调用。

### Step 2 — clawlet RPC 签名并广播

bot 对每笔 pending_transaction 调用 clawlet RPC：

```http
POST http://localhost:7777/tx/send
Content-Type: application/json

{
  "chain": "eip155:11155111",
  "account": "sepolia-test",
  "to": "0x1238536071E1c677A632429e3655c799b22cDA52",
  "calldata": "0x0c49ccbe...",
  "value": "0x0"
}
```

clawlet 返回 tx hash，bot 等待 receipt。

> **注意**：多步事务（decrease → collect → mint）必须按顺序串行执行，每步等待 receipt 确认后再调用下一步。

---

## 完整 Bot 伪代码

```python
import subprocess, json, requests

RUNNER_CONFIG = "/path/to/config/runner.sepolia.yaml"
WORKSPACE     = "/path/to/workspace"
CLAWLET_URL   = "http://localhost:7777"
CLAWLET_ACCT  = "sepolia-test"

def rebalance(token_id: str, range_width: str = "600"):
    # Step 1: AIS CLI → unsigned txs
    result = subprocess.run([
        "ais-runner", "run", "workflow",
        "--workflow", f"{WORKSPACE}/uniswap-v3-lp-rebalance.ais-flow.yaml",
        "--config", RUNNER_CONFIG,
        "--inputs", json.dumps({"token_id": token_id, "range_width_ticks": range_width}),
        "--dry-run", "--format", "json"
    ], capture_output=True, text=True, check=True)

    output = json.loads(result.stdout)

    # If assert fired (in-range), no txs to send
    pending = output.get("pending_transactions", [])
    if not pending:
        print(f"[{token_id}] in range, no rebalance needed")
        return

    # Step 2: clawlet RPC → sign + broadcast each tx in order
    for tx in pending:
        resp = requests.post(f"{CLAWLET_URL}/tx/send", json={
            "chain":    tx["chain"],
            "account":  CLAWLET_ACCT,
            "to":       tx["to"],
            "calldata": tx["calldata"],
            "value":    tx.get("value", "0x0"),
        })
        resp.raise_for_status()
        tx_hash = resp.json()["tx_hash"]
        print(f"[{token_id}] {tx['step']} → {tx_hash}")
        wait_for_receipt(tx_hash)   # poll clawlet or RPC node

# Run every 5 minutes
while True:
    rebalance("12345")
    time.sleep(300)
```

---

## OpenClaw Cron 配置

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "Check and rebalance Uniswap V3 LP position token_id=12345 on Sepolia. Step 1: exec ais-runner with --dry-run to get unsigned txs. Step 2: for each pending_transaction, call clawlet RPC at http://localhost:7777/tx/send to sign and broadcast. Wait for receipt between steps. Report final status.",
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
   # 填写 rpc_url 和 wallet_address（clawlet 账户地址）
   ```

2. **配置 clawlet**
   ```bash
   clawlet account add sepolia-test --key 0xYOUR_SEPOLIA_KEY
   clawlet start   # 启动 RPC daemon（默认 :7777）
   ```

3. **验证 AIS CLI**
   ```bash
   ais-runner run workflow \
     --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
     --config config/runner.sepolia.yaml \
     --inputs '{"token_id": "12345"}' \
     --dry-run --format json
   # 应输出 pending_transactions 数组或空数组（in-range）
   ```

---

## 错误处理

| 错误 | 原因 | 处理 |
|------|------|------|
| `assert failed: liquidity > 0` | position 已无流动性 | 跳过，不调 clawlet |
| `assert failed: pool_address != 0x0` | pool 不存在 | 检查 token pair 和 fee tier |
| `assert failed: position is still in range` | 价格在范围内 | 正常，无需操作 |
| clawlet RPC 4xx/5xx | 账户或参数错误 | 检查 account 和 calldata |
| clawlet RPC connection refused | daemon 未运行 | `clawlet start` |
| tx reverted | 链上执行失败 | 检查 Sepolia 余额和 token 授权 |
