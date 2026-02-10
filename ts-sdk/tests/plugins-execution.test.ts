import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import {
  parseProtocolSpec,
  createExecutionTypeRegistry,
  ValueRefSchema,
} from '../src/index.js';

describe('T440 execution type plugins', () => {
  it('rejects unknown execution.type by default', () => {
    const yaml = `
schema: "ais/0.0.2"
meta: { protocol: "p", version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  a:
    description: "a"
    risk_level: 1
    execution:
      "eip155:*":
        type: "my_plugin_exec"
        foo: { lit: 1 }
`;
    expect(() => parseProtocolSpec(yaml)).toThrow(/Unknown execution type "my_plugin_exec"/);
  });

  it('accepts registered plugin execution types and validates with plugin schema', () => {
    const registry = createExecutionTypeRegistry();
    registry.register({
      type: 'my_plugin_exec',
      schema: z.object({
        type: z.literal('my_plugin_exec'),
        foo: ValueRefSchema,
      }),
    });

    const yaml = `
schema: "ais/0.0.2"
meta: { protocol: "p", version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  a:
    description: "a"
    risk_level: 1
    params:
      - { name: x, type: uint256, description: "x", required: true }
    execution:
      "eip155:*":
        type: "my_plugin_exec"
        foo: { ref: "params.x" }
`;

    const spec = parseProtocolSpec(yaml, { execution_registry: registry });
    expect(spec.actions.a.execution['eip155:*']?.type).toBe('my_plugin_exec');
  });

  it('does not allow core execution types to bypass core schemas via plugin fallback', () => {
    const yaml = `
schema: "ais/0.0.2"
meta: { protocol: "p", version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  a:
    description: "a"
    risk_level: 1
    execution:
      "eip155:*":
        type: "evm_call"
        abi: { type: "function", name: "f", inputs: [], outputs: [] }
        args: {}
`;
    expect(() => parseProtocolSpec(yaml)).toThrow(/validation failed/i);
  });

  it('rejects removed placeholder types (can be re-added via plugins later)', () => {
    const yaml = `
schema: "ais/0.0.2"
meta: { protocol: "p", version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  a:
    description: "a"
    risk_level: 1
    execution:
      "eip155:*":
        type: "cosmos_message"
        msg_type: "/cosmos.bank.v1beta1.MsgSend"
        mapping: {}
`;
    expect(() => parseProtocolSpec(yaml)).toThrow(/Unknown execution type "cosmos_message"/);
  });
});

