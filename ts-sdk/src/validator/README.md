# Validator Module

Validation logic for AIS documents — constraint checking against Pack policies and workflow dependency validation.

## File Structure

| File | Purpose |
|------|---------|
| `constraint.ts` | Validate values against Pack policy constraints (tokens, slippage, approvals) |
| `workflow.ts` | Validate workflow node references, dependencies, and expression bindings |
| `lint.ts` | Best-practice lint rules for AIS documents (pluggable) |
| `plugins.ts` | Validator plugin registry (lint + validate injection) |
| `workspace.ts` | Cross-file validation (workflow→pack→protocol), versions, and enabled detect providers |
| `index.ts` | Re-exports all validators |

## Core API

### Constraint Validation

```ts
import { validateConstraints, getHardConstraints } from '@owliabot/ais-ts-sdk';

// Validate an action against policy constraints
const result = validateConstraints(pack.policy, pack.token_policy, {
  token_address: '0x...',
  token_symbol: 'USDC',
  chain: 'eip155:1',
  slippage_bps: 100,
  unlimited_approval: false,
  risk_level: 3,
});

if (!result.valid) {
  console.log('Violations:', result.violations);
}
if (result.requires_approval) {
  console.log('Needs approval:', result.approval_reasons);
}

// Extract hard constraints for UI display
const limits = getHardConstraints(pack.policy);
// { max_slippage_bps?: number, allow_unlimited_approval?: boolean, ... }
```

### Workflow Validation

```ts
import {
  validateWorkflow,
  validateWorkspaceReferences,
  createValidatorRegistry,
  lintDocument,
  getWorkflowProtocols,
  getWorkflowDependencies,
  getExecutionOrder,
} from '@owliabot/ais-ts-sdk';

// Validate workflow against loaded protocols
const validationResult = validateWorkflow(workflow, resolverContext);
if (!validationResult.valid) {
  for (const issue of validationResult.issues) {
    console.log(`Node ${issue.nodeId}: ${issue.message}`);
  }
}

// Get all protocols referenced by this workflow
const protocols = getWorkflowProtocols(workflow);
// ['uniswap-v3', 'aave-v3']

// Get full action/query references
const deps = getWorkflowDependencies(workflow);
// ['uniswap-v3/swap', 'aave-v3/supply']

// Get execution order (respects deps)
const orderedNodes = getExecutionOrder(workflow);
```

### Workspace Validation (cross-file)

Use this when validating a directory/workspace (multiple files). It checks relationships like:
- `workflow.requires_pack` → existing pack
- pack `includes[]` → existing protocols with matching versions
- workflow node `protocol/action/query` → resolvable protocol/action/query
- detect kinds/providers used by a workflow → enabled in the required pack

```ts
import { loadDirectory, validateWorkspaceReferences } from '@owliabot/ais-ts-sdk';

const dir = await loadDirectory('./examples', { recursive: true });
const issues = validateWorkspaceReferences({
  protocols: dir.protocols,
  packs: dir.packs,
  workflows: dir.workflows,
});
```

### Lint (best practices)

```ts
import { lintDocument } from '@owliabot/ais-ts-sdk';

const issues = lintDocument(doc, { file_path: './examples/foo.ais.yaml' });
```

Built-in reference-format lint rule ids:
- `pack-protocol-ref-format`
- `workflow-node-protocol-ref-format`

### Plugins (custom lint/validate)

```ts
import { createValidatorRegistry, validateWorkflow, lintDocument } from '@owliabot/ais-ts-sdk';

const registry = createValidatorRegistry();
registry.register({
  id: 'my-plugin',
  lint_rules: [
    {
      id: 'require-tags',
      severity: 'warning',
      check: (doc) => (doc.schema === 'ais/0.0.2' && !doc.meta.tags ? [{ rule: 'require-tags', severity: 'warning', message: 'Add tags' }] : []),
    },
  ],
  validate_workflow: (wf) => (wf.nodes.length === 0 ? [{ nodeId: '(workflow)', field: 'nodes', message: 'empty workflow' }] : []),
});

const validation = validateWorkflow(workflow, resolverContext, { registry, enforce_imports: true });
const lint = lintDocument(workflow, { registry });
```

## Types

### ConstraintInput

Input values to validate against policy:

```ts
interface ConstraintInput {
  token_address?: string;      // Token contract address
  token_symbol?: string;       // Token symbol
  chain?: string;              // Chain ID (CAIP-2)
  spend_amount?: string;       // Amount being spent
  slippage_bps?: number;       // Slippage in basis points
  unlimited_approval?: boolean; // Whether approval is unlimited
  risk_level?: number;         // Risk level (1-5)
  risk_tags?: string[];        // Risk tags for the action
}
```

### ConstraintResult

```ts
interface ConstraintResult {
  valid: boolean;              // True if no hard violations
  violations: ConstraintViolation[]; // Hard constraint failures
  requires_approval: boolean;  // True if user approval needed
  approval_reasons: string[];  // Why approval is required
}
```

### WorkflowValidationResult

```ts
interface WorkflowValidationResult {
  valid: boolean;
  issues: WorkflowIssue[];  // { nodeId, field, message, reference? }
}
```

## Validation Checks

### Constraint Validation

1. **Token allowlist** — Is token in allowed list? (strict or soft mode)
2. **Slippage limits** — Does slippage exceed `max_slippage_bps`?
3. **Approval restrictions** — Are unlimited approvals allowed?
4. **Risk thresholds** — Does risk level require manual approval?
5. **Risk tags** — Do any tags trigger approval requirement?

### Workflow Validation

1. **Protocol existence** — Do all `protocol` references resolve?
2. **Action/query existence** — Do referenced actions/queries exist in protocol?
3. **Chain presence** — Does each node resolve a chain via `nodes[].chain` or `workflow.default_chain`?
4. **Ref binding** — Do `ValueRef` paths like `{ ref: "inputs.x" }` match declared inputs?
5. **Node references** — Do `ValueRef` paths like `{ ref: "nodes.x.outputs.y" }` point to existing nodes?
6. **deps (DAG)** — Are all `deps` valid node ids, and is the dependency graph acyclic?
7. **Import policy** — In strict mode (`enforce_imports: true`), workspace-scanned protocols must be listed under `workflow.imports.protocols[]` unless source is builtin/manual/import.

Note:
- `nodes[].until` is evaluated *after* the node runs, so it may reference its own outputs (e.g. `nodes.wait.outputs.*`).
- `nodes[].assert` is also post-execution; it evaluates once and fails fast when falsy.

## Dependencies

- `schema/` — Type definitions for Policy, TokenPolicy, Workflow
- `resolver/` — Context and reference resolution for workflow validation
