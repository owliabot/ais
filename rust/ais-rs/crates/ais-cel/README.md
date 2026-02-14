# `ais-cel`

CEL lexer/parser/numeric/evaluator for AIS expression evaluation.

## Responsibility

- Tokenize and parse CEL expressions into AST
- Provide exact numeric model (`Decimal` + integer-first arithmetic)
- Evaluate AST against context with expression cache
- Builtin function set used by AIS

## Public entry points

- Parser:
  - `tokenize`
  - `parse_expression`
  - `AstNode`
- Numeric:
  - `Decimal`
  - `NumericError`
- Evaluator:
  - `evaluate_expression`
  - `evaluate_ast`
  - `CelValue`, `CelContext`
  - `CELEvaluator`

## Dependencies

- Independent from `ais-sdk`
- Intended for reuse by `ais-sdk` ValueRef CEL branch and later planner/engine checks
- Uses `num-bigint` for unbounded integer semantics (`CelValue::Integer`)
- Uses `bigdecimal` for decimal representation/ops (`Decimal`)

## Test layout

- Unit tests live in dedicated `*_test.rs` files in `src/`.

## Current status

- Implemented:
  - `AISRS-CEL-001` AST + lexer
  - `AISRS-CEL-002` parser
  - `AISRS-CEL-003` numeric model
  - `AISRS-CEL-004` evaluator + cache
  - `AISRS-CEL-005` builtins
  - Lexer tokenization flow split into focused helper functions (`identifier`/`number`/`string`/`symbol`) for easier maintenance.
  - Integer model migrated from `i128` to `num_bigint::BigInt`; decimal core migrated to `bigdecimal`.
  - `to_atomic`/`to_human` are unified on `Decimal` (`bigdecimal`-backed) conversion with exact atomic scaling checks.
  - Evaluator now coerces numeric strings into numeric values for compare/arithmetic paths, including very large integer strings (BigInt path) to avoid decimal-overflow failures in on-chain balance assertions.
  - `to_human` now supports very large integers with exact decimal scaling and no panic on large values.
- Planned next:
  - integrate into `ais-sdk` `ValueRef::Cel` evaluation path
