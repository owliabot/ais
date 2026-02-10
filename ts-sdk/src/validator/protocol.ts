import type { ProtocolSpec, ExecutionSpec, EvmRead } from '../schema/index.js';
import { isCoreExecutionSpec } from '../schema/index.js';

export type ProtocolIssueSeverity = 'error' | 'warning' | 'info';

export interface ProtocolIssue {
  severity: ProtocolIssueSeverity;
  message: string;
  field_path?: string;
}

export function validateProtocolSemantics(spec: ProtocolSpec): ProtocolIssue[] {
  const issues: ProtocolIssue[] = [];

  // Queries: returns ↔ ABI outputs binding (EVM reads)
  for (const [queryId, query] of Object.entries(spec.queries ?? {})) {
    const returns = query.returns ?? null;
    const returnNames = returns ? returns.map((r) => r.name) : [];

    if (returns) {
      const seen = new Set<string>();
      for (let i = 0; i < returnNames.length; i++) {
        const n = returnNames[i]!;
        if (!n || typeof n !== 'string') continue;
        if (seen.has(n)) {
          issues.push({
            severity: 'error',
            message: `Query "${queryId}" has duplicate return name: "${n}"`,
            field_path: `queries.${queryId}.returns[${i}].name`,
          });
        }
        seen.add(n);
      }
    }

    for (const [pattern, execution] of Object.entries(query.execution ?? {} as Record<string, ExecutionSpec>)) {
      if (!isCoreExecutionSpec(execution)) continue;
      if (execution.type !== 'evm_read') continue;

      issues.push(
        ...validateEvmReadReturnsBinding({
          queryId,
          chainPattern: pattern,
          returns,
          execution,
        })
      );
    }
  }

  return issues;
}

export class ProtocolSemanticsError extends Error {
  readonly details: { issues: ProtocolIssue[] };

  constructor(
    message: string,
    public readonly issues: ProtocolIssue[]
  ) {
    super(message);
    this.name = 'ProtocolSemanticsError';
    this.details = { issues };
  }
}

export function assertProtocolSemantics(spec: ProtocolSpec): void {
  const issues = validateProtocolSemantics(spec).filter((i) => i.severity === 'error');
  if (issues.length === 0) return;
  const msg = issues
    .slice(0, 20)
    .map((i) => `${i.field_path ? `${i.field_path}: ` : ''}${i.message}`)
    .join('; ');
  throw new ProtocolSemanticsError(`Protocol semantics validation failed: ${msg}`, issues);
}

function validateEvmReadReturnsBinding(args: {
  queryId: string;
  chainPattern: string;
  returns: Array<{ name: string; type: string }> | null;
  execution: EvmRead;
}): ProtocolIssue[] {
  const issues: ProtocolIssue[] = [];
  const { queryId, chainPattern, returns, execution } = args;

  const abiOutputs = execution.abi.outputs ?? [];
  const abiOutNames = abiOutputs.map((o) => o.name);
  const abiOutTypes = abiOutputs.map((o) => o.type);

  if (!returns || returns.length === 0) {
    if (abiOutputs.length > 0) {
      issues.push({
        severity: 'error',
        message: `Query "${queryId}" is evm_read but is missing returns; returns must match abi.outputs (chain=${chainPattern})`,
        field_path: `queries.${queryId}.returns`,
      });
    }
    return issues;
  }

  if (abiOutputs.length === 0) {
    issues.push({
      severity: 'error',
      message: `Query "${queryId}" is evm_read but abi.outputs is empty; abi.outputs must match returns (chain=${chainPattern})`,
      field_path: `queries.${queryId}.execution.${chainPattern}.abi.outputs`,
    });
    return issues;
  }

  if (abiOutputs.length !== returns.length) {
    issues.push({
      severity: 'error',
      message: `Query "${queryId}" returns count (${returns.length}) must match abi.outputs count (${abiOutputs.length}) (chain=${chainPattern})`,
      field_path: `queries.${queryId}.returns`,
    });
    return issues;
  }

  for (let i = 0; i < returns.length; i++) {
    const r = returns[i]!;
    const outName = abiOutNames[i]!;
    const outType = abiOutTypes[i]!;
    if (!outName || typeof outName !== 'string' || outName.length === 0) {
      issues.push({
        severity: 'error',
        message: `Query "${queryId}" abi.outputs[${i}] must have a non-empty name (chain=${chainPattern})`,
        field_path: `queries.${queryId}.execution.${chainPattern}.abi.outputs[${i}].name`,
      });
    }
    if (outName !== r.name) {
      issues.push({
        severity: 'error',
        message: `Query "${queryId}" returns[${i}].name must equal abi.outputs[${i}].name ("${r.name}" != "${outName}") (chain=${chainPattern})`,
        field_path: `queries.${queryId}.returns[${i}].name`,
      });
    }
    if (typeof outType === 'string' && typeof r.type === 'string' && outType !== r.type) {
      issues.push({
        severity: 'error',
        message: `Query "${queryId}" returns[${i}].type must equal abi.outputs[${i}].type ("${r.type}" != "${outType}") (chain=${chainPattern})`,
        field_path: `queries.${queryId}.returns[${i}].type`,
      });
    }
  }

  return issues;
}
