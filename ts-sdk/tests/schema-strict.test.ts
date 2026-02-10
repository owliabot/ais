import { describe, it, expect } from 'vitest';
import {
  AISParseError,
  parseProtocolSpec,
  parsePack,
  parseWorkflow,
  DetectSchema,
  ExecutionPlanSchema,
} from '../src/index.js';

function expectUnrecognizedKeys(fn: () => unknown, opts?: { pathContains?: string; key?: string }) {
  let err: unknown;
  try {
    fn();
  } catch (e) {
    err = e;
  }
  expect(err).toBeInstanceOf(AISParseError);

  const issues = (err as AISParseError).details as any[];
  expect(Array.isArray(issues)).toBe(true);

  const msgs = issues.map((i) => String(i.message ?? '')).join('\n');
  expect(msgs).toContain('Unrecognized key');

  if (opts?.key) {
    expect(msgs).toContain(opts.key);
  }
  if (opts?.pathContains) {
    const paths = issues.map((i) => (Array.isArray(i.path) ? i.path.join('.') : String(i.path ?? ''))).join('\n');
    expect(paths).toContain(opts.pathContains);
  }
}

describe('T001 strict schemas + extensions', () => {
  it('rejects unknown top-level fields (protocol)', () => {
    expectUnrecognizedKeys(
      () =>
        parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments: []
actions: {}
foo: 1
`),
      { key: 'foo' }
    );
  });

  it('rejects unknown nested fields (protocol.meta)', () => {
    expectUnrecognizedKeys(
      () =>
        parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2", foo: 1 }
deployments: []
actions: {}
`),
      { pathContains: 'meta', key: 'foo' }
    );
  });

  it('rejects unknown fields in action objects', () => {
    expectUnrecognizedKeys(
      () =>
        parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x0000000000000000000000000000000000000000" }
actions:
  a:
    description: "a"
    risk_level: 1
    foo: 1
    execution:
      "*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "x", inputs: [], outputs: [] }
        args: {}
`),
      { pathContains: 'actions.a', key: 'foo' }
    );
  });

  it('rejects unknown fields in core execution specs (evm_call)', () => {
    expectUnrecognizedKeys(
      () =>
        parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x0000000000000000000000000000000000000000" }
actions:
  a:
    description: "a"
    risk_level: 1
    execution:
      "*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "x", inputs: [], outputs: [] }
        args: {}
        gas: { lit: "1" }
`),
      { pathContains: 'execution.*', key: 'gas' }
    );
  });

  it('rejects unknown top-level fields (pack)', () => {
    expectUnrecognizedKeys(
      () =>
        parsePack(`
schema: "ais-pack/0.0.2"
meta: { name: p, version: "0.0.2" }
includes: []
foo: 1
`),
      { key: 'foo' }
    );
  });

  it('rejects unknown fields inside pack.includes[]', () => {
    expectUnrecognizedKeys(
      () =>
        parsePack(`
schema: "ais-pack/0.0.2"
meta: { name: p, version: "0.0.2" }
includes:
  - protocol: demo
    version: "0.0.2"
    foo: 1
`),
      { pathContains: 'includes.0', key: 'foo' }
    );
  });

  it('rejects unknown top-level fields (workflow)', () => {
    expectUnrecognizedKeys(
      () =>
        parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: wf, version: "0.0.2" }
default_chain: "eip155:1"
nodes: []
foo: 1
`),
      { key: 'foo' }
    );
  });

  it('rejects unknown fields inside workflow.nodes[]', () => {
    expectUnrecognizedKeys(
      () =>
        parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: wf, version: "0.0.2" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: query_ref
    skill: "demo@0.0.2"
    query: q
    foo: 1
`),
      { pathContains: 'nodes.0', key: 'foo' }
    );
  });

  it('rejects unknown fields inside detect objects', () => {
    expect(() =>
      DetectSchema.parse({
        kind: 'choose_one',
        candidates: [{ lit: '1' }],
        foo: 1,
      } as any)
    ).toThrowError(/Unrecognized key/);
  });

  it('rejects unknown fields in ExecutionPlan', () => {
    const bad = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 'n1',
          chain: 'eip155:1',
          kind: 'execution',
          execution: {
            type: 'evm_read',
            to: { lit: '0x0000000000000000000000000000000000000000' },
            abi: { type: 'function', name: 'x', inputs: [], outputs: [] },
            args: {},
          },
          foo: 1,
        },
      ],
    };
    expect(() => ExecutionPlanSchema.parse(bad as any)).toThrowError(/Unrecognized key/);
  });

  it('allows extensions on core objects', () => {
    const doc = parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2", extensions: { ui: { color: "red" } } }
deployments: []
actions:
  a:
    description: "a"
    risk_level: 1
    extensions: { note: "ok" }
    execution:
      "*":
        type: evm_call
        to: { lit: "0x0000000000000000000000000000000000000000" }
        abi: { type: "function", name: "x", inputs: [], outputs: [] }
        args: {}
extensions: { top: true }
`);
    expect(doc.meta.extensions).toBeTruthy();
    expect((doc.actions as any).a.extensions).toBeTruthy();
    expect((doc as any).extensions).toBeTruthy();
  });
});
