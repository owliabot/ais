# AIS-2: Execution Types

> Status: Draft
> Version: 1.0.0

## Abstract

AIS-2 defines chain-specific execution formats. Each execution type describes exactly how to build a transaction for a given blockchain architecture.

## Execution Types

### 1. `evm_call` — EVM Chains

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
    
    # Optional
    pre_approve:                      # Token approval if needed.
      token: string                   # Param reference or address.
      spender: string                 # Contract name that needs approval.
      amount: string                  # "params.amount" or "unlimited".
    
    multicall: boolean                # Whether this is part of a multicall batch.
    deadline: string                  # "block.timestamp + 300" etc.
```

**Mapping syntax:**

| Value | Meaning |
|---|---|
| `"params.token_in"` | From action params |
| `"wallet_address"` | Signer's address |
| `"calculated"` | Engine computes (e.g., minOut from quote × slippage) |
| `"auto_detect"` | Engine determines optimal value (e.g., fee tier) |
| `"contracts.router"` | From deployment contracts |
| `"0"` / `"0x0"` | Literal value |
| `"block.timestamp + 300"` | Expression |

**Example:**

```yaml
execution:
  "eip155:*":
    type: evm_call
    contract: "router"
    function: "exactInputSingle"
    abi: "(address,address,uint24,address,uint256,uint256,uint160)"
    pre_approve:
      token: "params.token_in"
      spender: "router"
      amount: "params.amount"
    mapping:
      tokenIn: "params.token_in"
      tokenOut: "params.token_out"
      fee: "auto_detect"
      recipient: "wallet_address"
      amountIn: "params.amount"
      amountOutMinimum: "calculated"
      sqrtPriceLimitX96: "0"
    deadline: "block.timestamp + 600"
```

---

### 2. `solana_instruction` — Solana

```yaml
execution:
  "solana:*":
    type: solana_instruction
    program: string                   # Program ID (base58).
    instruction: string               # Instruction name (from IDL).
    idl: string                       # Optional. IPFS URI to Anchor IDL.
    discriminator: string             # Optional. 8-byte hex if no IDL.
    
    accounts:                         # Required. All accounts the instruction touches.
      - name: string                  # Human-readable name.
        signer: boolean               # Is this a signer?
        writable: boolean             # Is this writable?
        source: string                # Where the address comes from.
        
        # For derived accounts
        derived: "ata" | "pda" | null
        seeds: [string]               # PDA seeds (if derived=pda).
        program: string               # Deriving program (if derived=pda).
    
    mapping: object                   # Param → instruction data mapping.
    compute_units: integer            # Optional. CU estimate.
    lookup_tables: [string]           # Optional. ALT addresses.
```

**Account source values:**

| Source | Meaning |
|---|---|
| `"wallet"` | Signer's public key |
| `"params.token_in"` | From action params |
| `"constant:TokenkegQ..."` | Fixed address |
| `"derived"` | Computed from seeds (see `derived` field) |

**Derived account types:**
- `ata` — Associated Token Account. Engine computes: `getAssociatedTokenAddress(wallet, mint)`
- `pda` — Program Derived Address. Engine computes from `seeds` + `program`

**Example (Jupiter swap):**

```yaml
execution:
  "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp":
    type: solana_instruction
    program: "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"
    instruction: "route"
    idl: "ipfs://QmJupiterIdl..."
    accounts:
      - name: "user"
        signer: true
        writable: true
        source: "wallet"
      - name: "input_mint"
        signer: false
        writable: false
        source: "params.token_in"
      - name: "output_mint"
        signer: false
        writable: false
        source: "params.token_out"
      - name: "input_token_account"
        signer: false
        writable: true
        derived: "ata"
        seeds: ["wallet", "params.token_in"]
      - name: "output_token_account"
        signer: false
        writable: true
        derived: "ata"
        seeds: ["wallet", "params.token_out"]
      - name: "token_program"
        signer: false
        writable: false
        source: "constant:TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    mapping:
      amount_in: "params.amount"
      slippage_bps: "params.slippage_bps"
      route: "auto_detect"
    compute_units: 400000
```

---

### 3. `cosmos_message` — Cosmos SDK Chains

```yaml
execution:
  "cosmos:osmosis-1":
    type: cosmos_message
    msg_type: string                  # Protobuf message type path.
    mapping: object                   # Param → message field mapping.
    gas_estimate: integer             # Optional.
    memo: string                      # Optional. Transaction memo.
```

**Example (Osmosis swap):**

```yaml
execution:
  "cosmos:osmosis-1":
    type: cosmos_message
    msg_type: "/osmosis.gamm.v1beta1.MsgSwapExactAmountIn"
    mapping:
      sender: "wallet_address"
      routes:
        - poolId: "auto_detect"
          tokenOutDenom: "params.token_out"
      tokenIn:
        denom: "params.token_in"
        amount: "params.amount"
      tokenOutMinAmount: "calculated"
    gas_estimate: 250000
```

---

### 4. `bitcoin_psbt` — Bitcoin

Bitcoin doesn't have smart contract calls in the traditional sense, but AIS can describe PSBT construction for specific protocols (Ordinals, Runes, Lightning channel ops).

```yaml
execution:
  "bip122:*":
    type: bitcoin_psbt
    script_type: "p2wpkh" | "p2tr" | "p2sh" | "p2wsh"
    mapping: object
    
    # For Ordinals/Runes
    op_return: string                 # Optional. OP_RETURN data template.
    
    # For simple transfers
    outputs:
      - address: string              # "params.to_address"
        amount: string               # "params.amount" (in sats)
```

---

### 5. `move_entry` — Move Chains (Aptos/Sui)

```yaml
execution:
  "aptos:1":
    type: move_entry
    module: string                    # Full module path: "0x1::coin"
    function: string                  # Entry function name.
    type_args: [string]              # Move generic type arguments.
    mapping: object
    gas_estimate: integer
```

**Example (Aptos token transfer):**

```yaml
execution:
  "aptos:1":
    type: move_entry
    module: "0x1::aptos_account"
    function: "transfer_coins"
    type_args: ["0x1::aptos_coin::AptosCoin"]
    mapping:
      to: "params.to_address"
      amount: "params.amount"
    gas_estimate: 1000
```

---

## Composite Execution

Some actions require multiple steps (e.g., approve → swap). Use `steps` for ordered execution:

```yaml
execution:
  "eip155:*":
    type: composite
    steps:
      - type: evm_call
        description: "Approve router to spend token"
        contract: "token_in"          # Dynamic: resolved from params
        function: "approve"
        abi: "(address,uint256)"
        mapping:
          spender: "contracts.router"
          amount: "params.amount"
        condition: "allowance < params.amount"  # Skip if already approved
        
      - type: evm_call
        description: "Execute swap"
        contract: "router"
        function: "exactInputSingle"
        abi: "..."
        mapping: { ... }
```

## Engine Requirements

An AIS-compliant engine MUST:

1. **Validate mapping references** — All `params.*` references must exist in the action's params.
2. **Resolve derived accounts** — Correctly compute ATAs and PDAs for Solana.
3. **Handle `calculated` values** — Use the protocol's query functions (e.g., quoter) to compute values like minimum output.
4. **Handle `auto_detect` values** — Use heuristics or queries to determine optimal values (e.g., fee tier, route).
5. **Check `condition`** — In composite execution, evaluate conditions before each step.
6. **Respect `compute_units` / `gas_estimate`** — Use as hints for transaction construction.

An AIS-compliant engine SHOULD:

1. Simulate transactions before signing when possible.
2. Cache query results per `cache_ttl`.
3. Support multiple specs for the same action and choose the most specific chain match.
