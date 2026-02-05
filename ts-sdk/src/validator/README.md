# Validator Module

Validation logic for AIS documents — constraint checking against Pack policies and workflow dependency validation.

## File Structure

| File | Purpose |
|------|---------|
| `constraint.ts` | Validate values against Pack policy constraints (tokens, slippage, approvals) |
| `workflow.ts` | Validate workflow node references, dependencies, and expression bindings |
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

// Get execution order (respects requires_queries)
const orderedNodes = getExecutionOrder(workflow);
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

1. **Protocol existence** — Do all `skill` references resolve?
2. **Action/query existence** — Do referenced actions/queries exist in protocol?
3. **Input binding** — Do `${inputs.x}` references match declared inputs?
4. **Node ordering** — Do `${nodes.x}` references point to earlier nodes?
5. **requires_queries** — Are required nodes defined before usage?

## Dependencies

- `schema/` — Type definitions for Policy, TokenPolicy, Workflow
- `resolver/` — Context and reference resolution for workflow validation
