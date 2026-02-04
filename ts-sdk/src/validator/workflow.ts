/**
 * Workflow validation - validate node references and dependencies
 */
import type { Workflow, WorkflowNode } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { resolveAction, resolveQuery, parseSkillRef } from '../resolver/index.js';
import { extractExpressions } from '../resolver/expression.js';

export interface WorkflowIssue {
  nodeId: string;
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
 * - All skill references resolve to known protocols
 * - All action/query references exist in the protocol
 * - All node references in expressions point to previous nodes
 * - Input references match declared inputs
 */
export function validateWorkflow(
  workflow: Workflow,
  ctx: ResolverContext
): WorkflowValidationResult {
  const issues: WorkflowIssue[] = [];
  const declaredInputs = new Set(
    workflow.inputs ? Object.keys(workflow.inputs) : []
  );
  const previousNodes = new Set<string>();

  for (const node of workflow.nodes) {
    // Check skill reference exists
    const { protocol } = parseSkillRef(node.skill);
    if (!ctx.protocols.has(protocol)) {
      issues.push({
        nodeId: node.id,
        field: 'skill',
        message: `Protocol "${protocol}" not found`,
        reference: node.skill,
      });
    } else {
      // Check action/query reference
      if (node.type === 'action_ref' && node.action) {
        const actionRef = `${node.skill}/${node.action}`;
        const actionResult = resolveAction(ctx, actionRef);
        if (!actionResult) {
          issues.push({
            nodeId: node.id,
            field: 'action',
            message: `Action "${node.action}" not found in ${node.skill}`,
            reference: actionRef,
          });
        }
      }

      if (node.type === 'query_ref' && node.query) {
        const queryRef = `${node.skill}/${node.query}`;
        const queryResult = resolveQuery(ctx, queryRef);
        if (!queryResult) {
          issues.push({
            nodeId: node.id,
            field: 'query',
            message: `Query "${node.query}" not found in ${node.skill}`,
            reference: queryRef,
          });
        }
      }
    }

    // Check expressions in args
    if (node.args) {
      for (const [key, value] of Object.entries(node.args)) {
        if (typeof value === 'string') {
          const expressions = extractExpressions(value);
          for (const expr of expressions) {
            const issue = validateExpression(
              expr,
              node.id,
              `args.${key}`,
              declaredInputs,
              previousNodes
            );
            if (issue) issues.push(issue);
          }
        }
      }
    }

    // Check condition expression
    if (node.condition) {
      const expressions = extractExpressions(node.condition);
      for (const expr of expressions) {
        const issue = validateExpression(
          expr,
          node.id,
          'condition',
          declaredInputs,
          previousNodes
        );
        if (issue) issues.push(issue);
      }
    }

    // Check requires_queries references
    if (node.requires_queries) {
      for (const reqNode of node.requires_queries) {
        if (!previousNodes.has(reqNode)) {
          issues.push({
            nodeId: node.id,
            field: 'requires_queries',
            message: `Required node "${reqNode}" not defined before this node`,
            reference: reqNode,
          });
        }
      }
    }

    // Add this node to available nodes for subsequent nodes
    previousNodes.add(node.id);
  }

  return {
    valid: issues.length === 0,
    issues,
  };
}

function validateExpression(
  expr: string,
  nodeId: string,
  field: string,
  declaredInputs: Set<string>,
  previousNodes: Set<string>
): WorkflowIssue | null {
  const parts = expr.split('.');
  const namespace = parts[0];

  switch (namespace) {
    case 'inputs': {
      const inputName = parts[1];
      if (inputName && !declaredInputs.has(inputName)) {
        return {
          nodeId,
          field,
          message: `Input "${inputName}" not declared in workflow inputs`,
          reference: expr,
        };
      }
      break;
    }

    case 'nodes': {
      const refNodeId = parts[1];
      if (refNodeId && !previousNodes.has(refNodeId)) {
        return {
          nodeId,
          field,
          message: `Node "${refNodeId}" referenced before definition or does not exist`,
          reference: expr,
        };
      }
      break;
    }

    // ctx references are validated at runtime
  }

  return null;
}

/**
 * Get all skill references used in a workflow
 */
export function getWorkflowDependencies(workflow: Workflow): string[] {
  return workflow.nodes.map((node) => {
    if (node.type === 'action_ref' && node.action) {
      return `${node.skill}/${node.action}`;
    }
    if (node.type === 'query_ref' && node.query) {
      return `${node.skill}/${node.query}`;
    }
    return node.skill;
  });
}

/**
 * Get all unique protocols referenced in a workflow
 */
export function getWorkflowProtocols(workflow: Workflow): string[] {
  const protocols = new Set<string>();
  for (const node of workflow.nodes) {
    const { protocol } = parseSkillRef(node.skill);
    protocols.add(protocol);
  }
  return Array.from(protocols);
}

/**
 * Get workflow nodes in dependency order
 */
export function getExecutionOrder(workflow: Workflow): WorkflowNode[] {
  // For now, assume nodes are already in order
  // TODO: Implement topological sort based on requires_queries
  return workflow.nodes;
}
