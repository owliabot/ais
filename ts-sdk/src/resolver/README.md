# Resolver

Reference resolution and ValueRef evaluation for AIS documents. Handles protocol lookups, action/query references, `${...}` placeholder substitution (optional), and `ValueRef` evaluation (`{lit|ref|cel|detect|object|array}`).

## File Structure

- `index.ts` — Module entry point; re-exports all resolver functions
- `context.ts` — `ResolverContext` state container for loaded protocols and runtime variables
- `reference.ts` — Protocol, action, and query reference resolution
- `expression.ts` — `${...}` placeholder parsing and substitution
- `value-ref.ts` — `ValueRef` evaluation (`lit/ref/cel/detect/object/array`)

## Core API

### Context Management (`context.ts`)

```ts
createContext(): ResolverContext
getRuntimeRoot(ctx): Record<string, unknown>
getRef(ctx, refPath): unknown
setRef(ctx, refPath, value): void
setQueryResult(ctx, queryId, result): void
setNodeOutputs(ctx, nodeId, outputs, options?): void
```

The `ResolverContext` holds:
- `protocols` — Map of loaded protocol specs by name
- `runtime` — Structured runtime state: `inputs/params/ctx/query/contracts/calculated/policy/nodes`

### Reference Resolution (`reference.ts`)

```ts
registerProtocol(ctx, spec): void          // Add a protocol to context
parseSkillRef(ref): { protocol, version? } // Parse "name@version" format
resolveProtocolRef(ctx, ref): ProtocolSpec | null
resolveAction(ctx, ref): { protocol, actionId, action } | null
resolveQuery(ctx, ref): { protocol, queryId, query } | null
expandPack(ctx, pack): { protocols, missing }
getContractAddress(spec, chain, name): string | null
getSupportedChains(spec): string[]
```

### Expression Resolution (`expression.ts`)

```ts
hasExpressions(str): boolean               // Check for ${...} placeholders
extractExpressions(str): string[]          // List all placeholders
resolveExpression(expr, ctx): unknown      // Resolve single expression
resolveExpressionString(template, ctx): string
resolveExpressionObject(obj, ctx): Record<string, unknown>
```

### ValueRef Evaluation (`value-ref.ts`)

```ts
evaluateValueRef(valueRef, ctx, options?): unknown
evaluateValueRefAsync(valueRef, ctx, options?): Promise<unknown>
```

Notes:
- `{ref:"..."}` uses the same dot-path resolution as `resolveExpression()`.
- `{cel:"..."}` runs CEL against the runtime root object (plus optional `root_overrides`). Numeric values are evaluated using AIS 0.0.2 numeric rules (integers as `bigint`, decimals as exact decimals; JS `number` is rejected on execution-critical paths).
- `{detect:{...}}` requires a `DetectResolver` unless `kind: choose_one` with `candidates`.

## Usage Example

```ts
import { createContext, registerProtocol, setRef, resolveAction, resolveExpressionString } from './resolver';
import { parseAisFile } from '../index.js';

// 1. Create context and load protocols
const ctx = createContext();
const uniswap = await parseAisFile('protocols/uniswap-v3.ais.yaml');
registerProtocol(ctx, uniswap);

// 2. Resolve references
const result = resolveAction(ctx, 'uniswap-v3/swap_exact_in');
if (result) {
  console.log(result.action.description);
}

// 3. Set runtime variables (structured)
setRef(ctx, 'inputs.amount', '1000000000000000000');
setRef(ctx, 'ctx.sender', '0x1234...');

// 4. Resolve expressions
const template = 'Swapping ${inputs.amount} from ${ctx.sender}';
const resolved = resolveExpressionString(template, ctx);
// → "Swapping 1000000000000000000 from 0x1234..."
```

## Implementation Details

### Reference Format

| Format | Example | Description |
|--------|---------|-------------|
| Protocol | `uniswap-v3` | Latest version of protocol |
| Protocol@version | `uniswap-v3@1.0.0` | Specific version |
| Action | `uniswap-v3/swap_exact_in` | Protocol + action ID |
| Query | `aave-v3/get_user_data` | Protocol + query ID |

### Expression Namespaces

Expressions follow the pattern `${namespace.path}`:

| Namespace | Example | Description |
|-----------|---------|-------------|
| `inputs` | `${inputs.token_in}` | Workflow input parameter |
| `nodes` | `${nodes.step1.outputs.amount_out}` | Previous node output |
| `ctx` | `${ctx.sender}` | Execution context (chain, sender) |
| `query` | `${query.quote.amountOut}` | Query results (by query id) |

### Pack Expansion

`expandPack()` resolves all skill includes in a Pack and returns:
- `protocols` — Successfully resolved protocol specs
- `missing` — Unresolved references (for error reporting)

## Dependencies

- **schema** — Protocol, Action, Query, Pack type definitions
