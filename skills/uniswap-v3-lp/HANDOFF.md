# Uniswap V3 LP 自动调仓 — 交接文档

**日期：** 2026-03-09  
**负责人：** zzhen  
**接收方：** 后续维护者

---

## 1. 任务目标

在 owlia bot 内通过 OpenClaw skill 实现 Uniswap V3 LP 自动调仓，完整流程：

1. 用户输入池子参数（token pair + fee tier）
2. Bot 检查用户在该池子下是否已有 LP position
   - 无 position → 自动添加 LP（mint）
   - 有 position → 继续监控
3. 定时监控池子价格，当价格超出 position 范围时，自动调仓（rebalance）
4. 所有交易通过 `ais-runner` 组装，`clawlet` 签名发送

---

## 2. 整体架构

```
owlia bot (OpenClaw skill)
  │
  ├─ 用户触发 / cron 定时
  │
  ├─ ais-runner run workflow          ← 组装交易（纯确定性，无 LLM）
  │     ├─ evm_read  → 读链上数据
  │     └─ evm_call  → 构造 tx 并通过 clawlet 签名+广播
  │
  └─ clawlet (daemon)                 ← 签名 + 广播到链
        └─ signer.type: clawlet (需实现)
```

---

## 3. 已完成的工作

### Skill 文件（`skills/uniswap-v3-lp/`）

| 文件 | 说明 |
|------|------|
| `SKILL.md` | Skill 入口文档，bot 触发此 skill 时读取 |
| `assets/uniswap-v3-lp.ais.yaml` | AIS 协议：mint、decrease-liquidity、collect-fees、queries |
| `assets/uniswap-v3-lp-rebalance.ais-flow.yaml` | 调仓 workflow（8 步：查询 → 撤流动性 → collect → 重新 mint）|
| `assets/uniswap-v3-lp.ais-pack.yaml` | Pack（policy + token allowlist） |
| `assets/runner.sepolia.yaml` | Sepolia 测试链 runner config 模板 |
| `assets/runner.example.yaml` | 主网 runner config 模板 |
| `references/lp-concepts.md` | LP 概念、tick 数学、范围选择说明 |
| `references/openclaw-bot.md` | Bot 集成方案、伪代码、错误处理、cron 配置 |

### Rebalance Workflow 节点流

```
q_position      → 读取当前 position（assert: liquidity > 0）
q_pool          → 获取 pool 地址（assert: pool 存在）
q_slot0         → 读取当前 tick（assert: 价格超出范围）
a_decrease      → decreaseLiquidity（全部撤出）
q_position_after→ 再次查 position 获取 tokensOwed
a_collect       → collect（收回代币 + 手续费）
q_allowance0/1  → 检查 token approve 状态
a_mint          → mint-position-atomic（在新范围重开仓）
```

**Outputs：** `new_token_id`、`current_tick`、`old_tick_lower/upper`、`owed_before_collect_amount0/1`

---

## 4. 尚未实现（必须完成才能上线）

### 4.1 `composite` 执行类型（高优先级）

**位置：** `rust/ais-rs/crates/ais-evm-executor/src/executor.rs`

**问题：** `mint-position` 和 `mint-position-atomic` 使用 `type: composite`（approve + mint 两步），
当前 executor 只注册了 `evm_call`、`evm_read`、`evm_rpc`，没有 `composite` handler，执行报路由错误。

**修复方案：**
- 在 executor 注册 `composite` handler
- 按顺序执行 steps，每步评估 condition（如 `skip_if`）决定是否跳过
- Step 示例（来自协议文件 mint-position-atomic）：
  ```yaml
  steps:
    - id: approve_token0
      type: evm_call
      skip_if: "inputs.token0_allowance >= inputs.amount0_atomic"
      ...
    - id: approve_token1
      type: evm_call
      skip_if: "inputs.token1_allowance >= inputs.amount1_atomic"
      ...
    - id: mint
      type: evm_call
      ...
  ```

### 4.2 `ClawletSigner` 实现（高优先级）

**位置：**
- `rust/ais-rs/crates/ais-evm-executor/src/signer.rs` — 实现 `ClawletSigner`
- `rust/ais-rs/crates/ais-runner/src/config.rs` — 添加 `SignerConfig::Clawlet` variant

**当前状态：** runner 只支持 `evm_private_key`。

**需要实现：**
```rust
// config.rs
pub enum SignerConfig {
    EvmPrivateKey { key: String },
    Clawlet {
        endpoint: String,   // e.g. "http://localhost:7777"
        account: String,    // clawlet account label
    },
}

// signer.rs
pub struct ClawletSigner {
    endpoint: String,
    account: String,
}

impl Signer for ClawletSigner {
    async fn sign_and_send(&self, tx: TypedTransaction) -> Result<TxHash> {
        // POST to clawlet RPC: { account, tx_data }
        // return tx hash
    }
}
```

**runner.yaml 对应配置（实现后）：**
```yaml
chains:
  "eip155:11155111":
    rpc_url: "https://rpc.sepolia.org"
    signer:
      type: "clawlet"
      endpoint: "http://localhost:7777"
      account: "sepolia-test"
```

### 4.3 用户 Position 查询 + 初始 Mint 流程

**当前状态：** rebalance workflow 假设 position 已存在（需要 `token_id` 作为输入）。

**缺少：**
- 根据 wallet_address + pool 查询用户所有 positions（链上 NFT 枚举或 subgraph）
- 如果没有 position → 触发初始 mint（已有 `mint-position-atomic` action，需要封装成 workflow 或 agent intent）

**推荐实现方案：**
```python
# bot 伪代码
positions = query_positions(wallet, token0, token1, fee)
if not positions:
    # 调用 ais-runner agent 模式 mint 新 position
    mint_new_position(token0, token1, fee, amount0, amount1)
else:
    # 使用已有 token_id 运行 rebalance workflow
    for pos in positions:
        check_and_rebalance(pos.token_id)
```

**查询方式选项（二选一）：**
1. **链上枚举**：调用 `NonfungiblePositionManager.balanceOf(wallet)` + `tokenOfOwnerByIndex(wallet, i)` + `positions(tokenId)` 筛选 pool
2. **Subgraph**：查询 The Graph Uniswap V3 subgraph（速度快，但依赖中心化服务）

---

## 5. Bot 集成调用示例

### 检查并调仓（完整后）

```python
import subprocess, json, tempfile, os

def run_rebalance(token_id: str, wallet: str, range_width: str = "600"):
    inputs = {
        "inputs": {
            "token_id": token_id,
            "range_width_ticks": range_width,
            "slippage_bps": "50"
        },
        "ctx": {"wallet_address": wallet}
    }
    with tempfile.NamedTemporaryFile(mode='w', suffix='.json', delete=False) as f:
        json.dump(inputs, f)
        tmp = f.name

    try:
        result = subprocess.run([
            "ais-runner", "run", "workflow",
            "--workflow", "workspace/uniswap-v3-lp-rebalance.ais-flow.yaml",
            "--config",   "config/runner.yaml",
            "--runtime",  tmp,
            "--format",   "json"
        ], capture_output=True, text=True)

        if result.returncode == 0:
            out = json.loads(result.stdout)
            print("调仓成功, new token_id:", out["outputs"]["new_token_id"])
        else:
            err = result.stderr
            if "position is still in range" in err:
                print("价格在范围内，无需调仓")
            else:
                print("错误:", err)
    finally:
        os.unlink(tmp)
```

### OpenClaw Cron 配置（每 5 分钟）

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "检查 Uniswap V3 LP position token_id=12345 是否需要调仓。写 runtime inputs（token_id, range_width_ticks=600, slippage_bps=50, wallet_address=0xYOUR）到临时文件，运行 ais-runner run workflow --runtime <file>，报告结果。",
    "timeoutSeconds": 180
  },
  "delivery": { "mode": "announce" }
}
```

---

## 6. 测试环境（Sepolia）

| 配置项 | 值 |
|--------|-----|
| Chain | eip155:11155111 |
| WETH | `0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14` |
| USDC | `0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238` |
| NonfungiblePositionManager | 协议文件中已配置 |
| 测试 ETH | https://sepoliafaucet.com |

**验证编译（可现在跑）：**
```bash
cd rust/ais-rs
cargo run -p ais-runner -- run workflow \
  --workflow ../../skills/uniswap-v3-lp/assets/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config   ../../skills/uniswap-v3-lp/assets/runner.sepolia.yaml \
  --dry-run  --format json
# 预期: issues: [] — 编译通过
```

---

## 7. 任务优先级清单

| 优先级 | 任务 | 状态 |
|--------|------|------|
| P0 | 实现 `composite` execution handler | ❌ 待实现 |
| P0 | 实现 `ClawletSigner` + `SignerConfig::Clawlet` | ❌ 待实现 |
| P1 | 实现 position 枚举（根据 wallet + pool 查找 tokenIds） | ❌ 待实现 |
| P1 | 实现初始 mint workflow / agent 触发 | ❌ 待实现 |
| P2 | 配置 OpenClaw cron 定时任务 | ⏳ 等 P0 完成 |
| P2 | Sepolia 端到端测试 | ⏳ 等 P0 完成 |
| P3 | 主网部署 | ⏳ 等测试通过 |

---

## 8. 关键文件路径速查

```
skills/uniswap-v3-lp/
├── SKILL.md                                    ← skill 入口（OpenClaw 读取）
├── HANDOFF.md                                  ← 本文档
├── assets/
│   ├── uniswap-v3-lp.ais.yaml                 ← AIS 协议定义
│   ├── uniswap-v3-lp-rebalance.ais-flow.yaml  ← 调仓 workflow
│   ├── uniswap-v3-lp.ais-pack.yaml            ← Pack（policy + allowlist）
│   ├── runner.sepolia.yaml                     ← Sepolia config 模板
│   └── runner.example.yaml                     ← 主网 config 模板
└── references/
    ├── lp-concepts.md                          ← LP 概念和 tick 数学
    └── openclaw-bot.md                         ← Bot 集成详细说明

rust/ais-rs/crates/
├── ais-evm-executor/src/executor.rs            ← 需加 composite handler
├── ais-evm-executor/src/signer.rs              ← 需加 ClawletSigner
└── ais-runner/src/config.rs                    ← 需加 SignerConfig::Clawlet
```

---

## 9. 联系 & 参考

- **AIS 文档：** `docs/` 或 https://docs.openclaw.ai
- **Uniswap V3 合约：** https://docs.uniswap.org/contracts/v3/reference/overview
- **clawlet API：** 参考 clawlet daemon RPC 文档（内部）
- **有问题找：** zzhen（Discord: zz.hen）
