import { describe, it, expect } from 'vitest';
import {
  parseAIS,
  parseProtocolSpec,
  parsePack,
  parseWorkflow,
  detectType,
  validate,
  AISParseError,
} from '../src/index.js';

describe('parseAIS', () => {
  it('parses a valid protocol spec', () => {
    const yaml = `
schema: "ais/1.0"
meta:
  protocol: test-protocol
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x1234567890123456789012345678901234567890"
actions:
  test_action:
    contract: router
    method: execute
`;
    const result = parseAIS(yaml);
    expect(result.schema).toBe('ais/1.0');
    if (result.schema === 'ais/1.0') {
      expect(result.meta.protocol).toBe('test-protocol');
      expect(Object.keys(result.actions)).toHaveLength(1);
    }
  });

  it('parses a valid pack', () => {
    const yaml = `
schema: "ais-pack/1.0"
name: test-pack
version: "1.0.0"
includes:
  - "uniswap-v3@1.0.0"
`;
    const result = parseAIS(yaml);
    expect(result.schema).toBe('ais-pack/1.0');
    if (result.schema === 'ais-pack/1.0') {
      expect(result.name).toBe('test-pack');
      expect(result.includes).toHaveLength(1);
    }
  });

  it('parses a valid workflow', () => {
    const yaml = `
schema: "ais-flow/1.0"
meta:
  name: test-workflow
  version: "1.0.0"
inputs:
  token:
    type: address
nodes:
  - id: step1
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap_exact_in
    args:
      token_in: "\${inputs.token}"
`;
    const result = parseAIS(yaml);
    expect(result.schema).toBe('ais-flow/1.0');
    if (result.schema === 'ais-flow/1.0') {
      expect(result.meta.name).toBe('test-workflow');
      expect(result.nodes).toHaveLength(1);
    }
  });

  it('throws on invalid YAML', () => {
    expect(() => parseAIS('{{{')).toThrow(AISParseError);
  });

  it('throws on invalid document structure', () => {
    expect(() => parseAIS('schema: invalid')).toThrow(AISParseError);
  });
});

describe('parseProtocolSpec', () => {
  it('validates protocol-specific fields', () => {
    const yaml = `
schema: "ais/1.0"
meta:
  protocol: uniswap-v3
  version: "1.0.0"
  name: Uniswap V3
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
      factory: "0x1F98431c8aD98523631AE4a59f267346ea31F984"
queries:
  get_pool:
    contract: factory
    method: getPool
    params:
      - name: token0
        type: address
    outputs:
      - name: pool
        type: address
actions:
  swap_exact_in:
    contract: router
    method: exactInputSingle
    params:
      - name: tokenIn
        type: address
      - name: amountIn
        type: uint256
    requires_queries:
      - get_pool
`;
    const result = parseProtocolSpec(yaml);
    expect(result.meta.protocol).toBe('uniswap-v3');
    expect(result.queries?.get_pool).toBeDefined();
    expect(result.actions.swap_exact_in.requires_queries).toContain('get_pool');
  });

  it('rejects pack documents', () => {
    const yaml = `
schema: "ais-pack/1.0"
name: test
version: "1.0.0"
includes: []
`;
    expect(() => parseProtocolSpec(yaml)).toThrow(AISParseError);
  });
});

describe('detectType', () => {
  it('detects protocol type', () => {
    expect(detectType('schema: "ais/1.0"')).toBe('ais/1.0');
  });

  it('detects pack type', () => {
    expect(detectType('schema: "ais-pack/1.0"')).toBe('ais-pack/1.0');
  });

  it('detects workflow type', () => {
    expect(detectType('schema: "ais-flow/1.0"')).toBe('ais-flow/1.0');
  });

  it('returns null for invalid type', () => {
    expect(detectType('schema: invalid')).toBeNull();
  });

  it('returns null for invalid YAML', () => {
    expect(detectType('{{{')).toBeNull();
  });
});

describe('validate', () => {
  it('returns valid: true for valid documents', () => {
    const yaml = `
schema: "ais/1.0"
meta:
  protocol: test
  version: "1.0.0"
deployments: []
actions: {}
`;
    const result = validate(yaml);
    expect(result.valid).toBe(true);
  });

  it('returns issues for invalid documents', () => {
    const result = validate('schema: "ais/1.0"');
    expect(result.valid).toBe(false);
    if (!result.valid) {
      expect(result.issues.length).toBeGreaterThan(0);
    }
  });
});

describe('parsePack', () => {
  it('parses pack with policy', () => {
    const yaml = `
schema: "ais-pack/1.0"
name: safe-defi
version: "1.0.0"
includes:
  - "uniswap-v3@1.0.0"
policy:
  risk_threshold: 3
  hard_constraints:
    max_slippage_bps: 50
token_policy:
  allowlist:
    - "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
  resolution: strict
`;
    const result = parsePack(yaml);
    expect(result.policy?.hard_constraints?.max_slippage_bps).toBe(50);
    expect(result.token_policy?.resolution).toBe('strict');
  });
});

describe('parseWorkflow', () => {
  it('parses workflow with multiple nodes', () => {
    const yaml = `
schema: "ais-flow/1.0"
meta:
  name: swap-to-token
  version: "1.0.0"
inputs:
  target_token:
    type: address
  amount:
    type: uint256
nodes:
  - id: approve
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args:
      spender: "\${ctx.router_address}"
      amount: "\${inputs.amount}"
  - id: swap
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap_exact_in
    args:
      token_out: "\${inputs.target_token}"
    requires_queries:
      - approve
    condition: "nodes.approve.outputs.success == true"
`;
    const result = parseWorkflow(yaml);
    expect(result.nodes).toHaveLength(2);
    expect(result.nodes[1].condition).toContain('approve');
  });
});
