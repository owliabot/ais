# Runner local fixture: Uniswap V3 — Sepolia

端到端 fixture：在 Sepolia 测试网通过 Uniswap V3 执行 swap。

## 合约地址（Sepolia, eip155:11155111）

| 合约 | 地址 |
|------|------|
| SwapRouter02 | `0x3bFA4769FB09eefC5a80d6E87c3B9C650f7Ae48` |
| QuoterV2 | `0xEd1f6473345F45b75F8179591dd5bA1888cf2FB3` |
| Factory | `0x0227628f3F023bb0B980b67D528571c95c6DaC1c` |
| NonfungiblePositionManager | `0x1238536071E1c677A632429e3655c799b22cDA52` |

| Token | 地址 | Decimals |
|-------|------|----------|
| WETH | `0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14` | 18 |
| USDC (test) | `0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238` | 6 |

## Layout

```
config/
  runner.sepolia.yaml         # 链配置（需填写 rpc_url / private_key）
workspace/
  uniswap-v3.ais.yaml         # Uniswap V3 protocol（含 Sepolia deployment）
  uniswap-v3-sepolia.ais-pack.yaml  # pack（token allowlist、policy）
```

## 前置条件

1. Sepolia 钱包（有测试 ETH）
2. 编辑 `config/runner.sepolia.yaml`，填写：
   - `chains["eip155:11155111"].rpc_url`（e.g. Alchemy / Infura / `https://rpc.sepolia.org`）
   - `chains["eip155:11155111"].signer.private_key`（Sepolia 测试私钥，**勿用于主网**）
   - `runtime.ctx.wallet_address`（与私钥对应的地址）
3. 若使用 agent 模式，取消注释并填写 `llm` 块的 `api_key`

## 运行（agent 模式）

在 `rust/ais-rs` 下：

```bash
cargo run -p ais-runner -- agent \
  --intent "swap 0.01 WETH for USDC on sepolia fee 3000" \
  --config fixtures/runner-local/uniswap-v3-sepolia/config/runner.sepolia.yaml \
  --workspace fixtures/runner-local/uniswap-v3-sepolia/workspace \
  --pack fixtures/runner-local/uniswap-v3-sepolia/workspace/uniswap-v3-sepolia.ais-pack.yaml \
  --approvals-mode safe
```

`--approvals-mode` 可选：
- `safe` — 执行前人工确认（推荐）
- `assist` — 低风险自动，高风险人工确认
- `yolo` — 全自动（测试用）

## Troubleshooting

- **connection refused** — 检查 `rpc_url` 是否可访问
- **insufficient funds** — 钱包需有 Sepolia ETH（水龙头：https://sepoliafaucet.com）
- **quote fails** — Sepolia 上部分池流动性极低，尝试换 fee tier（500 / 3000）
- **USDC 余额不足** — 需先获取测试 USDC，或改为 WETH→其他已有流动性的对
