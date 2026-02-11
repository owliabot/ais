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

const MIN_PROTOCOL = `
schema: "ais/0.0.2"
meta:
  protocol: test-protocol
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1234567890123456789012345678901234567890" }
actions:
  test-action:
    description: "Test action"
    risk_level: 2
    execution:
      "eip155:*":
        type: evm_call
        to: { lit: "0x1234567890123456789012345678901234567890" }
        abi: { type: "function", name: "execute", inputs: [], outputs: [] }
        args: {}
`;

const MIN_PACK = `
schema: "ais-pack/0.0.2"
name: test-pack
version: "0.0.2"
includes:
  - protocol: uniswap-v3
    version: "0.0.2"
`;

const MIN_WORKFLOW = `
schema: "ais-flow/0.0.3"
meta:
  name: test-workflow
  version: "0.0.3"
inputs:
  token:
    type: address
nodes:
  - id: step1
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap-exact-in
    args:
      token_in: { ref: "inputs.token" }
`;

describe('parseAIS', () => {
  it('parses a valid protocol spec', () => {
    const result = parseAIS(MIN_PROTOCOL);
    expect(result.schema).toBe('ais/0.0.2');
    if (result.schema === 'ais/0.0.2') {
      expect(result.meta.protocol).toBe('test-protocol');
      expect(Object.keys(result.actions)).toHaveLength(1);
    }
  });

  it('parses a valid pack', () => {
    const result = parseAIS(MIN_PACK);
    expect(result.schema).toBe('ais-pack/0.0.2');
    if (result.schema === 'ais-pack/0.0.2') {
      expect(result.name).toBe('test-pack');
      expect(result.includes).toHaveLength(1);
    }
  });

  it('parses a valid workflow', () => {
    const result = parseAIS(MIN_WORKFLOW);
    expect(result.schema).toBe('ais-flow/0.0.3');
    if (result.schema === 'ais-flow/0.0.3') {
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
  it('parses a protocol spec', () => {
    const result = parseProtocolSpec(MIN_PROTOCOL);
    expect(result.meta.protocol).toBe('test-protocol');
  });

  it('rejects pack documents', () => {
    expect(() => parseProtocolSpec(MIN_PACK)).toThrow(AISParseError);
  });
});

describe('detectType', () => {
  it('detects protocol type', () => {
    expect(detectType('schema: "ais/0.0.2"')).toBe('ais/0.0.2');
  });

  it('detects pack type', () => {
    expect(detectType('schema: "ais-pack/0.0.2"')).toBe('ais-pack/0.0.2');
  });

  it('detects workflow type', () => {
    expect(detectType('schema: "ais-flow/0.0.3"')).toBe('ais-flow/0.0.3');
  });

  it('returns null for invalid type', () => {
    expect(detectType('schema: invalid')).toBeNull();
  });
});

describe('validate', () => {
  it('returns valid: true for valid documents', () => {
    const result = validate(MIN_PROTOCOL);
    expect(result.valid).toBe(true);
  });

  it('returns issues for invalid documents', () => {
    const result = validate('schema: "ais/0.0.2"');
    expect(result.valid).toBe(false);
    if (!result.valid) {
      expect(result.issues.length).toBeGreaterThan(0);
    }
  });
});

describe('parsePack', () => {
  it('parses a pack', () => {
    const result = parsePack(MIN_PACK);
    expect(result.includes).toHaveLength(1);
  });
});

describe('parseWorkflow', () => {
  it('parses a workflow', () => {
    const result = parseWorkflow(MIN_WORKFLOW);
    expect(result.nodes).toHaveLength(1);
  });
});
