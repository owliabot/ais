# Engine Module

Interfaces and utilities for running AIS execution plans with a decoupled loop:

- Planner: `buildWorkflowExecutionPlan()` → `ExecutionPlan`
- Readiness: `getNodeReadiness()` → missing refs / detect requirements
- Solver: patches missing runtime state (pluggable)
- Executor: performs chain IO (pluggable)
- Engine: orchestrates scheduling + checkpoint

## File Structure

| File | Purpose |
|------|---------|
| `patch.ts` | `RuntimePatch` (`set`/`merge`) + apply helpers (+ undo capture) |
| `types.ts` | `Solver` / `Executor` / `EngineEvent` / `CheckpointStore` interfaces |
| `runner.ts` | `runPlan()` reference runner (scheduling + checkpoint) |
| `json.ts` | Shared JSON codec (BigInt/Uint8Array/Error safe) |
| `checkpoint.ts` | Checkpoint JSON codec (uses `json.ts`) |
| `trace.ts` | Optional `ExecutionTraceSink` + JSONL trace helpers (audit/debug/recovery) |
| `adapters/` | Integration adapters (JSONL event stream + JSONL RPC peer) |
| `executors/evm-jsonrpc.ts` | Reference executor for EVM JSON-RPC (`eth_call`, raw tx send) |
| `executors/solana-rpc.ts` | Reference executor for Solana RPC (`sendRawTransaction`, `confirmTransaction`) |
| `solvers/solver.ts` | Minimal built-in solver (fills common missing refs, emits confirmations) |
| `index.ts` | Re-exports |

## Core API

### RuntimePatch

```ts
import { applyRuntimePatches } from '@owliabot/ais-ts-sdk';

applyRuntimePatches(ctx, [
  { op: 'set', path: 'inputs.amount', value: '1.0' },
  { op: 'merge', path: 'nodes.q1.outputs', value: { fee: 3000n } },
], { record_undo: true });
```

### Interfaces

```ts
import type { Solver, Executor, EngineEvent } from '@owliabot/ais-ts-sdk';
```

Notes:
- The engine core (runner + patch + checkpoint + trace) intentionally does **not** perform network IO.
- Network IO is performed by pluggable executors (e.g. `EvmJsonRpcExecutor`, `SolanaRpcExecutor`).
- The authoritative roadmap lives in `docs/TODO.md`.
  - `EvmJsonRpcExecutor` is a reference implementation for wiring plan nodes to a JSON-RPC transport (mockable in tests).
    - Supports `evm_read` / `evm_call` and the built-in plugin `evm_rpc` (read-only allowlist).
  - `solver` is a minimal built-in solver (use `createSolver()` to customize); it resolves `node.source.protocol` references when auto-filling `runtime.contracts`.

### runPlan()

```ts
import { runPlan, solver, EvmJsonRpcExecutor } from '@owliabot/ais-ts-sdk';

for await (const ev of runPlan(plan, ctx, { solver, executors: [executor] })) {
  // stream progress to agent/UI
  if (ev.type === 'node_waiting') {
    // polling: engine will re-run the node after next_attempt_at_ms
  }
  if (ev.type === 'need_user_confirm') {
    // node-level pause (engine will keep running other independent branches)
  }
  if (ev.type === 'engine_paused') {
    // no more progress possible until paused nodes are resolved externally
    break;
  }
}
```

Post-check behavior:
- `until` supports polling/retry semantics.
- `assert` is a single-shot post-execution check; falsy assertion produces an `error` event (uses `assert_message` when present).

### Async `{ detect: ... }` support

If your workflow uses `{ detect: ... }` ValueRefs that require async resolution (e.g. quotes/routes), pass a `detect` resolver to `runPlan(...)`.

When `detect` is provided, the runner uses async readiness evaluation and executors may compile using async compilers.

```ts
import { runPlan, createDetectProviderRegistry, createDetectResolver } from '@owliabot/ais-ts-sdk';

const registry = createDetectProviderRegistry();
registry.register('protocol_specific', 'my-provider', async (detect, ctx) => {
  // ...network IO...
  return detect.candidates?.[0] ?? { lit: '0' };
});

const detect = createDetectResolver(registry);

for await (const ev of runPlan(plan, ctx, { solver, executors: [executor], detect })) {
  // ...
}
```

### Node `params.*`

When building a plan from a workflow, `nodes[].args` are resolved into a per-node param object and exposed to execution specs as `params.*`.

- EVM: `evm_read` / `evm_call` compilation can reference `{ ref: "params.<name>" }`.
- Solana: `solana_instruction` compilation and `solana_read` params evaluation can reference `{ ref: "params.<name>" }`.

### Plan node `writes`

Plan nodes can carry explicit `writes` to control where executor outputs are patched into the runtime context.

The reference executors honor `writes` for both reads and writes, which is used by composite step expansion to record step outputs under `nodes.<parent>.outputs.steps.<step_id>`.

### ExecutionTraceSink (optional)

Trace is for **audit/debug/recovery**, not for feeding logs into an LLM context.

```ts
import { runPlan, createJsonlTraceSink } from '@owliabot/ais-ts-sdk';

const traceSink = createJsonlTraceSink({ file_path: './engine-trace.jsonl' });

for await (const ev of runPlan(plan, ctx, {
  solver,
  executors: [executor],
  trace: { sink: traceSink, run_id: 'run-1' },
})) {
  // ...
}
```

### Checkpoint JSON codec

If you persist checkpoints as JSON, use the codec helpers to preserve `bigint` and `Uint8Array`:

```ts
import { serializeCheckpoint, deserializeCheckpoint } from '@owliabot/ais-ts-sdk';

const raw = serializeCheckpoint(checkpoint, { pretty: true });
const restored = deserializeCheckpoint(raw);
```

### Adapters: JSONL event stream / RPC peer (optional)

For integrating `runPlan()` with external processes/systems, you can stream engine events as JSONL or use a minimal JSONL peer.

```ts
import { createEngineEventJsonlWriter, runPlan } from '@owliabot/ais-ts-sdk';

const writer = createEngineEventJsonlWriter({ file_path: './engine-events.jsonl' });
for await (const ev of runPlan(plan, ctx, { solver, executors: [executor] })) {
  writer.append(ev);
}
writer.close();
```
