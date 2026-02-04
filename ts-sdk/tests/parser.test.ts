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
ais_version: "1.0"
type: protocol
protocol:
  name: test-protocol
  version: "1.0.0"
  chain_id: 1
  addresses:
    router: "0x1234567890123456789012345678901234567890"
actions:
  - name: test_action
    contract: router
    method: execute
    inputs:
      - name: amount
        type: uint256
`;
    const result = parseAIS(yaml);
    expect(result.type).toBe('protocol');
    expect(result.ais_version).toBe('1.0');
    if (result.type === 'protocol') {
      expect(result.protocol.name).toBe('test-protocol');
      expect(result.actions).toHaveLength(1);
    }
  });

  it('parses a valid pack', () => {
    const yaml = `
ais_version: "1.0"
type: pack
pack:
  name: test-pack
  version: "1.0.0"
protocols:
  - protocol: uniswap-v3
    version: "1.0.0"
`;
    const result = parseAIS(yaml);
    expect(result.type).toBe('pack');
    if (result.type === 'pack') {
      expect(result.pack.name).toBe('test-pack');
      expect(result.protocols).toHaveLength(1);
    }
  });

  it('parses a valid workflow', () => {
    const yaml = `
ais_version: "1.0"
type: workflow
workflow:
  name: test-workflow
  version: "1.0.0"
inputs:
  - name: token
    type: address
steps:
  - id: step1
    uses: uniswap-v3/swap_exact_in
    with:
      token_in: "\${input.token}"
`;
    const result = parseAIS(yaml);
    expect(result.type).toBe('workflow');
    if (result.type === 'workflow') {
      expect(result.workflow.name).toBe('test-workflow');
      expect(result.steps).toHaveLength(1);
    }
  });

  it('throws on invalid YAML', () => {
    expect(() => parseAIS('{{{')).toThrow(AISParseError);
  });

  it('throws on invalid document structure', () => {
    expect(() => parseAIS('type: invalid')).toThrow(AISParseError);
  });
});

describe('parseProtocolSpec', () => {
  it('validates protocol-specific fields', () => {
    const yaml = `
ais_version: "1.0"
type: protocol
protocol:
  name: uniswap-v3
  version: "1.0.0"
  chain_id: 1
  addresses:
    router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
queries:
  - name: get_pool
    contract: factory
    method: getPool
    inputs:
      - name: token0
        type: address
    outputs:
      - name: pool
        type: address
actions:
  - name: swap_exact_in
    contract: router
    method: exactInputSingle
    inputs:
      - name: tokenIn
        type: address
      - name: amountIn
        type: uint256
    requires_queries:
      - get_pool
`;
    const result = parseProtocolSpec(yaml);
    expect(result.protocol.name).toBe('uniswap-v3');
    expect(result.queries).toHaveLength(1);
    expect(result.actions[0].requires_queries).toContain('get_pool');
  });

  it('rejects pack documents', () => {
    const yaml = `
ais_version: "1.0"
type: pack
pack:
  name: test
  version: "1.0.0"
protocols: []
`;
    expect(() => parseProtocolSpec(yaml)).toThrow(AISParseError);
  });
});

describe('detectType', () => {
  it('detects protocol type', () => {
    expect(detectType('type: protocol')).toBe('protocol');
  });

  it('detects pack type', () => {
    expect(detectType('type: pack')).toBe('pack');
  });

  it('detects workflow type', () => {
    expect(detectType('type: workflow')).toBe('workflow');
  });

  it('returns null for invalid type', () => {
    expect(detectType('type: invalid')).toBeNull();
  });

  it('returns null for invalid YAML', () => {
    expect(detectType('{{{')).toBeNull();
  });
});

describe('validate', () => {
  it('returns valid: true for valid documents', () => {
    const yaml = `
ais_version: "1.0"
type: protocol
protocol:
  name: test
  version: "1.0.0"
  chain_id: 1
  addresses: {}
actions: []
`;
    const result = validate(yaml);
    expect(result.valid).toBe(true);
  });

  it('returns issues for invalid documents', () => {
    const result = validate('type: protocol');
    expect(result.valid).toBe(false);
    if (!result.valid) {
      expect(result.issues.length).toBeGreaterThan(0);
    }
  });
});

describe('parsePack', () => {
  it('parses pack with constraints', () => {
    const yaml = `
ais_version: "1.0"
type: pack
pack:
  name: safe-defi
  version: "1.0.0"
protocols:
  - protocol: uniswap-v3
    version: "1.0.0"
constraints:
  tokens:
    allowlist:
      - "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
  slippage:
    max_bps: 50
  require_simulation: true
`;
    const result = parsePack(yaml);
    expect(result.constraints?.slippage?.max_bps).toBe(50);
    expect(result.constraints?.require_simulation).toBe(true);
  });
});

describe('parseWorkflow', () => {
  it('parses workflow with multiple steps', () => {
    const yaml = `
ais_version: "1.0"
type: workflow
workflow:
  name: swap-to-token
  version: "1.0.0"
inputs:
  - name: target_token
    type: address
  - name: amount
    type: uint256
steps:
  - id: approve
    uses: erc20/approve
    with:
      spender: "\${address.router}"
      amount: "\${input.amount}"
  - id: swap
    uses: uniswap-v3/swap_exact_in
    with:
      token_out: "\${input.target_token}"
    condition: "\${step.approve.success}"
`;
    const result = parseWorkflow(yaml);
    expect(result.steps).toHaveLength(2);
    expect(result.steps[1].condition).toBe('${step.approve.success}');
  });
});
