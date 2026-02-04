/**
 * Expression resolution - resolve ${...} placeholders
 */
import type { ResolverContext } from './context.js';

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
 * Supports: input.x, query.name.field, step.id.output, address.name
 */
export function resolveExpression(expr: string, ctx: ResolverContext): unknown {
  const parts = expr.split('.');
  const namespace = parts[0];

  switch (namespace) {
    case 'input':
    case 'inputs': {
      const key = parts.slice(1).join('.');
      return ctx.variables[key];
    }

    case 'query': {
      const queryName = parts[1];
      const field = parts.slice(2).join('.');
      const result = ctx.queryResults.get(queryName);
      if (!result) return undefined;
      return field ? result[field] : result;
    }

    case 'step': {
      const stepId = parts[1];
      const output = parts.slice(2).join('.');
      const stepKey = `step.${stepId}`;
      const result = ctx.variables[stepKey] as Record<string, unknown> | undefined;
      if (!result) return undefined;
      return output ? result[output] : result;
    }

    case 'address': {
      const addrName = parts[1];
      for (const spec of ctx.protocols.values()) {
        if (addrName in spec.protocol.addresses) {
          return spec.protocol.addresses[addrName];
        }
      }
      return undefined;
    }

    default:
      return ctx.variables[expr];
  }
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
