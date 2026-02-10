# AIS-1D: Types & Numeric Model — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

## 1. Core types (high-level)

AIS types are intent-level. Execution specs map them to chain-specific types.

### 1.0 Scalar types (MUST)

The following scalar types are used by `params[].type` across AIS documents:

- `address`
- `bool`
- `string`
- `bytes` (dynamic bytes)
- `float` (NOT RECOMMENDED for execution-critical amounts; see numeric model)
- `intN` / `uintN` where `N` is a multiple of 8, `8 <= N <= 256` (e.g., `uint24`, `int128`)
- `bytesN` where `1 <= N <= 32` (e.g., `bytes32`)
- `array<T>` and `tuple<T1,T2,...>` (for structured params; exact mapping depends on execution layer)

### 1.1 `asset`

```yaml
asset:
  chain_id: "eip155:8453"      # CAIP-2
  address: "0x..."             # chain-native address
  symbol: "USDC"               # optional
  decimals: 6                  # optional; engine may fetch if missing
```

### 1.2 `token_amount`

Human-facing amount bound to an `asset` parameter.

- **Value representation:** decimal string (no exponent), e.g. `"1.23"`.
- **Binding:** the *parameter definition* MUST include `asset_ref` pointing to an `asset` param.

Example (parameter definition):

```yaml
params:
  - name: token_in
    type: asset
    description: "Input token"
  - name: amount_in
    type: token_amount
    description: "Human amount for token_in"
    asset_ref: "token_in"
```

## 2. Numeric model (MUST)

### 2.1 No IEEE754 in execution-critical paths

Engines MUST NOT use IEEE754 floating point (`number`) to compute:

- atomic amounts
- allowances/approvals
- minOut/slippage bounds
- deadlines that are encoded on-chain

### 2.2 Integer representation

All on-chain integer values (EVM `uint*`, Solana u64, etc.) are represented as:

- **decimal strings** in YAML (e.g., `"1000000"`), OR
- **BigInt** internally in engines/SDKs.

YAML numeric literals MUST be rejected for on-chain integers (to avoid accidental scientific notation/precision loss).

### 2.2.1 Decimal string format (MUST)

AIS uses **strings** for numeric values in YAML to avoid IEEE754/scientific-notation ambiguity.

Definitions:

- **IntegerString**: `^-?[0-9]+$`
- **DecimalString**: `^-?[0-9]+(\.[0-9]+)?$`

Rules:

- No exponent notation (`e`/`E`) is allowed.
- No leading `+` is allowed.
- No whitespace is allowed.
- A decimal point MUST have at least 1 digit on both sides (so `"1.0"` is valid; `"1."` and `".5"` are invalid).

### 2.3 `to_atomic(amount, asset)` (MUST)

`to_atomic()` converts a human decimal string to an integer atomic amount.

Rules:

- Input `amount` MUST be a **non-negative** `DecimalString` (no exponent). (Sentinels like `"max"` are not numeric and MUST be handled by protocol logic before calling `to_atomic()`.)
- `asset.decimals` MUST be known; if unknown, engine MUST fail (unless a fetch mechanism is explicitly enabled and succeeds).
- Conversion is exact: `atomic = amount * 10^decimals` with truncation disallowed (engine MUST fail if more fractional digits than decimals).

Additional requirements:

- `decimals` MUST be an integer in range `0..77` (inclusive).
- `to_atomic()` MUST fail if:
  - `amount` is negative
  - `amount` is not a `DecimalString`
  - `amount` has more fractional digits than `decimals` (truncation is disallowed)

Examples (normative):

- `to_atomic("1.23", 6) = 1230000`
- `to_atomic("0.00000001", 8) = 1`
- `to_atomic("1.234", 2)` MUST fail (too many fractional digits)

### 2.4 `to_human(atomic, asset)` (MUST)

`to_human()` converts an integer atomic amount into a human-facing decimal string.

Rules:

- Input `atomic` MUST be a **non-negative integer** (BigInt or `IntegerString`).
- `decimals` MUST be an integer in range `0..77` (inclusive).
- Output MUST be a non-negative `DecimalString` in **canonical form**:
  - no trailing zeros in the fractional part
  - no decimal point if the fractional part becomes empty
  - `"0"` for zero

Examples (normative):

- `to_human(1230000, 6) = "1.23"`
- `to_human(1, 6) = "0.000001"`
- `to_human(1000000, 6) = "1"`

### 2.5 `mul_div(a, b, denom)` (MUST)

`mul_div()` performs integer math: `floor((a * b) / denom)` for **non-negative integers**.

Rules:

- Inputs `a`, `b`, `denom` MUST be non-negative integers (BigInt or `IntegerString`).
- `denom` MUST be `> 0`.
- Result MUST be `floor((a * b) / denom)`.
- Implementations MUST fail on:
  - `denom = 0`
  - non-integer inputs (e.g., decimal strings)
  - negative inputs

Notes:

- Implementations that target fixed-width chain integers (e.g. `uint256`) MUST still validate that the *final encoded value* fits the target type (this is an execution-layer concern, not a numeric-model concern).

### 2.6 Integer slippage math (recommended)

Prefer integer formulas over float factors, e.g.:

- `min_out = mul_div(quote_out, (10000 - slippage_bps), 10000)`

Engines SHOULD provide `mul_div(a,b,denom)` as a builtin for safe integer math.
