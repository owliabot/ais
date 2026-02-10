import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import {
  loadFile,
  loadProtocol,
  loadDirectory,
  loadDirectoryAsContext,
} from '../src/index.js';

const TEST_DIR = '/tmp/ais-sdk-test-fixtures';

beforeAll(async () => {
  await mkdir(TEST_DIR, { recursive: true });
  await mkdir(join(TEST_DIR, 'subdir'), { recursive: true });

  // Write test files
  await writeFile(
    join(TEST_DIR, 'uniswap-v3.ais.yaml'),
    `
schema: "ais/0.0.2"
meta:
  protocol: uniswap-v3
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
actions:
  swap:
    description: "Swap tokens"
    risk_level: 3
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "swap", inputs: [], outputs: [] }
        args: {}
`
  );

  await writeFile(
    join(TEST_DIR, 'subdir', 'aave-v3.ais.yaml'),
    `
schema: "ais/0.0.2"
meta:
  protocol: aave-v3
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts:
      pool: "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"
actions:
  supply:
    description: "Supply tokens"
    risk_level: 2
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.pool" }
        abi: { type: "function", name: "supply", inputs: [], outputs: [] }
        args: {}
`
  );

  await writeFile(
    join(TEST_DIR, 'safe-defi.ais-pack.yaml'),
    `
schema: "ais-pack/0.0.2"
name: safe-defi
version: "0.0.2"
includes:
  - protocol: uniswap-v3
    version: "0.0.2"
`
  );

  await writeFile(
    join(TEST_DIR, 'swap-flow.ais-flow.yaml'),
    `
schema: "ais-flow/0.0.2"
meta:
  name: swap-flow
  version: "0.0.2"
inputs:
  token:
    type: address
nodes:
  - id: swap
    type: action_ref
    skill: "uniswap-v3@0.0.2"
    action: swap
    args:
      token: { ref: "inputs.token" }
`
  );

  // Invalid file
  await writeFile(join(TEST_DIR, 'invalid.ais.yaml'), 'invalid: yaml: content:::');

  // Plugin execution type (unknown)
  await writeFile(
    join(TEST_DIR, 'plugin-unknown.ais.yaml'),
    `
schema: "ais/0.0.2"
meta:
  protocol: plugin-unknown
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:
  a:
    description: "a"
    risk_level: 1
    execution:
      "eip155:*":
        type: "some_plugin_exec"
        foo: { lit: 1 }
`
  );

  // Non-AIS file (should be ignored)
  await writeFile(join(TEST_DIR, 'readme.md'), '# Test');
});

afterAll(async () => {
  await rm(TEST_DIR, { recursive: true, force: true });
});

describe('loadFile', () => {
  it('loads and parses a protocol file', async () => {
    const doc = await loadFile(join(TEST_DIR, 'uniswap-v3.ais.yaml'));
    expect(doc.schema).toBe('ais/0.0.2');
    if (doc.schema === 'ais/0.0.2') {
      expect(doc.meta.protocol).toBe('uniswap-v3');
    }
  });

  it('loads and parses a pack file', async () => {
    const doc = await loadFile(join(TEST_DIR, 'safe-defi.ais-pack.yaml'));
    expect(doc.schema).toBe('ais-pack/0.0.2');
  });

  it('loads and parses a workflow file', async () => {
    const doc = await loadFile(join(TEST_DIR, 'swap-flow.ais-flow.yaml'));
    expect(doc.schema).toBe('ais-flow/0.0.2');
  });
});

describe('loadProtocol', () => {
  it('loads a protocol spec', async () => {
    const protocol = await loadProtocol(join(TEST_DIR, 'uniswap-v3.ais.yaml'));
    expect(protocol.meta.protocol).toBe('uniswap-v3');
    expect(protocol.actions.swap).toBeDefined();
  });
});

describe('loadDirectory', () => {
  it('loads all AIS files from directory', async () => {
    const result = await loadDirectory(TEST_DIR);

    expect(result.protocols).toHaveLength(2);
    expect(result.packs).toHaveLength(1);
    expect(result.workflows).toHaveLength(1);
    expect(result.errors).toHaveLength(2);
  });

  it('respects recursive: false option', async () => {
    const result = await loadDirectory(TEST_DIR, { recursive: false });

    expect(result.protocols).toHaveLength(1);
    expect(result.packs).toHaveLength(1);
    expect(result.workflows).toHaveLength(1);
  });

  it('collects errors for invalid files', async () => {
    const result = await loadDirectory(TEST_DIR);
    expect(result.errors).toHaveLength(2);
    expect(result.errors.some((e) => e.path.includes('invalid.ais.yaml'))).toBe(true);

    const pluginErr = result.errors.find((e) => e.path.includes('plugin-unknown.ais.yaml'));
    expect(pluginErr).toBeTruthy();
    expect(pluginErr!.kind).toBe('plugin_unknown_execution');
    expect(pluginErr!.execution_type).toBe('some_plugin_exec');
  });
});

describe('loadDirectoryAsContext', () => {
  it('creates context with registered protocols', async () => {
    const { context, result } = await loadDirectoryAsContext(TEST_DIR);

    expect(result.protocols).toHaveLength(2);
    expect(context.protocols.has('uniswap-v3')).toBe(true);
    expect(context.protocols.has('aave-v3')).toBe(true);
  });
});
