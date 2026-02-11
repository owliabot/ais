# Builder Module

Fluent DSL for programmatically constructing AIS documents (Protocol Specs, Packs, Workflows) with full TypeScript type safety.

## File Structure

| File | Purpose |
|------|---------|
| `base.ts` | Base builder class with common build/serialize methods |
| `protocol.ts` | ProtocolBuilder for creating Protocol Specs |
| `pack.ts` | PackBuilder for creating Packs |
| `workflow.ts` | WorkflowBuilder for creating Workflows |
| `index.ts` | Re-exports builders and helper functions |

## Core API

### Building a Protocol Spec

```ts
import { protocol, param, output } from '@owliabot/ais-ts-sdk';

const uniswap = protocol('uniswap-v3', '0.0.2')
  .name('Uniswap V3')
  .description('Decentralized exchange protocol')
  .homepage('https://uniswap.org')
  .tags('dex', 'amm', 'swap')
  .deployment('eip155:1', {
    router: '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45',
    quoter: '0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6',
  })
  .deployment('eip155:137', {
    router: '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45',
    quoter: '0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6',
  })
  .action('swap', {
    description: 'Swap tokens',
    risk_level: 2,
    risk_tags: ['slippage_risk'],
    params: [
      param('token_in', 'asset', { description: 'Input token' }),
      param('token_out', 'asset', { description: 'Output token' }),
      param('amount_in', 'token_amount', { asset_ref: 'token_in' }),
      param('slippage_bps', 'uint16', { default: 50 }),
    ],
    returns: [
      output('amount_out', 'token_amount'),
    ],
    // ExecutionSpec is AIS 0.0.2 (ValueRef + JSON ABI). See specs/ais-2-evm.md.
    execution: { /* ... */ },
  })
  .query('quote', {
    description: 'Get swap quote',
    params: [
      param('token_in', 'asset'),
      param('token_out', 'asset'),
      param('amount_in', 'token_amount'),
    ],
    returns: [
      output('amount_out', 'token_amount'),
    ],
    execution: { /* ... */ },
  })
  .build();  // Validates and returns ProtocolSpec
```

### Building a Pack

```ts
import { pack } from '@owliabot/ais-ts-sdk';

const defiPack = pack('defi-essentials', '0.0.2')
  .description('Essential DeFi protocols')
  .include('uniswap-v3', '0.0.2')
  .include('aave-v3', '0.0.2')
  .maxSlippage(100)
  .disallowUnlimitedApproval()
  .approvals({ auto_execute_max_risk_level: 2, require_approval_min_risk_level: 3 })
  .build();
```

`PackBuilder.include(...)` appends a `ProtocolInclude` entry to `pack.includes`.

### Building a Workflow

```ts
import { workflow } from '@owliabot/ais-ts-sdk';

const swapFlow = workflow('token-swap', '0.0.3')
  .description('Simple token swap')
  .defaultChain('eip155:1')
  .input('token_in', 'asset', { required: true })
  .input('token_out', 'asset', { required: true })
  .input('amount', 'token_amount', { required: true })
  .query('quote', 'uniswap-v3@0.0.2', 'quote', {
    args: {
      token_in: { ref: 'inputs.token_in' },
      token_out: { ref: 'inputs.token_out' },
      amount_in: { ref: 'inputs.amount' },
    },
  })
  .action('swap', 'uniswap-v3@0.0.2', 'swap', {
    args: {
      token_in: { ref: 'inputs.token_in' },
      token_out: { ref: 'inputs.token_out' },
      amount_in: { ref: 'inputs.amount' },
      min_amount_out: { ref: 'nodes.quote.outputs.amount_out' },
    },
    requires: ['quote'],
  })
  .build();
```

Notes:
- `args` values are `ValueRef` (`{lit|ref|cel|detect|object|array}`).
- Workflow builder emits `schema: "ais-flow/0.0.3"` and uses `nodes[].protocol` (not `skill`).
- Chain selection:
  - Prefer setting `workflow.default_chain` via `.defaultChain(...)`
  - Override per node via `def.chain` in `.node(...)` / `.action(...)` / `.query(...)`
- Polling (engine-driven):
  - Set `def.until` (ValueRef/CEL/ref) to keep re-running a node until the expression becomes truthy
  - Use `def.retry` and `def.timeout_ms` to control polling cadence and limits
- Assertions (engine-driven):
  - Set `def.assert` / `def.assert_message` for post-execution fail-fast checks
- Convenience coercions in `WorkflowBuilder`:
  - Strings like `${inputs.x}` become `{ ref: "inputs.x" }`
  - Other strings become `{ lit: "..." }`
  - `condition: "..."` becomes `{ cel: "..." }` (unless you pass an explicit `{ref:...}`/`{cel:...}`)

## Helper Functions

### `param(name, type, options?)`

Create a parameter definition:

```ts
param('amount', 'uint256')
param('token', 'asset', { description: 'Token address', required: true })
param('slippage', 'uint16', { default: 50, constraints: { max: 1000 } })
```

### `output(name, type, options?)`

Create a return field definition:

```ts
output('amount_out', 'token_amount')
output('success', 'bool', { description: 'Whether swap succeeded' })
```

## Builder Methods

### Common (all builders)

| Method | Description |
|--------|-------------|
| `.build()` | Validate and return the document |
| `.buildUnsafe()` | Return without validation |
| `.toYAML()` | Serialize to YAML string |
| `.toJSON(pretty?)` | Serialize to JSON string |

### ProtocolBuilder

| Method | Description |
|--------|-------------|
| `.description(text)` | Set description |
| `.name(displayName)` | Set display name |
| `.homepage(url)` | Set homepage URL |
| `.maintainer(name)` | Set maintainer |
| `.tags(...tags)` | Add tags |
| `.deployment(chain, contracts)` | Add chain deployment |
| `.action(id, def)` | Define an action |
| `.query(id, def)` | Define a query |
| `.capabilities(...names)` | Declare required capabilities |

### PackBuilder

| Method | Description |
|--------|-------------|
| `.description(text)` | Set description |
| `.include(protocol, version, options?)` | Include a protocol version |
| `.approvals(policy)` | Set approval thresholds |
| `.constraints(defaults)` | Set hard constraint defaults |
| `.maxSlippage(bps)` | Set max slippage |
| `.disallowUnlimitedApproval()` | Disallow unlimited approvals |
| `.tokenResolution(config)` | Set token resolution policy |
| `.allowToken(entry)` | Add token allowlist entry |
| `.quoteProvider(provider, options?)` | Enable quote provider |
| `.routingProviders(...providers)` | Enable routing providers |

### WorkflowBuilder

| Method | Description |
|--------|-------------|
| `.description(text)` | Set description |
| `.input(name, type, options?)` | Declare input parameter |
| `.defaultChain(chain)` | Set `workflow.default_chain` (CAIP-2) |
| `.node(id, def)` | Add workflow node (action/query) |
| `.policy(policy)` | Set workflow policy |

## Implementation Notes

- **Zod validation**: `.build()` validates against schema, throws on error
- **Immutable chaining**: Each method returns `this` for fluent API
- **Type inference**: Full TypeScript autocomplete for all builder methods

## Dependencies

- `schema/` — Zod schemas for validation
- `yaml` — YAML serialization (for `.toYAML()`)
