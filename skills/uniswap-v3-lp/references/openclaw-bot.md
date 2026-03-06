# OpenClaw Bot 集成 — 自动调仓

## 架构与事务签名模型

```
OpenClaw (oliwa bot)
  └── cron job (定时触发)
        └── ais-runner run workflow
              │
              │  AIS 只负责：
              │  - 读取链上数据（evm_read）
              │  - 组装 unsigned tx（ABI 编码 calldata + to + value）
              │
              └── unsigned tx ──→ clawlet
                                    ├── 签名（私钥在 clawlet 内，AIS 不接触）
                                    └── 广播到链
```

**关键原则：AIS 不配置 signer，不持有私钥。**
私钥完全在 clawlet 内部管理，AIS 只输出 unsigned transaction。

---

## Clawlet 集成方案

### 方案 A — ClawletSigner 插件（推荐，需实现）

在 `ais-evm-executor` crate 中实现 `ClawletSigner`：

```rust
// rust/ais-rs/crates/ais-evm-executor/src/signer.rs 中新增：
pub struct ClawletSigner {
    endpoint: String,   // e.g. "http://localhost:7777"
    account: String,    // clawlet account label
}

impl EvmTransactionSigner for ClawletSigner {
    fn sign_and_send(&self, tx: &UnsignedTx) -> Result<TxHash, SignerError> {
        // POST unsigned tx to clawlet endpoint
        // clawlet signs with its managed key and broadcasts
        // returns tx hash
    }
}
```

同时在 `ais-runner/src/config.rs` 的 `SignerConfig` enum 新增 variant：

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignerConfig {
    EvmPrivateKey { private_key: String },
    SolanaPrivateKey { private_key: String },
    Clawlet { endpoint: String, account: String },  // 新增
}
```

实现后，runner config 即可使用：
```yaml
signer:
  type: "clawlet"
  endpoint: "http://localhost:7777"
  account: "sepolia-test"
```

### 方案 B — Dry-run + pipe（临时方案，无需代码改动）

```bash
# ais-runner 输出 unsigned tx JSON，pipe 给 clawlet
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.sepolia.yaml \
  --inputs '{"token_id": "12345", "range_width_ticks": "600"}' \
  --dry-run \
  --format json \
| clawlet tx send --chain eip155:11155111 --account sepolia-test --stdin
```

---

## 设置 Cron 自动调仓

在 OpenClaw 中用 `cron` 工具创建定时任务：

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "Run Uniswap V3 LP rebalance for token_id 12345 on Sepolia. Use exec to run ais-runner, pipe unsigned tx to clawlet. Report whether rebalancing occurred.",
    "timeoutSeconds": 120
  },
  "delivery": { "mode": "announce" }
}
```

---

## 初始化步骤（由 bot 执行一次）

1. **准备 workspace**
   ```bash
   mkdir -p /path/to/workspace /path/to/config
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais.yaml         /path/to/workspace/
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp-rebalance.ais-flow.yaml /path/to/workspace/
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais-pack.yaml    /path/to/workspace/
   cp skills/uniswap-v3-lp/assets/runner.sepolia.yaml             /path/to/config/
   # 填写 rpc_url 和 wallet_address
   ```

2. **配置 clawlet**
   ```bash
   clawlet account add sepolia-test --key 0xYOUR_SEPOLIA_KEY
   clawlet start  # 启动 clawlet daemon
   ```

3. **验证（方案 B dry-run）**
   ```bash
   ais-runner run workflow \
     --workflow /path/to/workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
     --config /path/to/config/runner.sepolia.yaml \
     --inputs '{"token_id": "12345", "range_width_ticks": "600"}' \
     --dry-run --format json
   # 确认输出包含 unsigned tx 结构，再接入 clawlet
   ```

---

## 多个 Position 管理

```
Run rebalance for each position on Sepolia (token_ids: 12345, 67890).
Use dry-run mode, pipe each unsigned tx to clawlet for signing.
Report status for each.
```

---

## 输出解读

```json
{
  "status": "ok",
  "outputs": {
    "new_token_id": "12346",
    "current_tick": -74832,
    "old_tick_lower": -75600,
    "old_tick_upper": -74400,
    "collected_amount0": "1000000000000000",
    "collected_amount1": "500000"
  }
}
```

---

## 错误处理

| 错误 | 原因 | 处理 |
|------|------|------|
| `assert failed: liquidity > 0` | position 已无流动性 | 跳过该 token_id |
| `assert failed: pool_address != 0x0` | pool 不存在 | 检查 token pair 和 fee tier |
| `assert failed: position is still in range` | 价格在范围内 | 无需操作，下次继续监控 |
| `clawlet connection refused` | clawlet daemon 未运行 | `clawlet start` |
