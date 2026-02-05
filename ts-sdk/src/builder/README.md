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

const uniswap = protocol('uniswap-v3', '1.0.0')
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
    execution: {
      'eip155:*': {
        type: 'evm_call',
        contract: 'router',
        method: 'exactInputSingle',
        params: [/* ... */],
      },
    },
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
    execution: {
      'eip155:*': {
        type: 'evm_read',
        contract: 'quoter',
        method: 'quoteExactInputSingle',
        params: [/* ... */],
      },
    },
  })
  .build();  // Validates and returns ProtocolSpec
```

### Building a Pack

```ts
import { pack } from '@owliabot/ais-ts-sdk';

const defiPack = pack('defi-essentials', '1.0.0')
  .description('Essential DeFi protocols')
  .include('uniswap-v3', { version: '>=1.0.0' })
  .include('aave-v3', { version: '>=1.0.0' })
  .tokenAllowlist([
    { symbol: 'USDC', address: '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', chain: 'eip155:1' },
    { symbol: 'WETH', address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2', chain: 'eip155:1' },
  ])
  .hardConstraints({
    max_slippage_bps: 100,
    allow_unlimited_approval: false,
  })
  .approvalPolicy({
    auto_execute_max_risk_level: 2,
    require_approval_min_risk_level: 3,
  })
  .build();
```

### Building a Workflow

```ts
import { workflow } from '@owliabot/ais-ts-sdk';

const swapFlow = workflow('token-swap', '1.0.0')
  .description('Simple token swap')
  .input('token_in', 'asset', { required: true })
  .input('token_out', 'asset', { required: true })
  .input('amount', 'token_amount', { required: true })
  .node({
    id: 'quote',
    skill: 'uniswap-v3',
    type: 'query_ref',
    query: 'quote',
    args: {
      token_in: '${inputs.token_in}',
      token_out: '${inputs.token_out}',
      amount_in: '${inputs.amount}',
    },
  })
  .node({
    id: 'swap',
    skill: 'uniswap-v3',
    type: 'action_ref',
    action: 'swap',
    args: {
      token_in: '${inputs.token_in}',
      token_out: '${inputs.token_out}',
      amount_in: '${inputs.amount}',
      min_amount_out: '${nodes.quote.result.amount_out}',
    },
    requires_queries: ['quote'],
  })
  .build();
```

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
| `.capability(name)` | Declare required capability |

### PackBuilder

| Method | Description |
|--------|-------------|
| `.description(text)` | Set description |
| `.include(protocol, options?)` | Include a protocol |
| `.tokenAllowlist(tokens)` | Set allowed tokens |
| `.hardConstraints(constraints)` | Set hard constraints |
| `.approvalPolicy(policy)` | Set approval thresholds |
| `.override(protocol, action, overrides)` | Override action settings |

### WorkflowBuilder

| Method | Description |
|--------|-------------|
| `.description(text)` | Set description |
| `.input(name, type, options?)` | Declare input parameter |
| `.node(nodeDef)` | Add workflow node |
| `.policy(policy)` | Set workflow policy |

## Implementation Notes

- **Zod validation**: `.build()` validates against schema, throws on error
- **Immutable chaining**: Each method returns `this` for fluent API
- **Type inference**: Full TypeScript autocomplete for all builder methods

## Dependencies

- `schema/` — Zod schemas for validation
- `yaml` — YAML serialization (for `.toYAML()`)
