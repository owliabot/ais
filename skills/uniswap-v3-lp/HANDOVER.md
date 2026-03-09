# Uniswap V3 LP 自动调仓 — 交接文档

> 撰写时间：2026-03-09  
> 交接人：zzhen  
> 目标：owlia bot 集成 uniswap-v3-lp skill，实现用户查询池子、添加 LP、定时监控并自动调仓

---

## 一、功能概述

用户通过 owlia bot 交互，流程如下：

```
用户输入池子
  └─▶ 检查该池子下用户是否已有 LP position
        ├─ 无 position → 添加 LP（mint）
        └─ 有 position → 进入定时监控
              └─▶ 定时检查价格
                    ├─ 价格在范围内 → 不操作
                    └─ 价格超出范围 → 自动调仓（rebalance）
```

**交易流程（固定）：**
1. `ais-runner run workflow` 组装并执行交易
2. runner 内部通过 `ClawletSigner` 调用 clawlet 签名+广播

---

## 二、Skill 文件结构

```
skills/uniswap-v3-lp/
├── SKILL.md                              # OpenClaw skill 入口
├── HANDOVER.md                           # 本文档
├── assets/
│   ├── uniswap-v3-lp.ais.yaml           # AIS 协议（mint / decrease / collect / queries）
│   ├── uniswap-v3-lp-rebalance.ais-flow.yaml  # 自动调仓 workflow
│   ├── uniswap-v3-lp.ais-pack.yaml      # Pack（policy + token allowlist）
│   ├── runner.example.yaml              # Mainnet runner 配置模板
│   └── runner.sepolia.yaml              # Sepolia runner 配置模板
└── references/
    ├── lp-concepts.md                   # LP 概念、tick 数学、范围选择
    └── openclaw-bot.md                  # Bot 集成详细说明（当前状态 + 伪代码）
```

---

## 三、核心组件说明

### 3.1 AIS Protocol — `uniswap-v3-lp.ais.yaml`

定义可调用的 actions 和 queries：

| 类型 | 名称 | 风险等级 | 说明 |
|------|------|----------|------|
| action | `mint-position` | 4 | 分步 approve + mint（composite 类型，**待实现**） |
| action | `mint-position-atomic` | 4 | 单步 mint（需已授权） |
| action | `decrease-liquidity` | 3 | 移除流动性 |
| action | `collect-fees` | 1 | 领取手续费/代币 |
| query | `position-info` | — | 查询 NFT position 数据 |
| query | `pool-slot0` | — | 查询当前 tick 和价格 |
| query | `get-pool` | — | 查询池子地址 |
| query | `raw-allowance` | — | 查询 token 授权量 |

### 3.2 Rebalance Workflow — `uniswap-v3-lp-rebalance.ais-flow.yaml`

自动调仓的完整执行图，步骤：

```
1. query position-info        → 检查 liquidity > 0
2. query get-pool             → 验证池子存在
3. query pool-slot0           → 获取当前 tick（断言：price 超出范围）
4. action decrease-liquidity  → 移除全部流动性
5. query position-info        → 读取 tokensOwed（freed + fees）
6. action collect-fees        → 收回代币
7. query raw-allowance x2     → 检查 token0/1 授权
8. action mint-position-atomic → 在新 tick 范围内开新仓
```

**关键设计**：workflow 在 Step 3 assert 价格必须超出范围，否则中止。Bot 调用前应先比较 `current_tick` 与 `tickLower/tickUpper`。

### 3.3 Pack — `uniswap-v3-lp.ais-pack.yaml`

- 白名单代币：Sepolia WETH + USDC（mainnet 需手动添加）
- 策略：risk ≤ 1 自动执行，risk ≥ 3 需审批

---

## 四、⚠️ 当前未实现项（必读）

在 bot 完整上线前，以下两项必须实现：

### 4.1 `composite` 执行类型

`mint-position`（非 atomic 版本）的 action 类型为 `composite`，需按顺序执行 approve + mint 两步。

**当前状态**：`ais-evm-executor` 只处理 `evm_call / evm_read / evm_rpc`，无 `composite` handler，调用会报路由错误。

**实现位置**：`rust/ais-rs/crates/ais-evm-executor/src/executor.rs`

**临时方案**：使用 `mint-position-atomic`（workflow 中已使用此 action），需用户侧提前手动 approve token allowance。

### 4.2 `ClawletSigner`

**当前状态**：`ais-evm-executor` 只支持 `evm_private_key` signer。clawlet 集成缺少：
- `SignerConfig::Clawlet` variant（`ais-runner/src/config.rs`）
- `ClawletSigner` 实现（`ais-evm-executor/src/signer.rs`）

**实现后配置方式**：
```yaml
chains:
  "eip155:11155111":
    rpc_url: "https://rpc.sepolia.org"
    signer:
      type: "clawlet"
      endpoint: "http://localhost:7777"
      account: "sepolia-test"
```

---

## 五、Bot 集成流程

### 5.1 用户输入池子 → 检查是否存在 LP

Bot 需要：
1. 从用户输入解析出 `token0`, `token1`, `fee`
2. 查询 NonfungiblePositionManager 获取用户的 token IDs（链上 `tokenOfOwnerByIndex` 或 subgraph）
3. 对每个 token ID 调用 `position-info` query，匹配 pool 的 token0/token1/fee

> ⚠️ AIS protocol 中目前没有"列出用户所有 positions"的 query，需要 bot 自行实现这步链上查询（可用 ethers/viem 直调 NonfungiblePositionManager）。

### 5.2 无 LP → 添加流动性（mint）

```bash
ais-runner agent \
  --intent "add liquidity to WETH/USDC pool fee 3000 with 0.01 WETH and 20 USDC on sepolia, tick range ±600 from current" \
  --config config/runner.sepolia.yaml \
  --workspace workspace/ \
  --pack workspace/uniswap-v3-lp.ais-pack.yaml \
  --approvals-mode safe
```

或者确定性执行（推荐 bot 使用）：编写 `ais-plan/0.0.3` 直接 ref `mint-position-atomic`。

### 5.3 已有 LP → 定时监控 + 自动调仓

**定时检查**（OpenClaw cron，每 5 分钟）：

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "Check and rebalance Uniswap V3 LP position token_id=<TOKEN_ID> on Sepolia. Write runtime inputs (token_id, range_width_ticks=600, slippage_bps=50, wallet_address) to a temp file, run ais-runner run workflow with that file. If rebalanced, report new token_id and tick range. If in-range, report no action needed.",
    "timeoutSeconds": 180
  },
  "delivery": { "mode": "announce" }
}
```

**手动执行调仓**：

```bash
# 1. 创建 inputs 文件
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

# 2. 执行 workflow
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.sepolia.yaml \
  --runtime /tmp/rebalance-inputs.json \
  --format json
```

**输出解读**：

| 字段 | 含义 |
|------|------|
| `new_token_id` | 新仓位的 NFT token ID |
| `current_tick` | 触发调仓时的 pool tick |
| `old_tick_lower/upper` | 原仓位 tick 范围 |
| `owed_before_collect_amount0/1` | decrease 后可 collect 的数量（atomic） |

---

## 六、初始化步骤（接手后执行）

```bash
# 1. 复制 workspace 文件
cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais.yaml                workspace/
cp skills/uniswap-v3-lp/assets/uniswap-v3-lp-rebalance.ais-flow.yaml workspace/
cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais-pack.yaml           workspace/
cp skills/uniswap-v3-lp/assets/runner.sepolia.yaml                    config/

# 2. 编辑 config/runner.sepolia.yaml
#    - 填写 rpc_url
#    - 填写 signer（目前用 evm_private_key，待 ClawletSigner 实现后切换）
#    - 填写 wallet_address

# 3. 验证编译（dry-run，不执行链上操作）
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.sepolia.yaml \
  --dry-run --format json
# 期望输出: issues: []

# 4. 实现 composite handler 和 ClawletSigner（见第四节）

# 5. 注册 OpenClaw cron 定时任务
```

---

## 七、错误处理速查

| 错误信息 | 原因 | 处理方式 |
|----------|------|----------|
| `assert failed: liquidity > 0` | position 已无流动性 | 跳过该 position |
| `assert failed: pool != 0x0` | pool 不存在 | 检查 token pair 和 fee tier |
| `assert failed: position is still in range` | 价格仍在范围内 | 正常，无需操作 |
| `unregistered execution type: composite` | composite handler 未实现 | 改用 `mint-position-atomic` |
| `connection refused :7777` | clawlet daemon 未运行 | 执行 `clawlet start` |
| tx reverted | 链上执行失败 | 检查余额、授权、tick 对齐 |

---

## 八、待完成任务清单

| 优先级 | 任务 | 位置 | 负责人 |
|--------|------|------|--------|
| 🔴 高 | 实现 `composite` execution handler | `ais-evm-executor/src/executor.rs` | — |
| 🔴 高 | 实现 `ClawletSigner` + `SignerConfig::Clawlet` | `ais-evm-executor/src/signer.rs` + `ais-runner/src/config.rs` | — |
| 🟡 中 | Bot 侧实现"列出用户所有 positions"查询 | owlia bot 代码 | — |
| 🟡 中 | 多 position 管理（按 pool 索引，每 position 独立 cron） | owlia bot 代码 | — |
| 🟢 低 | 添加 mainnet token 到 pack allowlist | `assets/uniswap-v3-lp.ais-pack.yaml` | — |
| 🟢 低 | Subgraph 集成（更高效查询用户 positions） | owlia bot 代码 | — |

---

## 九、关键参考

- **LP 概念 & tick 数学**：`references/lp-concepts.md`
- **Bot 集成详细伪代码**：`references/openclaw-bot.md`
- **AIS runner 文档**：`rust/ais-rs/fixtures/runner-local/uniswap-v3-sepolia/README.md`
- **Uniswap V3 合约（Sepolia）**：
  - NonfungiblePositionManager: `0x1238536071E1c677A632429e3655c799b22cDA52`
  - Factory: `0x0227628f3F023bb0B980b67D528571c95c6DaC1c`
- **测试 ETH**：https://sepoliafaucet.com
