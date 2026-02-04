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
ais_version: "1.0"
type: protocol
protocol:
  name: uniswap-v3
  version: "1.0.0"
  chain_id: 1
  addresses:
    router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
actions:
  - name: swap
    contract: router
    method: swap
    inputs: []
`
  );

  await writeFile(
    join(TEST_DIR, 'subdir', 'aave-v3.ais.yaml'),
    `
ais_version: "1.0"
type: protocol
protocol:
  name: aave-v3
  version: "1.0.0"
  chain_id: 1
  addresses:
    pool: "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"
actions:
  - name: supply
    contract: pool
    method: supply
    inputs: []
`
  );

  await writeFile(
    join(TEST_DIR, 'safe-defi.ais-pack.yaml'),
    `
ais_version: "1.0"
type: pack
pack:
  name: safe-defi
  version: "1.0.0"
protocols:
  - protocol: uniswap-v3
    version: "1.0.0"
`
  );

  await writeFile(
    join(TEST_DIR, 'swap-flow.ais-flow.yaml'),
    `
ais_version: "1.0"
type: workflow
workflow:
  name: swap-flow
  version: "1.0.0"
inputs:
  - name: token
    type: address
steps:
  - id: swap
    uses: uniswap-v3/swap
    with:
      token: "\${input.token}"
`
  );

  // Invalid file
  await writeFile(join(TEST_DIR, 'invalid.ais.yaml'), 'invalid: yaml: content:::');

  // Non-AIS file (should be ignored)
  await writeFile(join(TEST_DIR, 'readme.md'), '# Test');
});

afterAll(async () => {
  await rm(TEST_DIR, { recursive: true, force: true });
});

describe('loadFile', () => {
  it('loads and parses a protocol file', async () => {
    const doc = await loadFile(join(TEST_DIR, 'uniswap-v3.ais.yaml'));
    expect(doc.type).toBe('protocol');
    if (doc.type === 'protocol') {
      expect(doc.protocol.name).toBe('uniswap-v3');
    }
  });

  it('loads and parses a pack file', async () => {
    const doc = await loadFile(join(TEST_DIR, 'safe-defi.ais-pack.yaml'));
    expect(doc.type).toBe('pack');
  });

  it('loads and parses a workflow file', async () => {
    const doc = await loadFile(join(TEST_DIR, 'swap-flow.ais-flow.yaml'));
    expect(doc.type).toBe('workflow');
  });
});

describe('loadProtocol', () => {
  it('loads a protocol spec', async () => {
    const protocol = await loadProtocol(join(TEST_DIR, 'uniswap-v3.ais.yaml'));
    expect(protocol.protocol.name).toBe('uniswap-v3');
    expect(protocol.actions[0].name).toBe('swap');
  });
});

describe('loadDirectory', () => {
  it('loads all AIS files from directory', async () => {
    const result = await loadDirectory(TEST_DIR);

    expect(result.protocols).toHaveLength(2); // uniswap-v3 + aave-v3 (in subdir)
    expect(result.packs).toHaveLength(1);
    expect(result.workflows).toHaveLength(1);
    expect(result.errors).toHaveLength(1); // invalid.ais.yaml
  });

  it('respects recursive: false option', async () => {
    const result = await loadDirectory(TEST_DIR, { recursive: false });

    expect(result.protocols).toHaveLength(1); // only uniswap-v3
    expect(result.packs).toHaveLength(1);
    expect(result.workflows).toHaveLength(1);
  });

  it('collects errors for invalid files', async () => {
    const result = await loadDirectory(TEST_DIR);
    expect(result.errors).toHaveLength(1);
    expect(result.errors[0].path).toContain('invalid.ais.yaml');
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
