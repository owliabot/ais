# TypeScript SDK API (`@owliabot/ais-ts-sdk`)

Source: `/home/ocbot/.openclaw/workspace/repos/ais/ts-sdk/README.md`

## Installation and CLI

```bash
npm install @owliabot/ais-ts-sdk
npm install -g @owliabot/ais-ts-sdk
```

CLI commands:
- `ais validate` - Validate AIS files against schema
- `ais lint` - Lint for best practices/common issues
- `ais check` - Run validate + lint + workflow checks
- `ais help` - Show help
- `ais version` - Show version

Examples:

```bash
ais validate ./protocols/
ais validate myprotocol.ais.yaml
ais lint ./specs/
ais check . --recursive
ais validate ./specs/ --json
ais check . --quiet
```

## Main API Surface

### Parsing

- `parseAIS(yaml, options?)`
- `parseProtocolSpec(yaml, options?)`
- `parsePack(yaml, options?)`
- `parseWorkflow(yaml, options?)`
- `detectType(yaml)`
- `validate(yaml)`

Typical import:

```typescript
import { parseAIS, parseProtocolSpec, parsePack, parseWorkflow } from '@owliabot/ais-ts-sdk';
```

### Resolution

- `createContext()`
- `registerProtocol(ctx, spec)`
- `resolveProtocolRef(ctx, ref)`
- `resolveAction(ctx, ref)`
- `resolveQuery(ctx, ref)`
- `expandPack(ctx, pack)`
- `resolveExpression(expr, ctx)`
- `resolveExpressionString(template, ctx)`
- `setVariable(ctx, key, value)`
- `setQueryResult(ctx, name, result)`

### Validation helpers

- `validateConstraints(constraints, input)`
- `requiresSimulation(constraints)`
- `validateWorkflow(workflow, ctx)`
- `getWorkflowDependencies(workflow)`
- `getWorkflowProtocols(workflow)`

### File loading

- `loadFile(path)`
- `loadProtocol(path)`
- `loadPack(path)`
- `loadWorkflow(path)`
- `loadDirectory(path, options?)`
- `loadDirectoryAsContext(path, options?)`

### Execution

- `buildTransaction(protocol, action, inputs, ctx, options)`
- `buildQuery(protocol, query, inputs, ctx, options)`
- `buildWorkflowTransactions(protocols, nodes, ctx, chain)`
- `encodeFunctionCall(signature, types, values)`
- `encodeFunctionSelector(signature)`
- `keccak256(input)`

### Builder DSL

- `protocol(name, version)`
- `pack(name, version)`
- `workflow(name, version)`
- `param(name, type, options?)`
- `output(name, type, options?)`
- `.build()`
- `.toYAML()`
- `.toJSON()`

### CEL evaluator

- `evaluateCEL(expression, context?)`
- `CELEvaluator`
- `CELLexer`
- `CELParser`

Supported expression features include arithmetic, comparisons, logical operators, ternary, member access, `in`, and built-ins such as `size`, `contains`, `startsWith`, `endsWith`, `matches`, `lower`, `upper`, `trim`, `int`, `string`, `type`, `abs`, `min`, `max`.

### Schemas (Zod)

- `AISDocumentSchema`
- `ProtocolSpecSchema`
- `PackSchema`
- `WorkflowSchema`
- `AssetSchema`
- `TokenAmountSchema`

## Document Types and Patterns

From the README examples:
- Protocol specs: `.ais.yaml`
- Packs: `.ais-pack.yaml`
- Workflows: `.ais-flow.yaml`

Expression placeholders shown in workflow templating:
- `${input.name}`
- `${query.name.field}`
- `${step.id.output}`
- `${address.name}`

## Example Integration Flow

```typescript
import {
  parseProtocolSpec,
  parsePack,
  parseWorkflow,
  validate,
  createContext,
  registerProtocol,
  resolveAction,
  buildTransaction,
} from '@owliabot/ais-ts-sdk';

const protocol = parseProtocolSpec(protocolYaml);
const pack = parsePack(packYaml);
const workflow = parseWorkflow(workflowYaml);

const check = validate(protocolYaml);
if (!check.valid) throw new Error(JSON.stringify(check.issues));

const ctx = createContext();
registerProtocol(ctx, protocol);

const action = resolveAction(ctx, 'uniswap-v3/swap_exact_in');
if (!action) throw new Error('Action not found');

const tx = buildTransaction(protocol, action.action, {
  amountIn: 1000n,
  amountOutMin: 990n,
}, ctx, { chain: 'eip155:1' });
```
