# Resolver

Reference resolution and expression evaluation for AIS documents. Handles protocol lookups, action/query references, and `${...}` placeholder substitution.

## File Structure

- `index.ts` — Module entry point; re-exports all resolver functions
- `context.ts` — `ResolverContext` state container for loaded protocols and runtime variables
- `reference.ts` — Protocol, action, and query reference resolution
- `expression.ts` — `${...}` placeholder parsing and substitution

## Core API

### Context Management (`context.ts`)

```ts
createContext(): ResolverContext
setVariable(ctx, key, value): void
setQueryResult(ctx, queryName, result): void
```

The `ResolverContext` holds:
- `protocols` — Map of loaded protocol specs by name
- `variables` — Runtime variables (inputs, node outputs, context)
- `queryResults` — Cached query results

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

## Usage Example

```ts
import { createContext, registerProtocol, setVariable, resolveAction, resolveExpressionString } from './resolver';
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

// 3. Set runtime variables
setVariable(ctx, 'inputs.amount', '1000000000000000000');
setVariable(ctx, 'ctx.sender', '0x1234...');

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
| `query` | `${query.pool_data.fee}` | Cached query result (legacy) |

### Pack Expansion

`expandPack()` resolves all skill includes in a Pack and returns:
- `protocols` — Successfully resolved protocol specs
- `missing` — Unresolved references (for error reporting)

## Dependencies

- **schema** — Protocol, Action, Query, Pack type definitions
