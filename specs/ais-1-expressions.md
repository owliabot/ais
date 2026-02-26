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

### 3.3 Allowed usage classes (normative)

CEL in AIS has two allowed usage classes:

- conditional gating:
  - `when.cel`
  - boolean guards inside policy/constraint evaluation
- deterministic computed values:
  - `ValueRef = { cel: "..." }` for parameter/value derivation

For deterministic computed values:

- expression MUST be pure, deterministic, and side-effect free.
- expression MUST NOT read time/random/network/environment.
- expression MUST NOT perform IO or host callbacks.
- same input context MUST produce the same output value.

### 3.4 Prohibited behavior

The following are explicitly prohibited:

- external service calls in CEL
- hidden mutable state
- dynamic code evaluation/reflection
- string-built executable references (for example constructing function/protocol names at runtime)
