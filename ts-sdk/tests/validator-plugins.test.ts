import { describe, it, expect } from 'vitest';
import {
  createValidatorRegistry,
  parseWorkflow,
  createContext,
  validateWorkflow,
  lintDocument,
} from '../src/index.js';

describe('T441 validator plugin points', () => {
  it('runs plugin workflow validators', () => {
    const ctx = createContext();
    const registry = createValidatorRegistry();
    registry.register({
      id: 'test-plugin',
      validate_workflow: (wf) => {
        const first = wf.nodes[0];
        if (first?.id === 'bad') {
          return [{ nodeId: 'bad', field: 'id', message: 'bad node id' }];
        }
        return [];
      },
    });

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: w, version: "0.0.2" }
default_chain: "eip155:1"
nodes:
  - id: bad
    type: action_ref
    skill: "x@0.0.2"
    action: "a"
`);

    const result = validateWorkflow(wf, ctx, { registry });
    expect(result.valid).toBe(false);
    expect(result.issues.some((i) => i.message === 'bad node id')).toBe(true);
  });

  it('runs plugin lint rules', () => {
    const registry = createValidatorRegistry();
    registry.register({
      id: 'lint-plugin',
      lint_rules: [
        {
          id: 'always-warn',
          severity: 'warning',
          check: () => [{ rule: 'always-warn', severity: 'warning', message: 'warn' }],
        },
      ],
    });

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: w, version: "0.0.2", description: "d" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: action_ref
    skill: "x@0.0.2"
    action: "a"
`);

    const issues = lintDocument(wf as any, { registry });
    expect(issues.some((i) => i.rule === 'always-warn')).toBe(true);
  });
});

