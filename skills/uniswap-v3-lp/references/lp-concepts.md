# Uniswap V3 LP — Key Concepts for AIS Integration

## Ticks and Price Ranges

- Uniswap V3 uses **ticks** (int24) to represent price boundaries.
- `tick = log(price) / log(1.0001)`, so `price = 1.0001^tick`.
- For a WETH/USDC pool, price is USDC per WETH. Current price comes from `slot0.tick`.
- **Tick spacing** is determined by fee tier:
  | Fee (bps) | Tick Spacing |
  |-----------|-------------|
  | 100       | 1           |
  | 500       | 10          |
  | 3000      | 60          |
  | 10000     | 200         |
- `tickLower` and `tickUpper` must be multiples of tick spacing.
- A position is **in range** when: `tick_lower ≤ current_tick ≤ tick_upper`.

## Token Ordering

- Uniswap V3 requires `token0.address < token1.address` (lexicographic).
- For WETH/USDC on mainnet: USDC (0xA0b8...) < WETH (0xC02a...) → token0=USDC, token1=WETH.
- Always sort before passing to mint-position.

## sqrtPriceX96 → Price Conversion

```
price = (sqrtPriceX96 / 2^96)^2
# For token1/token0 price
```

## Rebalancing Strategy

The `uniswap-v3-lp-rebalance` workflow:
1. Queries `positions(tokenId)` → gets current tick range + liquidity.
2. Queries `slot0` on the pool → gets current tick.
3. If `current_tick < tick_lower OR current_tick > tick_upper`: position is out of range.
4. Calls `decreaseLiquidity` with full liquidity, then `collect` to withdraw all tokens + fees.
5. Computes new tick range: `[current_tick - range_width, current_tick + range_width]`, aligned to tick spacing.
6. Calls `mint` to create new position at new range with collected amounts.

### Range Width Selection

| Volatility | Fee Tier | Suggested range_width_ticks |
|------------|----------|-----------------------------|
| Stable pairs (e.g. USDC/USDT) | 100 | 10–50 |
| Mid volatility (e.g. ETH/USDC) | 3000 | 600–1200 |
| High volatility | 10000 | 2000–4000 |

A narrow range earns more fees per unit of liquidity but gets out-of-range more often.

## Clawlet Integration

Clawlet is the transaction signer/broadcaster. AIS assembles the full transaction calldata
(including ABI encoding), then hands it to clawlet via the runner config:

```yaml
signer:
  type: "clawlet"
  endpoint: "http://localhost:7777"
  account: "default"
```

Clawlet handles:
- Key management (signing)
- Nonce management
- Gas estimation and bumping
- Broadcast + receipt waiting

AIS never touches private keys; it only produces calldata + target address.

## Running the Rebalance Workflow

```bash
# Agent mode — describe intent in natural language:
ais-runner agent \
  --intent "rebalance my WETH/USDC LP position token ID 12345, use ±600 tick range" \
  --config config/runner.yaml \
  --workspace workspace/ \
  --pack workspace/uniswap-v3-lp.ais-pack.yaml \
  --approvals-mode safe

# Direct workflow mode — pass inputs explicitly:
ais-runner run workflow \
  --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
  --config config/runner.yaml \
  --inputs '{"token_id": 12345, "range_width_ticks": 600, "slippage_bps": 50}'
```

## Automation / Periodic Rebalancing

To auto-rebalance on price change, run the workflow on a schedule (e.g. via cron or a loop):

```bash
# Example: check every 5 minutes
while true; do
  ais-runner run workflow \
    --workflow workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
    --config config/runner.yaml \
    --inputs '{"token_id": 12345, "range_width_ticks": 600}'
  sleep 300
done
```

The workflow is idempotent: if the position is already in range, it does nothing
(`rebalanced: false` in outputs).
