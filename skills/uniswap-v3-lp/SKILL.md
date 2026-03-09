---
name: uniswap-v3-lp
description: >
  Manage Uniswap V3 LP positions: add liquidity (mint), remove liquidity, collect fees,
  and auto-rebalance positions when price moves out of range. Uses AIS to assemble
  transactions and clawlet to sign/send them. Use when: (1) adding LP to Uniswap V3,
  (2) removing or adjusting an existing position, (3) collecting fees, (4) setting up
  automatic position rebalancing when price changes, (5) authoring or extending the
  uniswap-v3-lp AIS protocol/pack/workflow files.
---

# Uniswap V3 LP Skill

Assets are in `skills/uniswap-v3-lp/assets/`:
- `uniswap-v3-lp.ais.yaml` — AIS protocol (mint, decrease-liquidity, collect-fees, queries)
- `uniswap-v3-lp-rebalance.ais-flow.yaml` — auto-rebalance workflow
- `uniswap-v3-lp.ais-pack.yaml` — pack with policy + token allowlist
- `runner.example.yaml` — runner config template (clawlet signer)

For LP concepts, tick math, range selection, and clawlet wiring: read [references/lp-concepts.md](references/lp-concepts.md).

## OpenClaw Bot 自动调仓

**架构：** OpenClaw cron → `ais-runner run workflow` (AIS 组装交易) → clawlet (签名发送)

Bot 不使用 agent/LLM 模式，只用 `run workflow`——完全确定性，无需 LLM。

### 核心命令

```bash
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.sepolia.yaml \
  --runtime /tmp/inputs.json \
  --format json
```

- 价格在范围内 → workflow assert 报错中止（调用方应先检查 tick 范围再触发）
- 价格超出范围 → `decreaseLiquidity` → `collect` → `mint`，clawlet 签名发送

### 注册 OpenClaw Cron（定时自动调仓）

用 cron 工具创建定时任务（每 5 分钟），payload 让 bot 在 isolated session 里 exec ais-runner：

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "Run ais-runner run workflow for Uniswap V3 LP rebalance, token_id 12345, Sepolia. Report rebalanced status.",
    "timeoutSeconds": 120
  },
  "delivery": { "mode": "announce" }
}
```

完整集成说明（初始化步骤、多 position 管理、输出解读、错误处理）：
→ [references/openclaw-bot.md](references/openclaw-bot.md)

---

## Quick Start (Sepolia — recommended for testing)

Sepolia contracts are already configured in the protocol file. No extra setup needed beyond runner config.

### 1. Set Up Sepolia Config

Copy `assets/runner.sepolia.yaml` → `config/runner.sepolia.yaml`. Fill in:
- `rpc_url` (e.g. `https://rpc.sepolia.org` or Alchemy/Infura endpoint)
- `signer.endpoint` — clawlet daemon address (default `http://localhost:7777`)
- `signer.account` — clawlet account label for your Sepolia wallet
- `runtime.ctx.wallet_address` — must match the clawlet account

**Sepolia tokens** (already in pack allowlist):
| Symbol | Address |
|--------|---------|
| WETH   | `0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14` |
| USDC   | `0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238` |

Need test ETH? → https://sepoliafaucet.com

### 2. Copy Workspace Files

```bash
cp assets/uniswap-v3-lp.ais.yaml                  workspace/
cp assets/uniswap-v3-lp-rebalance.ais-flow.yaml   workspace/
cp assets/uniswap-v3-lp.ais-pack.yaml             workspace/
```

### 3. Add LP on Sepolia (mint a new position)

Agent mode (natural language):
```bash
ais-runner agent \
  --intent "add liquidity to WETH/USDC pool fee 3000 with 0.01 WETH and 20 USDC on sepolia, tick range ±600 from current" \
  --config config/runner.sepolia.yaml \
  --workspace workspace/ \
  --pack workspace/uniswap-v3-lp.ais-pack.yaml \
  --approvals-mode safe
```

Or author an `ais-plan/0.0.3` referencing `mint-position` directly for deterministic execution.

### 4. Auto-Rebalance on Sepolia

```bash
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.sepolia.yaml \
  --runtime /tmp/rebalance-inputs.json
# /tmp/rebalance-inputs.json: {"inputs": {"token_id": 12345, "range_width_ticks": 600, "slippage_bps": 50}, "ctx": {"wallet_address": "0xYOUR_WALLET"}}
```

The workflow asserts the position IS out-of-range and aborts with an error if it is still in range. Check `outputs.current_tick` vs `outputs.old_tick_lower/upper` first in your bot loop before invoking.

For periodic auto-rebalancing, wrap in a cron loop or use OpenClaw cron with `agentTurn`.

---

## Quick Start (Mainnet)

Copy `assets/runner.example.yaml` → `config/runner.yaml`. Fill in `rpc_url`, clawlet signer, and `wallet_address`. Add tokens to the pack allowlist as needed. Same commands as above without the `-sepolia` suffix in config filename.

## Protocol Actions Summary

| Action | Risk Level | Description |
|--------|-----------|-------------|
| `mint-position` | 4 | Create new LP position (approves tokens + mints NFT) |
| `decrease-liquidity` | 3 | Remove liquidity from existing position |
| `collect-fees` | 1 | Collect fees / withdrawn tokens |

## Protocol Queries Summary

| Query | Description |
|-------|-------------|
| `position-info` | On-chain position data (tokens, ticks, liquidity) |
| `pool-slot0` | Current pool tick and sqrtPriceX96 |
| `get-pool` | Pool address from factory |
| `allowance-token0/1` | Approval check before mint |

## Clawlet 签名模型

**owlia bot 通过 CLI 调用 AIS，通过 RPC 调用 clawlet。**

```
owlia bot
  ├── CLI → ais-runner --dry-run    # 获取 unsigned tx
  └── clawlet signer（runner 内部）  # 签名 + 广播
```

- AIS 不持有私钥，不调用 clawlet
- `--dry-run` 只做编译预览，不执行查询或 tx 构造（输出 `DryRunJsonReport`）
- 实际执行：runner 调用 clawlet signer（需实现 `ClawletSigner`）完成签名+广播
- bot 只需调用一次 CLI，不直接调 clawlet RPC

详细集成流程、伪代码、cron 配置见 [references/openclaw-bot.md](references/openclaw-bot.md)。

## Extending the Protocol

- **New token pairs**: add to `token_policy.allowlist` in the pack file.
- **New chains**: add a `deployments` entry in the protocol file with correct contract addresses.
- **Tighter/wider ranges**: adjust `range_width_ticks` input to the rebalance workflow.
- **Custom rebalance triggers**: replace the out-of-range `assert` node with a price-deviation check.
