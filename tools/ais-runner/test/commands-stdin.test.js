import test from 'node:test';
import assert from 'node:assert/strict';

import { consumeCommandLines } from '../dist/runner/engine/commands-stdin.js';

test('consumeCommandLines emits accepted and rejected events', async () => {
  const sdk = {
    parseAisJson: JSON.parse,
    applyRuntimePatches: () => {},
    validateRunnerCommand: (payload) => {
      if (payload.kind === 'apply_patches') {
        return {
          ok: true,
          command: {
            id: payload.id,
            ts: payload.ts,
            kind: payload.kind,
            payload: payload.payload,
          },
        };
      }
      return {
        ok: false,
        error: { reason: 'unsupported kind', field_path: 'kind' },
      };
    },
    summarizeCommand: (command) => ({ id: command.id, ts: command.ts, kind: command.kind }),
  };

  const lines = (async function* () {
    yield '{"id":"c1","ts":"2026-02-12T00:00:00.000Z","kind":"apply_patches","payload":{"patches":[]}}';
    yield '{"id":"c2","ts":"2026-02-12T00:00:00.000Z","kind":"unknown","payload":{}}';
    yield '{"id":"c1","ts":"2026-02-12T00:00:00.000Z","kind":"apply_patches","payload":{"patches":[]}}';
  })();

  const events = await consumeCommandLines({
    sdk,
    plan: { nodes: [] },
    context: { runtime: { policy: {}, ctx: {} } },
    lines,
    pausedNodeIds: new Set(),
    seenCommandIds: new Set(),
  });

  assert.equal(events.events[0].type, 'patch_applied');
  assert.equal(events.events[1].type, 'command_accepted');
  assert.equal(events.events[1].command.id, 'c1');
  assert.equal(events.events[2].type, 'command_rejected');
  assert.equal(events.events[2].field_path, 'kind');
  assert.equal(events.events[3].type, 'command_rejected');
  assert.equal(events.events[3].reason, 'duplicate command id');
});

test('consumeCommandLines rejects apply_patches on blocked root namespace', async () => {
  const applied = [];
  const sdk = {
    parseAisJson: JSON.parse,
    applyRuntimePatches: (_ctx, patches) => {
      const rejectedPath = patches.find((p) => String(p.path || '').startsWith('nodes.'));
      if (rejectedPath) {
        const err = new Error('Runtime patch rejected by guard');
        err.details = { path: rejectedPath.path, reason: 'nodes blocked' };
        throw err;
      }
      applied.push(...patches);
    },
    validateRunnerCommand: (payload) => ({
      ok: true,
      command: {
        id: payload.id,
        ts: payload.ts,
        kind: payload.kind,
        payload: payload.payload,
      },
    }),
    summarizeCommand: (command) => ({ id: command.id, ts: command.ts, kind: command.kind }),
  };

  const lines = (async function* () {
    yield '{"id":"c-block","ts":"2026-02-12T00:00:00.000Z","kind":"apply_patches","payload":{"patches":[{"op":"set","path":"nodes.n1.outputs.ok","value":true}]}}';
  })();

  const result = await consumeCommandLines({
    sdk,
    plan: { nodes: [{ id: 'n1' }] },
    context: { runtime: { policy: {}, ctx: {} } },
    lines,
    pausedNodeIds: new Set(['n1']),
    seenCommandIds: new Set(),
  });

  assert.equal(result.events.length, 2);
  assert.equal(result.events[0].type, 'patch_rejected');
  assert.equal(result.events[1].type, 'command_rejected');
  assert.match(result.events[1].reason, /guard/);
  assert.equal(applied.length, 0);
});

test('consumeCommandLines user_confirm approve sets runtime approval and requests rerun', async () => {
  const sdk = {
    parseAisJson: JSON.parse,
    applyRuntimePatches: () => {},
    validateRunnerCommand: (payload) => ({
      ok: true,
      command: {
        id: payload.id,
        ts: payload.ts,
        kind: payload.kind,
        payload: payload.payload,
      },
    }),
    summarizeCommand: (command) => ({ id: command.id, ts: command.ts, kind: command.kind }),
  };

  const context = { runtime: { policy: {}, ctx: {} } };
  const lines = (async function* () {
    yield '{"id":"c-ok","ts":"2026-02-12T00:00:00.000Z","kind":"user_confirm","payload":{"node_id":"n1","approve":true}}';
  })();

  const result = await consumeCommandLines({
    sdk,
    plan: { nodes: [{ id: 'n1', source: { protocol: 'demo@0.0.2', action: 'swap' } }] },
    context,
    lines,
    pausedNodeIds: new Set(['n1']),
    seenCommandIds: new Set(),
  });

  assert.equal(result.accepted_count, 1);
  assert.equal(result.rerun_requested, true);
  assert.equal(result.cancel_requested, false);
  assert.equal(context.runtime.policy.runner_approvals.n1.approved, true);
});
