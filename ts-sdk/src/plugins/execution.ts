import { z } from 'zod';
import type { ProtocolSpec, ExecutionSpec, ValueRef } from '../schema/index.js';
import { CORE_EXECUTION_TYPES, isCoreExecutionSpec } from '../schema/index.js';

export interface ExecutionTypePlugin<T extends { type: string } = { type: string }> {
  type: string;
  schema: z.ZodType<T>;
  /**
   * Optional hook for readiness/missing-ref detection.
   * When omitted, the planner will use a generic deep ValueRef scan.
   */
  readinessRefsCollector?: (execution: T) => ValueRef[];
}

export class ExecutionTypeRegistry {
  private readonly byType = new Map<string, ExecutionTypePlugin>();

  register(plugin: ExecutionTypePlugin): void {
    if (!plugin.type || typeof plugin.type !== 'string') {
      throw new Error('ExecutionTypePlugin.type must be a non-empty string');
    }
    if ((CORE_EXECUTION_TYPES as readonly string[]).includes(plugin.type)) {
      throw new Error(`Cannot register plugin for core execution type: ${plugin.type}`);
    }
    this.byType.set(plugin.type, plugin);
  }

  get(type: string): ExecutionTypePlugin | null {
    return this.byType.get(type) ?? null;
  }

  list(): ExecutionTypePlugin[] {
    return Array.from(this.byType.values());
  }
}

export const defaultExecutionTypeRegistry = new ExecutionTypeRegistry();

export interface ValidateProtocolExecutionOptions {
  registry: ExecutionTypeRegistry;
  /**
   * Optional label for better error messages (e.g. file path).
   */
  source?: string;
}

export class UnknownExecutionTypeError extends Error {
  constructor(
    message: string,
    public readonly details?: { type: string; path?: string; source?: string }
  ) {
    super(message);
    this.name = 'UnknownExecutionTypeError';
  }
}

export class PluginExecutionSchemaError extends Error {
  constructor(
    message: string,
    public readonly details?: { type: string; path?: string; source?: string; issues?: unknown }
  ) {
    super(message);
    this.name = 'PluginExecutionSchemaError';
  }
}

export function createExecutionTypeRegistry(): ExecutionTypeRegistry {
  return new ExecutionTypeRegistry();
}

export function registerExecutionType(plugin: ExecutionTypePlugin): void {
  defaultExecutionTypeRegistry.register(plugin);
}

export function validateProtocolExecutionTypes(
  spec: ProtocolSpec,
  options: ValidateProtocolExecutionOptions
): void {
  const { registry, source } = options;

  for (const [actionId, action] of Object.entries(spec.actions)) {
    validateExecutionBlock(action.execution, registry, source, `actions.${actionId}.execution`);
  }

  const queries = spec.queries ?? {};
  for (const [queryId, query] of Object.entries(queries)) {
    validateExecutionBlock(query.execution, registry, source, `queries.${queryId}.execution`);
  }
}

function validateExecutionBlock(
  block: Record<string, ExecutionSpec>,
  registry: ExecutionTypeRegistry,
  source: string | undefined,
  basePath: string
): void {
  for (const [chainPattern, execution] of Object.entries(block)) {
    validateExecutionSpec(execution, registry, source, `${basePath}.${chainPattern}`);
  }
}

function validateExecutionSpec(
  execution: ExecutionSpec,
  registry: ExecutionTypeRegistry,
  source: string | undefined,
  path: string
): void {
  // Core types are validated by the core Zod schemas already.
  if (isCoreExecutionSpec(execution)) {
    // Still recurse into composite to validate nested plugin types.
    if (execution.type === 'composite') {
      for (const step of execution.steps) {
        validateExecutionSpec(step.execution as ExecutionSpec, registry, source, `${path}.steps.${step.id}.execution`);
      }
    }
    return;
  }

  if (execution.type === 'composite') {
    // Should never happen: composite is core.
    return;
  }

  const plugin = registry.get(execution.type);
  if (!plugin) {
    throw new UnknownExecutionTypeError(`Unknown execution type "${execution.type}" (register plugin first)`, {
      type: execution.type,
      path,
      source,
    });
  }

  const r = plugin.schema.safeParse(execution);
  if (!r.success) {
    throw new PluginExecutionSchemaError(`Plugin execution schema validation failed for "${execution.type}"`, {
      type: execution.type,
      path,
      source,
      issues: r.error.issues,
    });
  }
}
