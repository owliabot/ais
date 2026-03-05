# Runner local fixture: Uniswap V3 Sepolia

Fixture bundle for running Uniswap V3 swap/quote actions on Ethereum Sepolia testnet (`eip155:11155111`).

## Layout

```
config/
  runner.sepolia.yaml          # Runner config (RPC URL, signer, chain settings)
workspace/
  uniswap-v3-sepolia.ais.yaml       # Uniswap V3 Sepolia protocol (SwapRouter02 + QuoterV2)
  uniswap-v3-sepolia.ais-pack.yaml  # Pack with safe defaults (slippage, approvals, plugins)
```

## Setup

1. Edit `config/runner.sepolia.yaml`:
   - Set `chains["eip155:11155111"].rpc_url` to your Sepolia RPC endpoint (e.g. Alchemy/Infura/public).
   - Set `signer.private_key` to your Sepolia wallet private key (**testnet only**).
   - Set `runtime.ctx.wallet_address` to the corresponding address.
   - Uncomment the `llm` block and set a non-empty `api_key` if using `agent` mode. The runner rejects an empty key whenever the `llm` block is present.

2. Ensure your wallet holds some Sepolia ETH and the ERC-20 tokens you want to swap.

## Contracts (Sepolia)

| Contract    | Address |
|-------------|---------|
| SwapRouter02 | `0x3bFA4769FB09eefC5a80d6E87c3B9C650f7Ae48` |
| QuoterV2     | `0xEd1f6473345F45b75F8179591dd5bA1888cf2FB3` |
| Factory      | `0x0227628f3F023bb0B980b67D528571c95c6DaC1` |
| WETH9        | `0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14` |

## Run (from `rust/ais-rs`)

### Agent mode (intent → auto plan)

```bash
cargo run -p ais-runner -- agent \
  --intent "swap 0.01 WETH for USDC on sepolia with 0.3% fee" \
  --config fixtures/runner-local/uniswap-v3-sepolia/config/runner.sepolia.yaml \
  --workspace fixtures/runner-local/uniswap-v3-sepolia/workspace \
  --pack fixtures/runner-local/uniswap-v3-sepolia/workspace/uniswap-v3-sepolia.ais-pack.yaml \
  --approvals-mode safe
```

### Dry-run quote (validate protocol without execution)

`agent` does not support `--dry-run`. Use the `quote-exact-in` query via a hand-crafted plan, or just run the agent in `safe` mode which requires approval before any tx.

To validate the protocol document alone (no execution, no RPC):

```bash
cargo run -p ais-runner -- run workflow \
  --workflow fixtures/runner-local/uniswap-v3-sepolia/workspace/uniswap-v3-sepolia.ais-pack.yaml \
  --workspace fixtures/runner-local/uniswap-v3-sepolia/workspace \
  --dry-run --format json
```

## Troubleshooting

- **RPC errors**: check `rpc_url` and make sure it points to a Sepolia endpoint.
- **Insufficient allowance**: the protocol auto-approves if allowance < amount_in; check signer has gas.
- **Slippage revert**: increase `slippage_bps` (default 200 = 2%) or retry when liquidity improves.
- **`source: local` resolution**: runner resolves protocols by `protocol` name from files in `--workspace`.
