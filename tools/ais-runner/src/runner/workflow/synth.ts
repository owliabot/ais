import type { RunnerValueRef, RunnerWorkflow, RunnerWorkflowNode } from '../../types.js';

export function splitRef(ref: string): [string | null, string | null] {
  const i = ref.indexOf('/');
  if (i <= 0 || i >= ref.length - 1) return [null, null];
  return [ref.slice(0, i), ref.slice(i + 1)];
}

export function synthWorkflow(
  name: string,
  defaultChain: string,
  nodes: RunnerWorkflowNode[],
  imports?: RunnerWorkflow['imports']
): RunnerWorkflow {
  return {
    schema: 'ais-flow/0.0.3',
    meta: { name, version: '0.0.3' },
    default_chain: defaultChain,
    imports,
    nodes,
    extensions: {},
  };
}

export function toLitValueRefs(value: unknown): Record<string, RunnerValueRef> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('args must be a JSON object');
  }
  const out: Record<string, RunnerValueRef> = {};
  for (const [key, entry] of Object.entries(value)) out[key] = { lit: entry };
  return out;
}
