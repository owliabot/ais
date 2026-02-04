# @owliabot/ais-ts-sdk

TypeScript SDK for parsing and validating AIS (Agent Interaction Specification) files.

## Installation

```bash
npm install @owliabot/ais-ts-sdk
```

## Usage

### Parsing Documents

```typescript
import { parseAIS, parseProtocolSpec, parsePack, parseWorkflow } from '@owliabot/ais-ts-sdk';

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
import { validate, detectType } from '@owliabot/ais-ts-sdk';

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
} from '@owliabot/ais-ts-sdk';

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

### Constraint Validation

```typescript
import { validateConstraints, requiresSimulation } from '@owliabot/ais-ts-sdk';

// Validate against Pack constraints
const result = validateConstraints(pack.constraints, {
  token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
  amount_usd: 5000,
  slippage_bps: 50,
});

if (!result.valid) {
  console.log(result.violations);
  // [{ field: 'slippage_bps', message: 'Slippage 50 bps exceeds max 30 bps', ... }]
}

// Check if simulation is required
if (requiresSimulation(pack.constraints)) {
  // Run simulation before executing
}
```

### Workflow Validation

```typescript
import { validateWorkflow, getWorkflowProtocols } from '@owliabot/ais-ts-sdk';

// Validate workflow references
const result = validateWorkflow(workflow, ctx);
if (!result.valid) {
  console.log(result.issues);
  // [{ stepId: 'step1', field: 'uses', message: 'Action "unknown/action" not found' }]
}

// Get protocols needed by a workflow
const protocols = getWorkflowProtocols(workflow);
// ['uniswap-v3', 'erc20']
```

### File Loading

```typescript
import { loadDirectory, loadDirectoryAsContext } from '@owliabot/ais-ts-sdk';

// Load all AIS files from a directory
const result = await loadDirectory('./protocols');
console.log(result.protocols); // Protocol specs
console.log(result.packs);     // Packs
console.log(result.workflows); // Workflows
console.log(result.errors);    // Parse errors

// Load and create resolver context in one step
const { context, result } = await loadDirectoryAsContext('./protocols');
// context has all protocols registered
```

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

### Validation

- `validateConstraints(constraints, input)` - Validate against Pack constraints
- `requiresSimulation(constraints)` - Check if simulation is required
- `validateWorkflow(workflow, ctx)` - Validate workflow references
- `getWorkflowDependencies(workflow)` - Get all action references
- `getWorkflowProtocols(workflow)` - Get unique protocols used

### File Loading

- `loadFile(path)` - Load any AIS document from file
- `loadProtocol(path)` - Load protocol spec from file
- `loadPack(path)` - Load pack from file
- `loadWorkflow(path)` - Load workflow from file
- `loadDirectory(path, options?)` - Load all AIS files from directory
- `loadDirectoryAsContext(path, options?)` - Load directory and create resolver context

### Schemas (Zod)

- `AISDocumentSchema` - Discriminated union of all types
- `ProtocolSpecSchema` - Protocol spec schema
- `PackSchema` - Pack schema
- `WorkflowSchema` - Workflow schema
- `AssetSchema` - Asset type schema
- `TokenAmountSchema` - Token amount schema

## License

MIT
