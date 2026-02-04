# AIS-2: Execution Types

> Status: Draft
> Version: 1.0.0

## Abstract

AIS-2 defines chain-specific execution formats. Each execution type describes exactly how to build a transaction for a given blockchain architecture.

---

## Execution Types Overview

| Type | Description | Use Case |
|------|-------------|----------|
| `evm_read` | Single eth_call read | Query single value |
| `evm_multiread` | Batched multicall read | Query multiple values efficiently |
| `evm_call` | Single write transaction | Simple state change |
| `evm_multicall` | Batched write transaction | Multiple writes in one tx |
| `composite` | Multi-step execution | Approve + swap patterns |
| `solana_instruction` | Solana program call | Solana transactions |
| `cosmos_message` | Cosmos SDK message | Cosmos chain txs |
| `bitcoin_psbt` | Bitcoin PSBT | Bitcoin transactions |
| `move_entry` | Move entry function | Aptos/Sui transactions |

---

## 1. `evm_read` — EVM Single Read

For single `eth_call` operations (queries).

```yaml
execution:
  "eip155:*":
    type: evm_read
    contract: string                  # Contract reference from deployments.
    function: string                  # View function name.
    abi: string                       # Parameter type signature.
    mapping: object                   # Param name → value source mapping.
```

**Example:**

```yaml
execution:
  "eip155:*":
    type: evm_read
    contract: "quoter"
    function: "quoteExactInputSingle"
    abi: "(address,address,uint24,uint256,uint160)"
    mapping:
      tokenIn: "params.token_in.address"
      tokenOut: "params.token_out.address"
      fee: "params.fee"
      amountIn: "to_atomic(params.amount_in, params.token_in)"
      sqrtPriceLimitX96: "0"
```

---

## 2. `evm_multiread` — EVM Batched Read

For multiple reads in a single RPC call using Multicall3 or RPC batching.

```yaml
execution:
  "eip155:*":
    type: evm_multiread
    method: "multicall3" | "rpc_batch"  # Aggregation method.
    calls:
      - contract: string
        function: string
        abi: string
        mapping: object
        output_as: string             # Name for this result.
```

**Example:**

```yaml
execution:
  "eip155:*":
    type: evm_multiread
    method: "multicall3"
    calls:
      - contract: "token_in.address"
        function: "balanceOf"
        abi: "(address)"
        mapping:
          owner: "ctx.wallet_address"
        output_as: "balance"
      - contract: "token_in.address"
        function: "allowance"
        abi: "(address,address)"
        mapping:
          owner: "ctx.wallet_address"
          spender: "contracts.router"
        output_as: "allowance"
```

---

## 3. `evm_call` — EVM Single Write

For Ethereum, Arbitrum, Base, Polygon, Optimism, and all EVM-compatible chains.

```yaml
execution:
  "eip155:*":
    type: evm_call
    contract: string                  # Reference to deployment contract name.
    function: string                  # Solidity function name.
    abi: string                       # Parameter type signature.
    mapping: object                   # Param name → value source mapping.
    value: string | null              # ETH value for payable functions.
    
    # Optional: Pre-authorization
    pre_authorize:
      method: "approve" | "permit" | "permit2"
      token: string                   # Param reference or address.
      spender: string                 # Contract name that needs approval.
      amount: string                  # Amount expression.
```

### Pre-Authorization Methods

| Method | Description | Spender |
|--------|-------------|---------|
| `approve` | Standard ERC20 approve tx | Target contract |
| `permit` | EIP-2612 signature | Target contract |
| `permit2` | Uniswap Permit2 signature | Permit2 contract, then target |

**Permit2 Flow:**
1. Check Permit2 allowance for token
2. If insufficient, approve Permit2 contract
3. Sign Permit2 message for spender
4. Include signature in transaction

---

## 4. `evm_multicall` — EVM Batched Write (Optional)

For protocols supporting atomic multi-call patterns (e.g., Uniswap Universal Router).

```yaml
execution:
  "eip155:*":
    type: evm_multicall
    contract: string                  # Router with multicall support.
    calls:
      - function: string
        abi: string
        mapping: object
        condition: string             # Optional skip condition.
    deadline: string
```

---

## 5. `composite` — Multi-Step Execution

For actions requiring multiple sequential steps (e.g., approve → swap).

```yaml
execution:
  "eip155:*":
    type: composite
    steps:
      - id: string                    # Step identifier for references.
        type: evm_call | evm_read     # Step execution type.
        description: string           # Human-readable description.
        contract: string
        function: string
        abi: string
        mapping: object
        condition: string             # Optional. CEL expression to skip step.
```

**Condition Evaluation:**
- Conditions reference `query.*` outputs and `calculated.*` fields
- If condition evaluates to `false`, step is skipped
- Conditions cannot reference runtime transaction results

**Example:**

```yaml
execution:
  "eip155:*":
    type: composite
    steps:
      - id: approve_if_needed
        type: evm_call
        description: "Approve router to spend token_in if allowance insufficient"
        contract: "params.token_in.address"
        function: "approve"
        abi: "(address,uint256)"
        mapping:
          spender: "contracts.router"
          amount: "calculated.approval_amount_atomic"
        condition: |
          query.allowance-token-in.allowance_atomic < calculated.amount_in_atomic

      - id: swap
        type: evm_call
        description: "Execute exactInputSingle swap"
        contract: "router"
        function: "exactInputSingle"
        abi: "(address,address,uint24,address,uint256,uint256,uint160)"
        mapping:
          tokenIn: "params.token_in.address"
          tokenOut: "params.token_out.address"
          fee: "calculated.fee_tier"
          recipient: "calculated.recipient"
          amountIn: "calculated.amount_in_atomic"
          amountOutMinimum: "calculated.min_out_atomic"
          sqrtPriceLimitX96: "0"
        deadline: "calculated.deadline_unix"
```

---

## Mapping Syntax

Values in `mapping` can be:

| Value | Meaning |
|-------|---------|
| `"params.<name>"` | From action/query params |
| `"params.<name>.address"` | Address field from asset param |
| `"ctx.wallet_address"` | Signer's address |
| `"ctx.chain_id"` | Current chain ID |
| `"ctx.now"` | Current timestamp |
| `"calculated.<field>"` | From calculated_fields |
| `"query.<id>.<field>"` | From query output |
| `"contracts.<name>"` | From deployment contracts |
| `"0"` / `"0x0..."` | Literal value |
| `"to_atomic(...)"` | CEL conversion function |
| `{ detect: ... }` | Structured detection (see below) |

### Structured Detection

Replace string `"auto_detect"` with structured object:

```yaml
mapping:
  fee:
    detect:
      kind: "best_quote"
      provider: "uniswap_pool_query"
      candidates: [100, 500, 3000, 10000]
      constraints:
        prefer_liquidity: true
```

---

## 6. `solana_instruction` — Solana

```yaml
execution:
  "solana:*":
    type: solana_instruction
    program: string                   # Program ID (base58).
    instruction: string               # Instruction name (from IDL).
    idl: string                       # Optional. IPFS URI to Anchor IDL.
    discriminator: string             # Optional. 8-byte hex if no IDL.
    
    accounts:                         # Required. All accounts the instruction touches.
      - name: string
        signer: boolean
        writable: boolean
        source: string                # wallet | params.* | constant:* | derived
        derived: "ata" | "pda" | null
        seeds: [string]               # PDA seeds.
        program: string               # Deriving program.
    
    mapping: object
    compute_units: integer
    lookup_tables: [string]
```

**Account source values:**

| Source | Meaning |
|--------|---------|
| `"wallet"` | Signer's public key |
| `"params.<name>"` | From action params |
| `"constant:<address>"` | Fixed address |
| `"derived"` | Computed from seeds |

**Derived account types:**
- `ata` — Associated Token Account
- `pda` — Program Derived Address

---

## 7. `cosmos_message` — Cosmos SDK Chains

```yaml
execution:
  "cosmos:osmosis-1":
    type: cosmos_message
    msg_type: string                  # Protobuf message type path.
    mapping: object
    gas_estimate: integer
    memo: string
```

---

## 8. `bitcoin_psbt` — Bitcoin

```yaml
execution:
  "bip122:*":
    type: bitcoin_psbt
    script_type: "p2wpkh" | "p2tr" | "p2sh" | "p2wsh"
    mapping: object
    op_return: string                 # Optional OP_RETURN data.
    outputs:
      - address: string
        amount: string                # In satoshis.
```

---

## 9. `move_entry` — Move Chains (Aptos/Sui)

```yaml
execution:
  "aptos:1":
    type: move_entry
    module: string                    # Full module path.
    function: string                  # Entry function name.
    type_args: [string]               # Generic type arguments.
    mapping: object
    gas_estimate: integer
```

---

## Engine Requirements

An AIS-compliant engine MUST:

1. **Validate mapping references** — All `params.*` references must exist in the action's params
2. **Resolve derived accounts** — Correctly compute ATAs and PDAs for Solana
3. **Handle `calculated` values** — Evaluate CEL expressions using query outputs
4. **Handle `detect` objects** — Query providers or evaluate candidates to determine values
5. **Evaluate `condition`** — In composite execution, evaluate conditions before each step
6. **Respect gas/CU hints** — Use `gas_estimate` / `compute_units` for transaction construction
7. **Enforce `requires_queries`** — Execute required queries before action execution

An AIS-compliant engine SHOULD:

1. Simulate transactions before signing when possible (eth_call preflight)
2. Cache query results per `cache_ttl`
3. Support multiple specs and choose the most specific chain match
4. Provide clear errors when capabilities are insufficient

---

## Built-in Functions

Engines MUST implement these CEL functions:

| Function | Description |
|----------|-------------|
| `to_atomic(amount, asset)` | Convert human amount to atomic using asset decimals |
| `to_human(atomic, asset)` | Convert atomic amount to human readable |
| `min(a, b)` | Minimum of two values |
| `max(a, b)` | Maximum of two values |
| `abs(x)` | Absolute value |
| `floor(x)` | Floor to integer |
| `ceil(x)` | Ceiling to integer |
| `round(x)` | Round to nearest integer |
