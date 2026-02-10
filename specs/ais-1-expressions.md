# AIS-1E: Expressions & References (ValueRef) — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document defines **ValueRef**, the only allowed mechanism for dynamic values in AIS 0.0.2.

## 1. ValueRef

### 1.1 Motivation

AIS 1.0 overloaded plain strings to mean:

- literal `"0"` vs reference `"params.amount"` vs expression `"floor(a * 0.99)"`

This made validation and execution fragile. AIS 0.0.2 removes ambiguity by requiring structured forms.

### 1.2 Definition

`ValueRef` is a tagged union:

```yaml
{ lit: <literal> }          # literal scalar/object/array
{ ref: "<path>" }           # reference lookup by path
{ cel: "<expression>" }     # CEL expression evaluated against a context
{ detect: <Detect> }        # dynamic resolution via providers/engine plugins
{ object: { <k>: <ValueRef> } }  # structured object of ValueRef (for tuples/structs)
{ array: [ <ValueRef> ] }         # array of ValueRef
```

Notes:
- `object` and `array` exist to avoid mixing “literal object” with “dynamic leafs”.
- Engines MUST reject bare scalars where `ValueRef` is required (no implicit wrapping).

### 1.3 Literal restrictions (numeric safety)

When the target type is an on-chain integer (e.g., EVM `uint256`), engines MUST require:

- decimal string literals in `{lit: "123"}` (not YAML numbers), OR
- values produced by evaluation that are exactly representable as an integer.

See `specs/ais-1-types.md` for the numeric model.

## 2. Reference paths (`{ref:"..."}`)

### 2.1 Namespaces

Protocol Spec (actions/queries) and Execution evaluation MAY reference:

- `params.*` — resolved action/query params
- `ctx.*` — runtime context (wallet, chain, time, policy)
- `query.<id>.*` — query results
- `contracts.*` — deployment contracts resolved for the selected chain
- `calculated.*` — calculated fields
- `policy.*` — active policy constraints (pack/workflow)

Workflow MAY reference:

- `inputs.*`
- `nodes.<id>.outputs.*`
- `nodes.<id>.calculated.*`
- `ctx.*` (workflow runtime context)

Engines MUST define the exact context objects they supply and MUST reject missing paths (unless a field is explicitly nullable).

## 3. CEL (`{cel:"..."}`)

### 3.1 Profile

AIS 0.0.2 uses a restricted CEL profile:

- deterministic, pure (no side effects)
- no reflection/dynamic eval
- no string concatenation to generate addresses, ABI, or function names

### 3.2 Numeric model

CEL numeric values used for on-chain execution MUST be **integer** (BigInt / uint) unless explicitly documented otherwise.

Recommended pattern:

- Convert `token_amount` → atomic `uint` via `to_atomic()`
- Perform slippage math using integer helpers (see Types doc)

## 4. Detect (`{detect:{...}}`)

`detect` is a declaration for engine/provider-driven dynamic resolution.

```yaml
detect:
  kind: "choose_one" | "best_quote" | "best_path" | "protocol_specific"
  provider: "..."                      # provider id (optional for choose_one)
  candidates: [ {lit:...}, ... ]       # candidate values
  constraints: { ... }                 # provider-specific constraints
  requires_capabilities: ["..."]       # required engine capabilities
```

Engines MUST fail with a clear error if the requested detect kind/provider is unsupported.
