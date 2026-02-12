import { describe, expect, it } from 'vitest';
import {
  createContext,
  registerProtocol,
  parseProtocolSpec,
  parseWorkflow,
  validateWorkflow,
  buildWorkflowExecutionPlan,
  validateWorkspaceReferences,
  fromWorkflowIssues,
  fromWorkspaceIssues,
} from '../src/index.js';

function makeBasicProtocol(opts: { id: string; version: string; chain: string }) {
  const { id, version, chain } = opts;
  return parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: ${id}, version: "${version}" }
deployments:
  - chain: "${chain}"
    contracts: { router: "0x${'11'.repeat(20)}" }
actions:
  swap:
    description: "swap"
    risk_level: 3
    execution:
      "${chain}":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "swap", inputs: [], outputs: [] }
        args: {}
queries:
  quote:
    description: "quote"
    params: []
    returns: [{ name: out, type: uint256 }]
    execution:
      "${chain}":
        type: evm_read
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "q", inputs: [], outputs: [{ name: out, type: uint256 }] }
        args: {}
`);
}

describe('AGT104 failure cases coverage', () => {
  it('missing chain', () => {
    const ctx = createContext();
    registerProtocol(ctx, makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }));
    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: t, version: "0.0.3" }
nodes:
  - id: n1
    type: action_ref
    protocol: "demo@0.0.1"
    action: swap
    args: {}
`);
    const r = validateWorkflow(wf, ctx);
    expect(r.valid).toBe(false);
    const structured = fromWorkflowIssues(r.issues);
    expect(structured.some((i) => i.field_path === 'chain')).toBe(true);
  });

  it('action not exists', () => {
    const ctx = createContext();
    registerProtocol(ctx, makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }));
    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: t, version: "0.0.3" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: action_ref
    protocol: "demo@0.0.1"
    action: does_not_exist
    args: {}
`);
    const r = validateWorkflow(wf, ctx);
    expect(r.valid).toBe(false);
    expect(r.issues.some((i) => i.field === 'action')).toBe(true);
  });

  it('deps cycle', () => {
    const ctx = createContext();
    registerProtocol(ctx, makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }));
    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: t, version: "0.0.3" }
default_chain: "eip155:1"
nodes:
  - id: a
    type: query_ref
    protocol: "demo@0.0.1"
    query: quote
    deps: [b]
    args: {}
  - id: b
    type: query_ref
    protocol: "demo@0.0.1"
    query: quote
    deps: [a]
    args: {}
`);
    const r = validateWorkflow(wf, ctx);
    expect(r.valid).toBe(false);
    expect(r.issues.some((i) => String(i.message).includes('cycle'))).toBe(true);
  });

  it('protocol version mismatch', () => {
    const ctx = createContext();
    registerProtocol(ctx, makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }));
    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: t, version: "0.0.3" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: query_ref
    protocol: "demo@0.0.2"
    query: quote
    args: {}
`);
    const r = validateWorkflow(wf, ctx);
    expect(r.valid).toBe(false);
    expect(r.issues.some((i) => i.field === 'protocol')).toBe(true);
  });

  it('workflow imports enforcement (not imported)', () => {
    const ctx = createContext();
    registerProtocol(ctx, makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }));
    // Mark as workspace-loaded to require explicit import
    (ctx as any).protocol_sources.set('demo', 'workspace');
    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: t, version: "0.0.3" }
default_chain: "eip155:1"
imports: { protocols: [] }
nodes:
  - id: n1
    type: query_ref
    protocol: "demo@0.0.1"
    query: quote
    args: {}
`);
    const r = validateWorkflow(wf, ctx, { enforce_imports: true });
    expect(r.valid).toBe(false);
    expect(r.issues.some((i) => String(i.message).includes('must be explicitly imported'))).toBe(true);
  });

  it('execution chain mismatch causes plan build error', () => {
    const ctx = createContext();
    // Protocol supports only eip155:1
    registerProtocol(ctx, makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }));
    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: t, version: "0.0.3" }
default_chain: "eip155:137"
nodes:
  - id: n1
    type: action_ref
    protocol: "demo@0.0.1"
    action: swap
    args: {}
`);
    expect(() => buildWorkflowExecutionPlan(wf, ctx)).toThrow(/No execution matches chain/);
  });

  it('pack includes mismatch (workspace validator)', () => {
    const issues = validateWorkspaceReferences({
      protocols: [{ path: '/ws/p.yaml', document: makeBasicProtocol({ id: 'demo', version: '0.0.1', chain: 'eip155:1' }) } as any],
      packs: [
        {
          path: '/ws/pack.yaml',
          document: {
            schema: 'ais-pack/0.0.2',
            meta: { name: 'p', version: '1.0.0', extensions: {} },
            includes: [{ protocol: 'demo', version: '9.9.9', extensions: {} }],
            extensions: {},
          },
        } as any,
      ],
      workflows: [],
    });
    expect(issues.some((i) => i.severity === 'error')).toBe(true);
    const structured = fromWorkspaceIssues(issues);
    expect(structured.some((i) => i.kind === 'workspace_validation')).toBe(true);
  });

  it('structured converters always return agent-friendly shape', () => {
    const wfIssues = fromWorkflowIssues([{ nodeId: 'n1', field: 'deps', message: 'x', reference: 'y' }]);
    expect(wfIssues[0]).toMatchObject({ kind: 'workflow_validation', severity: 'error', node_id: 'n1', field_path: 'deps' });

    const wsIssues = fromWorkspaceIssues([{ path: '/x', severity: 'warning', message: 'm', field_path: 'meta' }]);
    expect(wsIssues[0]).toMatchObject({ kind: 'workspace_validation', severity: 'warning', field_path: 'meta' });
  });
});
