# @ais-protocol/sdk

TypeScript SDK for parsing and validating AIS (Agent Interaction Specification) files.

## Installation

```bash
npm install @ais-protocol/sdk
```

## Usage

### Parsing Documents

```typescript
import { parseAIS, parseProtocolSpec, parsePack, parseWorkflow } from '@ais-protocol/sdk';

// Parse any AIS document (auto-detects type)
const doc = parseAIS(yamlString);
if (doc.type === 'protocol') {
  console.log(doc.protocol.name);
}

// Parse specific document types
const protocol = parseProtocolSpec(protocolYaml);
const pack = parsePack(packYaml);
const workflow = parseWorkflow(workflowYaml);
```

### Validation

```typescript
import { validate, detectType } from '@ais-protocol/sdk';

// Quick type detection
const type = detectType(yamlString); // 'protocol' | 'pack' | 'workflow' | null

// Validate without parsing
const result = validate(yamlString);
if (!result.valid) {
  console.log(result.issues);
}
```

### Resolving References

```typescript
import {
  createContext,
  registerProtocol,
  resolveAction,
  resolveExpressionString,
  setVariable,
  setQueryResult,
} from '@ais-protocol/sdk';

// Create a resolver context
const ctx = createContext();

// Register protocols
registerProtocol(ctx, protocolSpec);

// Resolve action references
const result = resolveAction(ctx, 'uniswap-v3/swap_exact_in');
console.log(result?.action.method); // 'exactInputSingle'

// Set runtime variables
setVariable(ctx, 'amount', 1000);
setQueryResult(ctx, 'get_pool', { pool: '0x...', fee: 3000 });

// Resolve expression strings
const resolved = resolveExpressionString(
  'Swap ${input.amount} using pool ${query.get_pool.pool}',
  ctx
);
```

## Document Types

### Protocol Spec (`.ais.yaml`)

Defines a single DeFi protocol's interface:

```yaml
ais_version: "1.0"
type: protocol
protocol:
  name: uniswap-v3
  version: "1.0.0"
  chain_id: 1
  addresses:
    router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
queries:
  - name: get_pool
    contract: factory
    method: getPool
    outputs:
      - name: pool
        type: address
actions:
  - name: swap_exact_in
    contract: router
    method: exactInputSingle
    inputs:
      - name: tokenIn
        type: address
      - name: amountIn
        type: uint256
```

### Pack (`.ais-pack.yaml`)

Bundles protocols with security constraints:

```yaml
ais_version: "1.0"
type: pack
pack:
  name: safe-defi
  version: "1.0.0"
protocols:
  - protocol: uniswap-v3
    version: "1.0.0"
constraints:
  slippage:
    max_bps: 50
  require_simulation: true
```

### Workflow (`.ais-flow.yaml`)

Multi-step transaction flows:

```yaml
ais_version: "1.0"
type: workflow
workflow:
  name: swap-to-token
  version: "1.0.0"
inputs:
  - name: target_token
    type: address
steps:
  - id: swap
    uses: uniswap-v3/swap_exact_in
    with:
      token_out: "${input.target_token}"
```

## Expression Syntax

AIS supports expression placeholders in workflows:

- `${input.name}` - Input parameter
- `${query.name.field}` - Query result field
- `${step.id.output}` - Previous step output
- `${address.name}` - Protocol address

## API Reference

### Parsing

- `parseAIS(yaml, options?)` - Parse any AIS document
- `parseProtocolSpec(yaml, options?)` - Parse protocol spec
- `parsePack(yaml, options?)` - Parse pack
- `parseWorkflow(yaml, options?)` - Parse workflow
- `detectType(yaml)` - Detect document type
- `validate(yaml)` - Validate document

### Resolution

- `createContext()` - Create resolver context
- `registerProtocol(ctx, spec)` - Register protocol
- `resolveProtocolRef(ctx, ref)` - Resolve protocol reference
- `resolveAction(ctx, ref)` - Resolve action reference
- `resolveQuery(ctx, ref)` - Resolve query reference
- `expandPack(ctx, pack)` - Expand pack references
- `resolveExpression(expr, ctx)` - Resolve single expression
- `resolveExpressionString(template, ctx)` - Resolve all expressions in string
- `setVariable(ctx, key, value)` - Set runtime variable
- `setQueryResult(ctx, name, result)` - Store query result

### Schemas (Zod)

- `AISDocumentSchema` - Discriminated union of all types
- `ProtocolSpecSchema` - Protocol spec schema
- `PackSchema` - Pack schema
- `WorkflowSchema` - Workflow schema
- `AssetSchema` - Asset type schema
- `TokenAmountSchema` - Token amount schema

## License

MIT
