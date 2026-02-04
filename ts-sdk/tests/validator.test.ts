import { describe, it, expect, beforeEach } from 'vitest';
import {
  validateConstraints,
  getHardConstraints,
  validateWorkflow,
  getWorkflowDependencies,
  getWorkflowProtocols,
  createContext,
  registerProtocol,
  parseProtocolSpec,
  parseWorkflow,
  type ResolverContext,
  type Policy,
  type TokenPolicy,
} from '../src/index.js';

describe('validateConstraints', () => {
  const policy: Policy = {
    risk_threshold: 3,
    approval_required: ['flash_loan', 'unlimited_approval'],
    hard_constraints: {
      max_slippage_bps: 100,
      allow_unlimited_approval: false,
    },
  };

  const tokenPolicy: TokenPolicy = {
    allowlist: [
      '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2', // WETH
      '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', // USDC
    ],
    resolution: 'strict',
  };

  it('passes valid inputs', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      slippage_bps: 50,
    });
    expect(result.valid).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('rejects token not in allowlist (strict mode)', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      token: '0x1234567890123456789012345678901234567890',
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('token_policy.allowlist');
  });

  it('soft-rejects token not in allowlist (permissive mode)', () => {
    const permissivePolicy: TokenPolicy = {
      allowlist: ['0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2'],
      resolution: 'permissive',
    };
    const result = validateConstraints(policy, permissivePolicy, {
      token: '0x1234567890123456789012345678901234567890',
    });
    expect(result.valid).toBe(true);
    expect(result.requires_approval).toBe(true);
    expect(result.approval_reasons.length).toBeGreaterThan(0);
  });

  it('rejects slippage exceeding max_bps', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      slippage_bps: 150,
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('hard_constraints.max_slippage_bps');
  });

  it('rejects unlimited approval when not allowed', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      unlimited_approval: true,
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('hard_constraints.allow_unlimited_approval');
  });

  it('requires approval for high risk level', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      risk_level: 4,
    });
    expect(result.valid).toBe(true);
    expect(result.requires_approval).toBe(true);
    expect(result.approval_reasons[0]).toContain('Risk level');
  });

  it('requires approval for risky tags', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      risk_tags: ['flash_loan'],
    });
    expect(result.valid).toBe(true);
    expect(result.requires_approval).toBe(true);
    expect(result.approval_reasons[0]).toContain('flash_loan');
  });

  it('handles undefined policy', () => {
    const result = validateConstraints(undefined, undefined, {
      token: '0x1234',
      slippage_bps: 999999,
    });
    expect(result.valid).toBe(true);
  });
});

describe('getHardConstraints', () => {
  it('extracts hard constraints from policy', () => {
    const policy: Policy = {
      hard_constraints: {
        max_slippage_bps: 50,
        allow_unlimited_approval: false,
      },
    };
    const hc = getHardConstraints(policy);
    expect(hc.max_slippage_bps).toBe(50);
  });

  it('returns empty object for undefined policy', () => {
    expect(getHardConstraints(undefined)).toEqual({});
  });
});

describe('validateWorkflow', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(
      ctx,
      parseProtocolSpec(`
schema: "ais/1.0"
meta:
  protocol: uniswap-v3
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
actions:
  swap_exact_in:
    contract: router
    method: exactInputSingle
    params:
      - name: tokenIn
        type: address
`)
    );
    registerProtocol(
      ctx,
      parseProtocolSpec(`
schema: "ais/1.0"
meta:
  protocol: erc20
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  approve:
    contract: token
    method: approve
    params:
      - name: spender
        type: address
`)
    );
  });

  it('validates workflow with valid references', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
inputs:
  amount:
    type: uint256
nodes:
  - id: approve
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args:
      amount: "\${inputs.amount}"
  - id: swap
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap_exact_in
    args:
      amount: "\${nodes.approve.outputs.result}"
    requires_queries:
      - approve
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(true);
  });

  it('detects unknown protocol reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
nodes:
  - id: step1
    type: action_ref
    skill: "unknown-protocol@1.0.0"
    action: unknown_action
    args: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].field).toBe('skill');
  });

  it('detects unknown action reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
nodes:
  - id: step1
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: unknown_action
    args: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].field).toBe('action');
  });

  it('detects undeclared input reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
inputs: {}
nodes:
  - id: step1
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args:
      amount: "\${inputs.undeclared}"
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].message).toContain('undeclared');
  });

  it('detects forward node reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
nodes:
  - id: step1
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args:
      value: "\${nodes.step2.outputs.result}"
  - id: step2
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap_exact_in
    args: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].message).toContain('step2');
  });
});

describe('getWorkflowDependencies', () => {
  it('extracts all action references', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
nodes:
  - id: s1
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args: {}
  - id: s2
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap
    args: {}
`);
    const deps = getWorkflowDependencies(workflow);
    expect(deps).toContain('erc20@1.0.0/approve');
    expect(deps).toContain('uniswap-v3@1.0.0/swap');
  });
});

describe('getWorkflowProtocols', () => {
  it('extracts unique protocols', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/1.0"
meta:
  name: test
  version: "1.0.0"
nodes:
  - id: s1
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args: {}
  - id: s2
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap
    args: {}
  - id: s3
    type: action_ref
    skill: "erc20@1.0.0"
    action: transfer
    args: {}
`);
    const protocols = getWorkflowProtocols(workflow);
    expect(protocols.sort()).toEqual(['erc20', 'uniswap-v3']);
  });
});
