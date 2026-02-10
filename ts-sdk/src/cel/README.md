# CEL Module

Common Expression Language (CEL) parser and evaluator for AIS calculated fields and conditions.

## File Structure

| File | Purpose |
|------|---------|
| `lexer.ts` | Tokenizer — converts expression string to tokens |
| `parser.ts` | Parser — builds AST from tokens |
| `numeric.ts` | Numeric model — `int` (`bigint`) + exact `decimal` helpers |
| `evaluator.ts` | Evaluator — executes AST against context, includes builtins |
| `index.ts` | Re-exports all CEL APIs |

## Core API

### Quick Evaluation

```ts
import { evaluateCEL, CELEvaluator } from '@owliabot/ais-ts-sdk';

// One-shot evaluation
const result = evaluateCEL('mul_div(amount, 9950, 10000)', { amount: 1000000n });
// Returns: 995000n

// Reusable evaluator (caches parsed expressions)
const evaluator = new CELEvaluator();
const value = evaluator.evaluate('price > 100 && quantity > 0', {
  price: 150n,
  quantity: 10n,
});
// Returns: true
```

## Types

### CELValue

Valid value types in CEL expressions:

```ts
type CELValue =
  | string
  | bigint                     // AIS integers (execution-critical)
  | { kind: 'decimal'; int: bigint; scale: number } // exact decimals
  | boolean
  | null
  | CELValue[]                    // Lists
  | { [key: string]: CELValue };  // Maps
```

### CELContext

Execution context mapping variable names to values:

```ts
type CELContext = Record<string, CELValue>;

// Example context
const ctx: CELContext = {
  params: { amount: 1000, slippage: 50 },
  query_results: { price: 2500.50 },
  constants: { MAX_SLIPPAGE: 100 },
};
```

### Token

```ts
interface Token {
  type: TokenType;  // 'NUMBER', 'STRING', 'IDENT', 'PLUS', etc.
  value: string | boolean | null; // NUMBER tokens keep the raw lexeme string
  pos: number;      // Position in source
}
```

## Supported Syntax

### Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| Comparison | `==`, `!=`, `<`, `<=`, `>`, `>=` |
| Logical | `&&`, `||`, `!` |
| Membership | `in` |
| Ternary | `? :` |

### Literals

```cel
123           // Integer (evaluates to bigint)
12.34         // Decimal (exact; use string(12.34) to view)
"hello"       // String (double quotes)
'world'       // String (single quotes)
true          // Boolean
false
null
[1, 2, 3]     // List
```

### Member Access

```cel
params.amount           // Dot notation
query_results["price"]  // Bracket notation
items[0]                // List index
```

## Built-in Functions

### String Functions

| Function | Description | Example |
|----------|-------------|---------|
| `size(s)` | String length | `size("hello")` → 5 |
| `contains(s, sub)` | Contains substring | `contains("hello", "ell")` → true |
| `startsWith(s, prefix)` | Starts with | `startsWith("hello", "he")` → true |
| `endsWith(s, suffix)` | Ends with | `endsWith("hello", "lo")` → true |
| `matches(s, regex)` | Regex match | `matches("hello", "^h.*o$")` → true |
| `lower(s)` | Lowercase | `lower("HELLO")` → "hello" |
| `upper(s)` | Uppercase | `upper("hello")` → "HELLO" |
| `trim(s)` | Trim whitespace | `trim("  hi  ")` → "hi" |

### Math Functions

| Function | Description | Example |
|----------|-------------|---------|
| `abs(n)` | Absolute value | `abs(-5)` → 5 |
| `min(a, b, ...)` | Minimum | `min(1, 2, 3)` → 1 |
| `max(a, b, ...)` | Maximum | `max(1, 2, 3)` → 3 |
| `ceil(n)` | Ceiling | `ceil(1.2)` → 2 |
| `floor(n)` | Floor | `floor(1.8)` → 1 |
| `round(n)` | Round | `round(1.5)` → 2 |
| `mul_div(a,b,denom)` | Integer math | `mul_div(quote, 9950, 10000)` |

### Type Functions

| Function | Description | Example |
|----------|-------------|---------|
| `int(v)` | Convert to integer | `int("42")` → 42 |
| `uint(v)` | Convert to unsigned int | `uint(-5)` → 5 |
| `double(v)` | Convert to decimal | `double("3.14")` → 3.14 |
| `string(v)` | Convert to string | `string(42)` → "42" |
| `bool(v)` | Convert to boolean | `bool(1)` → true |
| `type(v)` | Get type name | `type([1,2])` → "list" |

### Collection Functions

| Function | Description | Example |
|----------|-------------|---------|
| `size(list)` | List length | `size([1,2,3])` → 3 |
| `exists(list)` | Any truthy element | `exists([0, 1])` → true |
| `all(list)` | All truthy elements | `all([1, 1])` → true |

### AIS-Specific Functions

| Function | Description |
|----------|-------------|
| `to_atomic(amount, assetOrDecimals)` | Convert decimal amount to atomic `bigint` (no truncation) |
| `to_human(atomic, assetOrDecimals)` | Convert atomic `bigint` to decimal string |
| `mul_div(a,b,denom)` | Safe integer math for slippage bounds |

## Usage in AIS

### Calculated Fields

```yaml
calculated_fields:
  amount_with_slippage:
    expression: "mul_div(params.amount_in_atomic, (10000 - params.slippage_bps), 10000)"
  min_output:
    expression: "mul_div(query.quote.amount_out_atomic, (10000 - params.slippage_bps), 10000)"
```

### Conditions

```yaml
execution:
  eip155:*:
    type: composite
    steps:
      - id: approve
        condition: "query_results.allowance < params.amount"
        ...
```

## Implementation Notes

- **Recursive descent parser**: Clean, readable implementation
- **No external dependencies**: Pure TypeScript
- **Caching**: Evaluator caches parsed ASTs for repeated expressions
- **Error messages**: Include position information for debugging
- **Numeric model (AIS 0.0.2)**: execution-critical math uses `bigint` and exact decimals; JS `number` is rejected for non-integer paths.

## Dependencies

None — standalone module.
