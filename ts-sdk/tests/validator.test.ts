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
      { chain: 'eip155:1', symbol: 'WETH', address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2' },
      { chain: 'eip155:1', symbol: 'USDC', address: '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48' },
    ],
    resolution: {
      require_allowlist_for_symbol_resolution: true,
    },
  };

  it('passes valid inputs', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      token_address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      slippage_bps: 50,
    });
    expect(result.valid).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('rejects token not in allowlist (strict mode)', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      token_address: '0x1234567890123456789012345678901234567890',
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('token_policy.allowlist');
  });

  it('soft-rejects token not in allowlist (permissive mode)', () => {
    const permissivePolicy: TokenPolicy = {
      allowlist: [
        { chain: 'eip155:1', symbol: 'WETH', address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2' },
      ],
      resolution: {
        require_allowlist_for_symbol_resolution: false,
      },
    };
    const result = validateConstraints(policy, permissivePolicy, {
      token_address: '0x1234567890123456789012345678901234567890',
    });
    expect(result.valid).toBe(true);
    expect(result.requires_approval).toBe(true);
    expect(result.approval_reasons.length).toBeGreaterThan(0);
  });

  it('rejects slippage exceeding max_bps', () => {
    const result = validateConstraints(policy, tokenPolicy, {
      token_address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
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
      token_address: '0x1234',
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
schema: "ais/0.0.2"
meta:
  protocol: uniswap-v3
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
actions:
  swap-exact-in:
    description: "Swap exact input"
    risk_level: 3
    params:
      - name: tokenIn
        type: address
        description: "Input token"
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi:
          type: "function"
          name: "exactInputSingle"
          inputs:
            - { name: "tokenIn", type: "address" }
          outputs: []
        args:
          tokenIn: { ref: "params.tokenIn" }
`)
    );
    registerProtocol(
      ctx,
      parseProtocolSpec(`
schema: "ais/0.0.2"
meta:
  protocol: erc20
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  approve:
    description: "Approve spender"
    risk_level: 2
    params:
      - name: token
        type: address
        description: "Token address"
      - name: spender
        type: address
        description: "Spender address"
      - name: amount
        type: uint256
        description: "Amount"
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "params.token" }
        abi:
          type: "function"
          name: "approve"
          inputs:
            - { name: "spender", type: "address" }
            - { name: "amount", type: "uint256" }
          outputs: []
        args:
          spender: { ref: "params.spender" }
          amount: { ref: "params.amount" }
`)
    );
  });

  it('validates workflow with valid references', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
inputs:
  amount:
    type: uint256
nodes:
  - id: approve
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args:
      amount: { ref: "inputs.amount" }
  - id: swap
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap-exact-in
    args:
      amount: { ref: "nodes.approve.outputs.result" }
    deps:
      - approve
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(true);
  });

  it('accepts out-of-order nodes (execution order comes from deps/refs)', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
inputs:
  amount:
    type: uint256
nodes:
  - id: swap
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap-exact-in
    args:
      amount: { ref: "nodes.approve.outputs.result" }
  - id: approve
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args:
      amount: { ref: "inputs.amount" }
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(true);
  });

  it('detects dependency cycles', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
nodes:
  - id: a
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args: {}
    deps: ["b"]
  - id: b
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args: {}
    deps: ["a"]
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues.some((i) => i.field === 'deps' && i.message.includes('cycle'))).toBe(true);
  });

  it('detects unknown protocol reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
nodes:
  - id: step1
    type: action_ref
    protocol: "unknown-protocol@0.0.2"
    action: unknown-action
    args: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].field).toBe('protocol');
  });

  it('detects unknown action reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
nodes:
  - id: step1
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: unknown-action
    args: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].field).toBe('action');
  });

  it('detects undeclared input reference', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
inputs: {}
nodes:
  - id: step1
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args:
      amount: { ref: "inputs.undeclared" }
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].message).toContain('undeclared');
  });

  it('accepts forward node reference (execution order inferred from refs)', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
nodes:
  - id: step1
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args:
      value: { ref: "nodes.step2.outputs.result" }
  - id: step2
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap-exact-in
    args: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(true);
  });

  it('rejects workspace-scanned protocol when not explicitly imported', () => {
    const ctx2 = createContext();
    registerProtocol(
      ctx2,
      parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  do:
    description: "do"
    risk_level: 1
    execution:
      "eip155:*":
        type: evm_call
        to: { lit: "0x1111111111111111111111111111111111111111" }
        abi: { type: "function", name: "do", inputs: [], outputs: [] }
        args: {}
`)
      ,
      { source: 'workspace' }
    );

    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: wf, version: "0.0.3" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: action_ref
    protocol: "demo@0.0.2"
    action: do
    args: {}
`);

    const result = validateWorkflow(workflow, ctx2, { enforce_imports: true });
    expect(result.valid).toBe(false);
    expect(result.issues.some((i) => i.field === 'protocol' && i.message.includes('explicitly imported'))).toBe(true);
  });

  it('accepts explicitly imported workspace protocol', () => {
    const ctx2 = createContext();
    registerProtocol(
      ctx2,
      parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  do:
    description: "do"
    risk_level: 1
    execution:
      "eip155:*":
        type: evm_call
        to: { lit: "0x1111111111111111111111111111111111111111" }
        abi: { type: "function", name: "do", inputs: [], outputs: [] }
        args: {}
`)
      ,
      { source: 'workspace' }
    );

    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: wf, version: "0.0.3" }
imports:
  protocols:
    - protocol: "demo@0.0.2"
      path: "./demo.ais.yaml"
default_chain: "eip155:1"
nodes:
  - id: n1
    type: action_ref
    protocol: "demo@0.0.2"
    action: do
    args: {}
`);

    const result = validateWorkflow(workflow, ctx2, { enforce_imports: true });
    expect(result.valid).toBe(true);
  });
});

describe('getWorkflowDependencies', () => {
  it('extracts all action references', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
nodes:
  - id: s1
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args: {}
  - id: s2
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap
    args: {}
`);
    const deps = getWorkflowDependencies(workflow);
    expect(deps).toContain('erc20@0.0.2/approve');
    expect(deps).toContain('uniswap-v3@0.0.2/swap');
  });
});

describe('getWorkflowProtocols', () => {
  it('extracts unique protocols', () => {
    const workflow = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta:
  name: test
  version: "0.0.3"
default_chain: "eip155:1"
nodes:
  - id: s1
    type: action_ref
    protocol: "erc20@0.0.2"
    action: approve
    args: {}
  - id: s2
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap
    args: {}
  - id: s3
    type: action_ref
    protocol: "erc20@0.0.2"
    action: transfer
    args: {}
`);
    const protocols = getWorkflowProtocols(workflow);
    expect(protocols.sort()).toEqual(['erc20', 'uniswap-v3']);
  });
});
