import { describe, expect, it } from 'vitest';
import { compilePlanSkeleton, createContext, parseProtocolSpec, registerProtocol } from '../src/index.js';

function registerDemoProtocol(ctx: any) {
  const protocolYaml = `
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2", name: "Demo" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1111111111111111111111111111111111111111" }
queries:
  quote:
    description: "read quote"
    params: [{ name: amount_in, type: uint256, description: "amount" }]
    returns: [{ name: amount_out, type: uint256 }]
    execution:
      "eip155:*":
        type: evm_read
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "quote", inputs: [{ name: "amount_in", type: "uint256" }], outputs: [{ name: "amount_out", type: "uint256" }] }
        args: { amount_in: { ref: "params.amount_in" } }
actions:
  swap:
    description: "swap"
    risk_level: 3
    risk_tags: ["swap"]
    params: [{ name: amount_in, type: uint256, description: "amount" }, { name: min_out, type: uint256, description: "min" }]
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "swap", inputs: [{ name: "amount_in", type: "uint256" }, { name: "min_out", type: "uint256" }], outputs: [] }
        args: { amount_in: { ref: "params.amount_in" }, min_out: { ref: "params.min_out" } }
`;
  registerProtocol(ctx, parseProtocolSpec(protocolYaml));
}

describe('AGT102 PlanSkeleton compiler', () => {
  it('compiles a minimal skeleton into an ExecutionPlan', () => {
    const ctx = createContext();
    registerDemoProtocol(ctx);
    const result = compilePlanSkeleton(
      {
        schema: 'ais-plan-skeleton/0.0.1',
        default_chain: 'eip155:1',
        nodes: [
          {
            id: 'quote',
            type: 'query_ref',
            protocol: 'demo@0.0.2',
            query: 'quote',
            args: { amount_in: { lit: '7' } },
          },
          {
            id: 'swap',
            type: 'action_ref',
            protocol: 'demo@0.0.2',
            action: 'swap',
            deps: ['quote'],
            args: { amount_in: { lit: '7' }, min_out: { ref: 'nodes.quote.outputs.amount_out' } },
          },
        ],
        policy_hints: { risk_preference: 'low' },
      },
      ctx
    );

    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.plan.schema).toBe('ais-plan/0.0.3');
    expect(result.plan.nodes.length).toBe(2);
    expect((result.plan as any).extensions.plan_skeleton.policy_hints.risk_preference).toBe('low');
  });

  it('returns structured issues on unknown dependency', () => {
    const ctx = createContext();
    registerDemoProtocol(ctx);
    const result = compilePlanSkeleton(
      {
        schema: 'ais-plan-skeleton/0.0.1',
        default_chain: 'eip155:1',
        nodes: [
          { id: 'a', type: 'action_ref', protocol: 'demo@0.0.2', action: 'swap', deps: ['missing'] },
        ],
      },
      ctx
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.issues[0].kind).toBe('dag_error');
    expect(result.issues[0].field_path).toContain('deps');
  });

  it('returns structured issues on missing action reference', () => {
    const ctx = createContext();
    registerDemoProtocol(ctx);
    const result = compilePlanSkeleton(
      {
        schema: 'ais-plan-skeleton/0.0.1',
        default_chain: 'eip155:1',
        nodes: [
          { id: 'a', type: 'action_ref', protocol: 'demo@0.0.2', action: 'nope' },
        ],
      },
      ctx
    );
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.issues.some((i) => i.kind === 'reference_error' || i.kind === 'plan_build_error')).toBe(true);
  });
});

