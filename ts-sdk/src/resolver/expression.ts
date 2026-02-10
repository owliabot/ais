/**
 * Expression resolution - resolve ${...} placeholders and CEL-like references
 */
import type { ResolverContext } from './context.js';
import { getRef } from './context.js';

const EXPR_PATTERN = /\$\{([^}]+)\}/g;

/**
 * Check if a string contains expression placeholders
 */
export function hasExpressions(value: string): boolean {
  return EXPR_PATTERN.test(value);
}

/**
 * Extract all expression references from a string
 */
export function extractExpressions(value: string): string[] {
  const matches: string[] = [];
  const pattern = new RegExp(EXPR_PATTERN.source, 'g');
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(value)) !== null) {
    matches.push(match[1]);
  }
  return matches;
}

/**
 * Resolve a single expression reference
 * Supports:
 * - inputs.param_name - Workflow input parameter
 * - nodes.node_id.outputs.field - Previous node output
 * - ctx.chain - Context chain ID
 * - ctx.sender - Context sender address
 */
export function resolveExpression(expr: string, ctx: ResolverContext): unknown {
  return getRef(ctx, expr);
}

/**
 * Resolve all ${...} expressions in a template string
 */
export function resolveExpressionString(
  template: string,
  ctx: ResolverContext
): string {
  return template.replace(EXPR_PATTERN, (_, expr) => {
    const value = resolveExpression(expr, ctx);
    return value !== undefined ? String(value) : `\${${expr}}`;
  });
}

/**
 * Resolve all expressions in an object recursively
 */
export function resolveExpressionObject(
  obj: Record<string, unknown>,
  ctx: ResolverContext
): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  
  for (const [key, value] of Object.entries(obj)) {
    if (typeof value === 'string') {
      result[key] = hasExpressions(value) 
        ? resolveExpressionString(value, ctx)
        : value;
    } else if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
      result[key] = resolveExpressionObject(value as Record<string, unknown>, ctx);
    } else {
      result[key] = value;
    }
  }
  
  return result;
}
