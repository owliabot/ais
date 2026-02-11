import type { ResolverContext } from '../resolver/index.js';
import {
  parseProtocolRef,
  resolveAction,
  resolveQuery,
} from '../resolver/index.js';
/**
 * Workflow validation - validate node references and dependencies
 */
import type {
  Workflow,
  WorkflowNode,
} from '../schema/index.js';
import {
  buildWorkflowDag,
  WorkflowDagError,
} from '../workflow/dag.js';
import type { ValidatorRegistry } from './plugins.js';
import { defaultValidatorRegistry } from './plugins.js';

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
 * - All protocol references resolve to known protocol specs
 * - All action/query references exist in the protocol
 * - All node references in expressions point to previous nodes
 * - Input references match declared inputs
 */
export function validateWorkflow(
  workflow: Workflow,
  ctx: ResolverContext,
  options: { registry?: ValidatorRegistry; enforce_imports?: boolean } = {}
): WorkflowValidationResult {
  const issues: WorkflowIssue[] = [];
  const registry = options.registry ?? defaultValidatorRegistry;
  const enforceImports = options.enforce_imports ?? true;
  const importedProtocols = enforceImports ? collectImportedProtocols(workflow) : new Set<string>();
  const declaredInputs = new Set(
    workflow.inputs ? Object.keys(workflow.inputs) : []
  );
  const nodeIds = workflow.nodes.map((n) => n.id);
  const nodeIdSet = new Set(nodeIds);

  for (const node of workflow.nodes) {
    if (!node.chain && !workflow.default_chain) {
      issues.push({
        nodeId: node.id,
        field: 'chain',
        message: 'Missing chain: set nodes[].chain or workflow.default_chain',
      });
    }

    // Check protocol reference exists (+ version matches the loaded workspace)
    const { protocol, version } = parseProtocolRef(node.protocol);
    const protocolSpec = ctx.protocols.get(protocol);
    const hasVersionMismatch = Boolean(version && protocolSpec && protocolSpec.meta.version !== version);
    if (!protocolSpec) {
      issues.push({
        nodeId: node.id,
        field: 'protocol',
        message: `Protocol "${protocol}" not found`,
        reference: (node as any).protocol,
      });
    } else if (hasVersionMismatch) {
      issues.push({
        nodeId: node.id,
        field: 'protocol',
        message: `Protocol version mismatch: requested ${protocol}@${version}, loaded ${protocol}@${protocolSpec.meta.version}`,
        reference: (node as any).protocol,
      });
    } else if (enforceImports) {
      // Import enforcement:
      // - builtin/manual protocols can be used without explicit import
      // - workspace-loaded protocols must be explicitly imported by workflow
      const src = ctx.protocol_sources.get(protocol) ?? 'manual';
      const protoRef = String((node as any).protocol ?? '');
      const isExplicitlyImported = importedProtocols.has(protoRef);
      const isAllowedWithoutImport = src === 'builtin' || src === 'manual' || src === 'import';
      if (!isExplicitlyImported && !isAllowedWithoutImport) {
        issues.push({
          nodeId: node.id,
          field: 'protocol',
          message: `Protocol "${protoRef}" must be explicitly imported by workflow (imports.protocols) or registered as builtin/manual`,
          reference: protoRef,
        });
      }
    }

    // Check action/query reference whenever protocol lookup succeeded.
    if (protocolSpec && !hasVersionMismatch) {
      if (node.type === 'action_ref' && node.action) {
        const actionRef = `${(node as any).protocol}/${node.action}`;
        const actionResult = resolveAction(ctx, actionRef);
        if (!actionResult) {
          issues.push({
            nodeId: node.id,
            field: 'action',
            message: `Action "${node.action}" not found in ${(node as any).protocol}`,
            reference: actionRef,
          });
        }
      }

      if (node.type === 'query_ref' && node.query) {
        const queryRef = `${(node as any).protocol}/${node.query}`;
        const queryResult = resolveQuery(ctx, queryRef);
        if (!queryResult) {
          issues.push({
            nodeId: node.id,
            field: 'query',
            message: `Query "${node.query}" not found in ${(node as any).protocol}`,
            reference: queryRef,
          });
        }
      }
    }

    // Check expressions in args
    if (node.args) {
      for (const [key, value] of Object.entries(node.args)) {
        validateValueRefLike(value, node.id, `args.${key}`, declaredInputs, nodeIdSet, issues);
      }
    }

    // Check condition expression
    if (node.condition) {
      validateValueRefLike(node.condition, node.id, 'condition', declaredInputs, nodeIdSet, issues);
    }

    // Check assert expression
    if ((node as any).assert) {
      validateValueRefLike((node as any).assert, node.id, 'assert', declaredInputs, nodeIdSet, issues, { allow_self_node_ref: true });
    }

    // Check until expression
    if (node.until) {
      // until is evaluated *after* node execution, so self-reference to `nodes.<id>.outputs.*` is valid.
      validateValueRefLike(node.until, node.id, 'until', declaredInputs, nodeIdSet, issues, { allow_self_node_ref: true });
    }

    // Check calculated_overrides
    if (node.calculated_overrides) {
      for (const [k, ov] of Object.entries(node.calculated_overrides)) {
        validateValueRefLike(ov.expr, node.id, `calculated_overrides.${k}.expr`, declaredInputs, nodeIdSet, issues);
      }
    }

    // Check explicit deps references (existence only; order is defined by DAG)
    if (node.deps) {
      for (const d of node.deps) {
        if (!nodeIdSet.has(d)) {
          issues.push({
            nodeId: node.id,
            field: 'deps',
            message: `Dependency "${d}" does not exist in workflow`,
            reference: d,
          });
        } else if (d === node.id) {
          issues.push({
            nodeId: node.id,
            field: 'deps',
            message: 'Node cannot depend on itself',
            reference: d,
          });
        }
      }
    }
  }

  // Check workflow outputs references
  if (workflow.outputs) {
    for (const [k, v] of Object.entries(workflow.outputs)) {
      validateValueRefLike(v, '(workflow)', `outputs.${k}`, declaredInputs, nodeIdSet, issues);
    }
  }

  // DAG validation (cycles / missing deps implied by refs / duplicates)
  try {
    buildWorkflowDag(workflow, { include_implicit_deps: true });
  } catch (err) {
    if (err instanceof WorkflowDagError) {
      if (err.kind === 'cycle') {
        for (const id of err.cycle ?? []) {
          issues.push({
            nodeId: id,
            field: 'deps',
            message: `Dependency cycle detected: ${(err.cycle ?? []).join(' -> ')}`,
          });
        }
      } else if (err.kind === 'duplicate_node_id') {
        issues.push({
          nodeId: err.nodeId ?? '(workflow)',
          field: 'id',
          message: err.message,
          reference: err.nodeId,
        });
      } else if (err.kind === 'unknown_dep') {
        issues.push({
          nodeId: err.nodeId ?? '(workflow)',
          field: 'deps',
          message: err.message,
          reference: err.depId,
        });
      } else if (err.kind === 'self_dep') {
        issues.push({
          nodeId: err.nodeId ?? '(workflow)',
          field: 'deps',
          message: err.message,
          reference: err.nodeId,
        });
      }
    } else {
      issues.push({
        nodeId: '(workflow)',
        field: 'deps',
        message: (err as Error)?.message ?? 'Unknown workflow DAG error',
      });
    }
  }

  // Plugin validators (normative errors)
  for (const v of registry.listWorkflowValidators()) {
    try {
      issues.push(...(v(workflow, ctx) ?? []));
    } catch (e) {
      issues.push({
        nodeId: '(workflow)',
        field: 'plugin',
        message: `Validator plugin threw: ${(e as Error)?.message ?? String(e)}`,
      });
    }
  }

  return {
    valid: issues.length === 0,
    issues,
  };
}

function collectImportedProtocols(workflow: Workflow): Set<string> {
  const out = new Set<string>();
  const imports: any = (workflow as any).imports;
  const protocols = imports?.protocols;
  if (Array.isArray(protocols)) {
    for (const p of protocols) {
      const proto = p?.protocol;
      if (typeof proto === 'string' && proto.length > 0) out.add(proto);
    }
  }
  return out;
}

function validateValueRefLike(
  value: unknown,
  nodeId: string,
  field: string,
  declaredInputs: Set<string>,
  nodeIdSet: Set<string>,
  issues: WorkflowIssue[],
  options: { allow_self_node_ref?: boolean } = {}
): void {
  const extracted = collectRefPathsAndCel(value);
  const allowSelf = options.allow_self_node_ref === true;

  for (const path of extracted.refPaths) {
    const parts = path.split('.');
    if (parts[0] === 'inputs') {
      const inputName = parts[1];
      if (inputName && !declaredInputs.has(inputName)) {
        issues.push({
          nodeId,
          field,
          message: `Input "${inputName}" not declared in workflow inputs`,
          reference: path,
        });
      }
    } else if (parts[0] === 'nodes') {
      const refNodeId = parts[1];
      if (refNodeId && !nodeIdSet.has(refNodeId)) {
        issues.push({
          nodeId,
          field,
          message: `Node "${refNodeId}" referenced but does not exist in workflow`,
          reference: path,
        });
      }
      if (!allowSelf && refNodeId && refNodeId === nodeId) {
        issues.push({
          nodeId,
          field,
          message: 'Node cannot reference its own outputs',
          reference: path,
        });
      }
    }
  }

  for (const cel of extracted.celExprs) {
    for (const inputName of extractIdsFromCel(cel, 'inputs')) {
      if (!declaredInputs.has(inputName)) {
        issues.push({
          nodeId,
          field,
          message: `Input "${inputName}" not declared in workflow inputs`,
          reference: `inputs.${inputName}`,
        });
      }
    }
    for (const refNodeId of extractIdsFromCel(cel, 'nodes')) {
      if (!nodeIdSet.has(refNodeId)) {
        issues.push({
          nodeId,
          field,
          message: `Node "${refNodeId}" referenced but does not exist in workflow`,
          reference: `nodes.${refNodeId}`,
        });
      }
      if (!allowSelf && refNodeId === nodeId) {
        issues.push({
          nodeId,
          field,
          message: 'Node cannot reference its own outputs',
          reference: `nodes.${refNodeId}`,
        });
      }
    }
  }
}

function collectRefPathsAndCel(value: unknown): { refPaths: string[]; celExprs: string[] } {
  const refPaths: string[] = [];
  const celExprs: string[] = [];

  const visit = (v: unknown) => {
    if (!v || typeof v !== 'object') return;
    const rec = v as Record<string, unknown>;

    if (typeof rec.ref === 'string') refPaths.push(rec.ref);
    if (typeof rec.cel === 'string') celExprs.push(rec.cel);

    if (rec.object && typeof rec.object === 'object' && rec.object !== null) {
      for (const child of Object.values(rec.object as Record<string, unknown>)) visit(child);
    }
    if (Array.isArray(rec.array)) {
      for (const child of rec.array) visit(child);
    }
  };

  visit(value);
  return { refPaths, celExprs };
}

function extractIdsFromCel(cel: string, namespace: 'nodes' | 'inputs'): string[] {
  const out: string[] = [];
  const re =
    namespace === 'nodes'
      ? /\bnodes\.([A-Za-z_][A-Za-z0-9_-]*)\b/g
      : /\binputs\.([A-Za-z_][A-Za-z0-9_-]*)\b/g;

  for (let m = re.exec(cel); m; m = re.exec(cel)) {
    if (m[1]) out.push(m[1]);
  }
  return out;
}

/**
 * Get all protocol references used in a workflow
 */
export function getWorkflowDependencies(workflow: Workflow): string[] {
  return workflow.nodes.map((node) => {
    const proto = (node as any).protocol;
    if (node.type === 'action_ref' && node.action) {
      return `${proto}/${node.action}`;
    }
    if (node.type === 'query_ref' && node.query) {
      return `${proto}/${node.query}`;
    }
    return String(proto ?? '');
  });
}

/**
 * Get all unique protocols referenced in a workflow
 */
export function getWorkflowProtocols(workflow: Workflow): string[] {
  const protocols = new Set<string>();
  for (const node of workflow.nodes) {
    const { protocol } = parseProtocolRef(String((node as any).protocol ?? ''));
    protocols.add(protocol);
  }
  return Array.from(protocols);
}

/**
 * Get workflow nodes in dependency order
 */
export function getExecutionOrder(workflow: Workflow): WorkflowNode[] {
  const dag = buildWorkflowDag(workflow, { include_implicit_deps: true });
  const nodesById = new Map(workflow.nodes.map((n) => [n.id, n] as const));
  return dag.order.map((id) => {
    const node = nodesById.get(id);
    if (!node) throw new Error(`Internal error: node "${id}" not found`);
    return node;
  });
}
