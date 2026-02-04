import { describe, it, expect, beforeEach } from 'vitest';
import {
  validateConstraints,
  requiresSimulation,
  validateWorkflow,
  getWorkflowDependencies,
  getWorkflowProtocols,
  createContext,
  registerProtocol,
  parseProtocolSpec,
  parseWorkflow,
  type ResolverContext,
  type PackConstraints,
} from '../src/index.js';

describe('validateConstraints', () => {
  const constraints: PackConstraints = {
    tokens: {
      allowlist: [
        '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2', // WETH
        '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', // USDC
      ],
      blocklist: ['0x0000000000000000000000000000000000000000'],
    },
    amounts: {
      max_usd: 10000,
      max_percentage_of_balance: 50,
    },
    slippage: {
      max_bps: 100,
    },
    require_simulation: true,
  };

  it('passes valid inputs', () => {
    const result = validateConstraints(constraints, {
      token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      amount_usd: 5000,
      slippage_bps: 50,
    });
    expect(result.valid).toBe(true);
    expect(result.violations).toHaveLength(0);
  });

  it('rejects token not in allowlist', () => {
    const result = validateConstraints(constraints, {
      token: '0x1234567890123456789012345678901234567890',
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('tokens.allowlist');
  });

  it('rejects blocklisted token', () => {
    const result = validateConstraints(constraints, {
      token: '0x0000000000000000000000000000000000000000',
    });
    expect(result.valid).toBe(false);
    expect(result.violations.some(v => v.constraint === 'tokens.blocklist')).toBe(true);
  });

  it('rejects amount exceeding max_usd', () => {
    const result = validateConstraints(constraints, {
      token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      amount_usd: 15000,
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('amounts.max_usd');
  });

  it('rejects slippage exceeding max_bps', () => {
    const result = validateConstraints(constraints, {
      token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      slippage_bps: 150,
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toBe('slippage.max_bps');
  });

  it('collects multiple violations', () => {
    const result = validateConstraints(constraints, {
      token: '0x1234567890123456789012345678901234567890',
      amount_usd: 20000,
      slippage_bps: 200,
    });
    expect(result.valid).toBe(false);
    expect(result.violations.length).toBeGreaterThanOrEqual(3);
  });

  it('handles undefined constraints', () => {
    const result = validateConstraints(undefined, {
      token: '0x1234',
      amount_usd: 999999,
    });
    expect(result.valid).toBe(true);
  });
});

describe('requiresSimulation', () => {
  it('returns true when require_simulation is true', () => {
    expect(requiresSimulation({ require_simulation: true })).toBe(true);
  });

  it('returns false when require_simulation is false', () => {
    expect(requiresSimulation({ require_simulation: false })).toBe(false);
  });

  it('returns false when undefined', () => {
    expect(requiresSimulation(undefined)).toBe(false);
    expect(requiresSimulation({})).toBe(false);
  });
});

describe('validateWorkflow', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(
      ctx,
      parseProtocolSpec(`
ais_version: "1.0"
type: protocol
protocol:
  name: uniswap-v3
  version: "1.0.0"
  chain_id: 1
  addresses:
    router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
actions:
  - name: swap_exact_in
    contract: router
    method: exactInputSingle
    inputs:
      - name: tokenIn
        type: address
`)
    );
    registerProtocol(
      ctx,
      parseProtocolSpec(`
ais_version: "1.0"
type: protocol
protocol:
  name: erc20
  version: "1.0.0"
  chain_id: 1
  addresses: {}
actions:
  - name: approve
    contract: token
    method: approve
    inputs:
      - name: spender
        type: address
`)
    );
  });

  it('validates workflow with valid references', () => {
    const workflow = parseWorkflow(`
ais_version: "1.0"
type: workflow
workflow:
  name: test
  version: "1.0.0"
inputs:
  - name: amount
    type: uint256
steps:
  - id: approve
    uses: erc20/approve
    with:
      amount: "\${input.amount}"
  - id: swap
    uses: uniswap-v3/swap_exact_in
    with:
      amount: "\${step.approve.result}"
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(true);
  });

  it('detects unknown action reference', () => {
    const workflow = parseWorkflow(`
ais_version: "1.0"
type: workflow
workflow:
  name: test
  version: "1.0.0"
inputs: []
steps:
  - id: step1
    uses: unknown-protocol/unknown-action
    with: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].field).toBe('uses');
    expect(result.issues[0].reference).toBe('unknown-protocol/unknown-action');
  });

  it('detects undeclared input reference', () => {
    const workflow = parseWorkflow(`
ais_version: "1.0"
type: workflow
workflow:
  name: test
  version: "1.0.0"
inputs: []
steps:
  - id: step1
    uses: erc20/approve
    with:
      amount: "\${input.undeclared}"
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].message).toContain('undeclared');
  });

  it('detects forward step reference', () => {
    const workflow = parseWorkflow(`
ais_version: "1.0"
type: workflow
workflow:
  name: test
  version: "1.0.0"
inputs: []
steps:
  - id: step1
    uses: erc20/approve
    with:
      value: "\${step.step2.output}"
  - id: step2
    uses: uniswap-v3/swap_exact_in
    with: {}
`);
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(false);
    expect(result.issues[0].message).toContain('step2');
  });
});

describe('getWorkflowDependencies', () => {
  it('extracts all action references', () => {
    const workflow = parseWorkflow(`
ais_version: "1.0"
type: workflow
workflow:
  name: test
  version: "1.0.0"
inputs: []
steps:
  - id: s1
    uses: erc20/approve
    with: {}
  - id: s2
    uses: uniswap-v3/swap
    with: {}
`);
    const deps = getWorkflowDependencies(workflow);
    expect(deps).toEqual(['erc20/approve', 'uniswap-v3/swap']);
  });
});

describe('getWorkflowProtocols', () => {
  it('extracts unique protocols', () => {
    const workflow = parseWorkflow(`
ais_version: "1.0"
type: workflow
workflow:
  name: test
  version: "1.0.0"
inputs: []
steps:
  - id: s1
    uses: erc20/approve
    with: {}
  - id: s2
    uses: uniswap-v3/swap
    with: {}
  - id: s3
    uses: erc20/transfer
    with: {}
`);
    const protocols = getWorkflowProtocols(workflow);
    expect(protocols.sort()).toEqual(['erc20', 'uniswap-v3']);
  });
});
