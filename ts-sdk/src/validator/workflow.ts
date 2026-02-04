/**
 * Workflow validation - validate step references and dependencies
 */
import type { Workflow } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { resolveAction } from '../resolver/index.js';
import { extractExpressions } from '../resolver/expression.js';

export interface WorkflowIssue {
  stepId: string;
  field: string;
  message: string;
  reference?: string;
}

export interface WorkflowValidationResult {
  valid: boolean;
  issues: WorkflowIssue[];
}

/**
 * Validate a workflow against a resolver context
 * Checks:
 * - All `uses` references resolve to known actions
 * - All step references in expressions point to previous steps
 * - Input references match declared inputs
 */
export function validateWorkflow(
  workflow: Workflow,
  ctx: ResolverContext
): WorkflowValidationResult {
  const issues: WorkflowIssue[] = [];
  const declaredInputs = new Set(workflow.inputs.map((i) => i.name));
  const previousSteps = new Set<string>();

  for (const step of workflow.steps) {
    // Check `uses` reference
    const actionResult = resolveAction(ctx, step.uses);
    if (!actionResult) {
      issues.push({
        stepId: step.id,
        field: 'uses',
        message: `Action "${step.uses}" not found`,
        reference: step.uses,
      });
    }

    // Check expressions in `with` values
    for (const [key, value] of Object.entries(step.with)) {
      if (typeof value === 'string') {
        const expressions = extractExpressions(value);
        for (const expr of expressions) {
          const issue = validateExpression(
            expr,
            step.id,
            `with.${key}`,
            declaredInputs,
            previousSteps
          );
          if (issue) issues.push(issue);
        }
      }
    }

    // Check condition expression
    if (step.condition) {
      const expressions = extractExpressions(step.condition);
      for (const expr of expressions) {
        const issue = validateExpression(
          expr,
          step.id,
          'condition',
          declaredInputs,
          previousSteps
        );
        if (issue) issues.push(issue);
      }
    }

    // Add this step to available steps for subsequent steps
    previousSteps.add(step.id);
  }

  return {
    valid: issues.length === 0,
    issues,
  };
}

function validateExpression(
  expr: string,
  stepId: string,
  field: string,
  declaredInputs: Set<string>,
  previousSteps: Set<string>
): WorkflowIssue | null {
  const parts = expr.split('.');
  const namespace = parts[0];

  switch (namespace) {
    case 'input':
    case 'inputs': {
      const inputName = parts[1];
      if (inputName && !declaredInputs.has(inputName)) {
        return {
          stepId,
          field,
          message: `Input "${inputName}" not declared in workflow inputs`,
          reference: expr,
        };
      }
      break;
    }

    case 'step': {
      const refStepId = parts[1];
      if (refStepId && !previousSteps.has(refStepId)) {
        return {
          stepId,
          field,
          message: `Step "${refStepId}" referenced before definition or does not exist`,
          reference: expr,
        };
      }
      break;
    }

    // address and query references are validated at runtime
  }

  return null;
}

/**
 * Get all action references used in a workflow
 */
export function getWorkflowDependencies(workflow: Workflow): string[] {
  return workflow.steps.map((step) => step.uses);
}

/**
 * Get all unique protocols referenced in a workflow
 */
export function getWorkflowProtocols(workflow: Workflow): string[] {
  const protocols = new Set<string>();
  for (const step of workflow.steps) {
    const [protocol] = step.uses.split('/');
    if (protocol) protocols.add(protocol);
  }
  return Array.from(protocols);
}
