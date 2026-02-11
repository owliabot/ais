import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { loadDirectory, validateWorkspaceReferences } from '../src/index.js';

const ROOT = '/tmp/ais-sdk-workspace-validator';

async function setupDir(name: string): Promise<string> {
  const dir = join(ROOT, name);
  await mkdir(dir, { recursive: true });
  return dir;
}

async function writeProtocol(dir: string, protocol: string, version: string, actionIds: string[] = ['swap']): Promise<void> {
  const actions = actionIds
    .map(
      (a) => `
  ${a}:
    description: "${a}"
    risk_level: 1
    execution:
      "eip155:*":
        type: evm_call
        to: { lit: "0x0000000000000000000000000000000000000000" }
        abi: { type: "function", name: "${a}", inputs: [], outputs: [] }
        args: {}
`
    )
    .join('');

  await writeFile(
    join(dir, `${protocol}.ais.yaml`),
    `
schema: "ais/0.0.2"
meta:
  protocol: ${protocol}
  version: "${version}"
deployments:
  - chain: "eip155:1"
    contracts: {}
actions:${actions}
`
  );
}

async function writePack(
  dir: string,
  name: string,
  version: string,
  includes: Array<{ protocol: string; version: string }>,
  detectEnabled?: Array<{ kind: string; provider: string }>
): Promise<void> {
  const providersBlock = detectEnabled
    ? `
providers:
  detect:
    enabled:
${detectEnabled.map((e) => `      - kind: ${e.kind}\n        provider: ${e.provider}`).join('\n')}
`
    : '';

  await writeFile(
    join(dir, `${name}.ais-pack.yaml`),
    `
schema: "ais-pack/0.0.2"
name: ${name}
version: "${version}"
includes:
${includes.map((i) => `  - protocol: ${i.protocol}\n    version: "${i.version}"`).join('\n')}
${providersBlock}
`
  );
}

async function writeWorkflow(dir: string, name: string, version: string, pack?: { name: string; version: string }, nodesYaml?: string) {
  await writeFile(
    join(dir, `${name}.ais-flow.yaml`),
    `
schema: "ais-flow/0.0.3"
meta:
  name: ${name}
  version: "${version}"
${pack ? `requires_pack:\n  name: ${pack.name}\n  version: "${pack.version}"\n` : ''}
default_chain: "eip155:1"
nodes:
${nodesYaml ?? ''}
`
  );
}

beforeEach(async () => {
  await mkdir(ROOT, { recursive: true });
});

afterEach(async () => {
  await rm(ROOT, { recursive: true, force: true });
});

describe('validateWorkspaceReferences', () => {
  it('passes for a consistent workspace', async () => {
    const dir = await setupDir('ok');
    await writeProtocol(dir, 'uniswap-v3', '0.0.2', ['swap']);
    await writePack(dir, 'safe-defi', '0.0.2', [{ protocol: 'uniswap-v3', version: '0.0.2' }], [
      { kind: 'choose_one', provider: 'builtin' },
    ]);
    await writeWorkflow(
      dir,
      'wf',
      '0.0.3',
      { name: 'safe-defi', version: '0.0.2' },
      `
  - id: swap
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap
    args:
      choice:
        detect:
          kind: choose_one
          provider: builtin
          candidates: [{ lit: 1 }, { lit: 2 }]
`
    );

    const loaded = await loadDirectory(dir, { recursive: true });
    expect(loaded.errors).toEqual([]);

    const issues = validateWorkspaceReferences({
      protocols: loaded.protocols,
      packs: loaded.packs,
      workflows: loaded.workflows,
    });
    expect(issues).toEqual([]);
  });

  it('errors when pack includes a missing/mismatched protocol version', async () => {
    const dir = await setupDir('pack-mismatch');
    await writeProtocol(dir, 'uniswap-v3', '0.0.2', ['swap']);
    await writePack(dir, 'safe-defi', '0.0.2', [{ protocol: 'uniswap-v3', version: '0.0.3' }]);

    const loaded = await loadDirectory(dir, { recursive: true });
    const issues = validateWorkspaceReferences({
      protocols: loaded.protocols,
      packs: loaded.packs,
      workflows: loaded.workflows,
    });

    expect(issues.some((i) => i.field_path === 'includes[0]' && i.message.includes('workspace has uniswap-v3 versions'))).toBe(true);
  });

  it('errors when workflow requires a missing pack', async () => {
    const dir = await setupDir('missing-pack');
    await writeProtocol(dir, 'uniswap-v3', '0.0.2', ['swap']);
    await writeWorkflow(
      dir,
      'wf',
      '0.0.3',
      { name: 'safe-defi', version: '0.0.2' },
      `
  - id: swap
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap
`
    );

    const loaded = await loadDirectory(dir, { recursive: true });
    const issues = validateWorkspaceReferences({
      protocols: loaded.protocols,
      packs: loaded.packs,
      workflows: loaded.workflows,
    });

    expect(issues.some((i) => i.field_path === 'requires_pack' && i.message.includes('missing pack'))).toBe(true);
  });

  it('errors when workflow uses detect not enabled in required pack', async () => {
    const dir = await setupDir('detect');
    await writeProtocol(dir, 'uniswap-v3', '0.0.2', ['swap']);
    await writePack(dir, 'safe-defi', '0.0.2', [{ protocol: 'uniswap-v3', version: '0.0.2' }], [
      { kind: 'choose_one', provider: 'builtin' },
    ]);
    await writeWorkflow(
      dir,
      'wf',
      '0.0.3',
      { name: 'safe-defi', version: '0.0.2' },
      `
  - id: swap
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: swap
    args:
      choice:
        detect:
          kind: best_quote
`
    );

    const loaded = await loadDirectory(dir, { recursive: true });
    const issues = validateWorkspaceReferences({
      protocols: loaded.protocols,
      packs: loaded.packs,
      workflows: loaded.workflows,
    });

    expect(issues.some((i) => i.field_path === 'nodes[0].args.choice' && i.message.includes('Detect(kind=best_quote)'))).toBe(true);
  });

  it('errors when workflow references a missing action', async () => {
    const dir = await setupDir('missing-action');
    await writeProtocol(dir, 'uniswap-v3', '0.0.2', ['swap']);
    await writePack(dir, 'safe-defi', '0.0.2', [{ protocol: 'uniswap-v3', version: '0.0.2' }]);
    await writeWorkflow(
      dir,
      'wf',
      '0.0.3',
      { name: 'safe-defi', version: '0.0.2' },
      `
  - id: x
    type: action_ref
    protocol: "uniswap-v3@0.0.2"
    action: missing
`
    );

    const loaded = await loadDirectory(dir, { recursive: true });
    const issues = validateWorkspaceReferences({
      protocols: loaded.protocols,
      packs: loaded.packs,
      workflows: loaded.workflows,
    });

    expect(issues.some((i) => i.field_path === 'nodes[0].action' && i.message.includes('Action not found'))).toBe(true);
  });
});
