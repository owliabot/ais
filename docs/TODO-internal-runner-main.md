# TODO: Internal Runner `main` (Workflow/Action/Query) Implementation

Scope: implement an **internal verification runner** (not public SDK API) as described in `docs/design-ts-sdk-internal-runner-main.md`.

Non-goals:
- Do not change AIS spec files for runner convenience.
- Do not turn runner into a production wallet/app.
- Keep all chain IO and signing pluggable and auditable.

Conventions:
- Task IDs: `RUN-###`
- Priority: `P0` (must-have), `P1` (should), `P2` (nice)
- Status: `todo | doing | done | blocked`

---

## Milestones

### M0: Skeleton + Dry-Run (no network)
Runner can load workspace + parse target + build plan + print what would run.

### M1: EVM Read/Write End-to-End (real RPC optional)
Runner can execute `evm_read` and (optionally) `evm_call` with receipt polling and strict success checks.

### M2: Solana Read/Instruction End-to-End (real RPC optional)
Runner can execute `solana_read` and (optionally) `solana_instruction` with confirmation checks.

### M3: Workflow Outputs + Checkpoint/Resume + Trace
Runner supports `--checkpoint/--resume`, trace JSONL, and prints evaluated workflow outputs.

### M4: Protocol Action Preflight (requires_queries + calculated_fields)
Runner can run actions that rely on `runtime.query[...]` and `runtime.calculated.*` (e.g. Uniswap examples).

### M5: Policy Gates (risk approvals, broadcast safety)
Runner enforces pack/workflow policy gates and requires explicit broadcast opt-in.

---

## Task Index (high level)

| ID | Pri | Status | Area | Short description | Depends |
|---:|:---:|:------:|:-----|:------------------|:--------|
| RUN-001 | P0 | done | Repo | Add `tools/ais-runner/` scaffold | - |
| RUN-002 | P0 | done | CLI | Implement CLI args + modes | RUN-001 |
| RUN-003 | P0 | done | Config | Implement runner config loader (YAML + env) | RUN-001 |
| RUN-004 | P0 | doing | Workspace | Load workspace + validate references | RUN-002 |
| RUN-005 | P0 | done | Runtime | Inputs/ctx injection + BigInt coercion | RUN-002 |
| RUN-006 | P0 | doing | Plan | Build plan from workflow/action/query | RUN-004,RUN-005 |
| RUN-007 | P0 | done | Exec | Chain router executor (exact chain match) | RUN-003 |
| RUN-008 | P0 | done | Exec | Strict success wrapper (EVM/Solana) | RUN-007 |
| RUN-009 | P0 | done | Engine | Wire `runPlan()` with checkpoint + trace | RUN-006,RUN-007 |
| RUN-010 | P0 | done | Output | Evaluate workflow outputs ValueRefs | RUN-009 |
| RUN-011 | P1 | done | Compat | QueryResult fanout: nodes -> runtime.query | RUN-009 |
| RUN-012 | P1 | done | Preflight | Action preflight for requires_queries | RUN-011 |
| RUN-013 | P1 | done | Preflight | calculated_fields evaluation | RUN-012 |
| RUN-014 | P1 | done | Policy | Risk approval gate + `--yes` override | RUN-003,RUN-004 |
| RUN-015 | P2 | done | DX | JSON schema for runner config + validation | RUN-003 |
| RUN-016 | P1 | done | Tests | Unit tests for config/runtime/router | RUN-003,RUN-005,RUN-007 |
| RUN-017 | P1 | done | Tests | Integration tests with mock RPC/signers | RUN-009 |
| RUN-018 | P1 | done | Docs | Add `tools/ais-runner/README.md` usage | RUN-002,RUN-003 |
| RUN-019 | P2 | todo | Detect | Dynamic detect providers integration (quotes/routes) | RUN-013 |

---

## Detailed TODOs

### RUN-001 (P0) Add runner scaffold

Status: `done`

Deliverables:
- Create `tools/ais-runner/` with:
  - `package.json` (type=module, scripts for build/run)
  - `tsconfig.json`
  - `src/` tree per design

Done when:
- `node tools/ais-runner/dist/main.js --help` runs after build.

Notes:
- Keep runner separate from `ts-sdk/src/cli` to avoid changing public CLI surface.

---

### RUN-002 (P0) CLI argument parsing + modes

Status: `done`

CLI shape (suggested):
- `ais-runner run workflow --file <.ais-flow.yaml> --workspace <dir> [--inputs <json>] [--ctx <json>]`
- `ais-runner run action --ref <protocol@ver>/<actionId> --workspace <dir> --args <json>`
- `ais-runner run query --ref <protocol@ver>/<queryId> --workspace <dir> --args <json> [--until <cel>] [--retry <json>]`
- Common flags:
  - `--config <yaml>`
  - `--checkpoint <path> --resume`
  - `--trace <path>`
  - `--dry-run`
  - `--broadcast` (default false)
  - `--yes` (auto-approve gates)

Done when:
- All modes parse and print a normalized “run request” object (no execution yet).

---

### RUN-003 (P0) Runner config loader (YAML + env expansion)

Status: `done`

Requirements:
- Load YAML config (see design doc section 5.1).
- Expand `${ENV}` placeholders.
- Provide defaults for `engine.max_concurrency` and `engine.per_chain`.
- Validate required keys:
  - `chains[<caip2>].rpc_url`
  - signer config presence when `--broadcast` is true

Implementation hints:
- Reuse `yaml` dependency (already in `ts-sdk` deps) or add to runner’s package.
- Keep config parsing independent of `ts-sdk` (tool layer).

Done when:
- `ais-runner run ... --config ...` loads and prints resolved per-chain config with env expanded.

---

### RUN-004 (P0) Workspace load + validation

Status: `doing`

Steps:
- Use `loadDirectory(dir)` or `loadDirectoryAsContext(dir)` from SDK.
- Run validators:
  - `validateWorkspaceReferences({protocols,packs,workflows})`
  - `validateWorkflow(workflow, ctx)`
- Decide how runner chooses which workflow to run when given a workspace:
  - `--file` explicit path (recommended)
  - Optional: `--workflow <name@ver>` selection if multiple

Done when:
- Runner prints clear errors with file path + field_path and exits non-zero when invalid.

Progress:
- Runner loads workspace via `ts-sdk/dist` and validates workspace/workflow.
- Workspace issues are filtered to the selected workflow + its referenced pack/protocols to avoid unrelated workspace errors blocking a run.

---

### RUN-005 (P0) Runtime injection + type coercion (BigInt safety)

Status: `done`

Requirements:
- Inject `inputs` into `ctx.runtime.inputs` from:
  - `--inputs <json>` and/or `--input key=value` flags
- Inject `ctx` into `ctx.runtime.ctx` from:
  - `--ctx <json>`
  - Derived fields:
    - `ctx.wallet_address` (from EVM signer)
    - `ctx.now` (unix seconds) for calculated fields
- Coerce numeric input types:
  - For workflow input types `uint*`, parse as `bigint` from string/number input.
  - For token amounts/decimals, keep strings unless explicitly meant to be `bigint`.

Done when:
- Running with example inputs never introduces JS `number` in execution-critical paths (uint256).

Progress:
- Workflow mode: coerces `ctx.runtime.inputs` by `workflow.inputs[*].type` (uint/int -> bigint).
- Action/query modes: resolves protocol param schema and coerces `--args` accordingly.
- Default-injects `ctx.now` (unix seconds) if missing (as bigint).

---

### RUN-006 (P0) Plan building for workflow/action/query modes

Status: `doing`

Workflow mode:
- Parse workflow file (`loadWorkflow` or `parseWorkflow`).
- `buildWorkflowExecutionPlan(workflow, ctx, { default_chain? })`.

Action mode:
- Resolve `protocol@ver` + action id from workspace context.
- Build a synthetic workflow with one `action_ref` node:
  - `default_chain` from CLI/config or node’s chain
  - `nodes[].args` from `--args` converted to `{ lit: ... }` ValueRefs

Query mode:
- Same synthetic workflow with one `query_ref` node.
- Support optional `until/retry/timeout_ms` from CLI for polling.

Done when:
- Runner prints plan nodes in stable order and their deps.

Progress:
- Workflow mode: loads workflow file, validates, builds `ExecutionPlan`, prints stable order.
- Action/query modes: synthesize a single-node workflow from `--ref` + `--args` and build a plan (requires `--chain` currently).
- Added `--dry-run` compile-only path: readiness + solver auto-fill + compile, no RPC.

---

### RUN-007 (P0) Chain router executor (exact chain match)

Status: `done`

Problem to solve:
- `EvmJsonRpcExecutor.supports()` matches any `eip155:*`, so multi-EVM-chain setups can mis-route requests.

Deliverable:
- `ChainRouterExecutor` wrapper:
  - Holds `Map<string, Executor>`
  - `supports(node)` returns true only if map has `node.chain` and the inner executor supports it
  - `execute(...)` dispatches strictly by `node.chain`

Done when:
- A plan containing nodes for `eip155:1` and `eip155:8453` always routes to correct RPC.

Progress:
- Runner creates per-chain executors and wraps them in a chain-bound `supports()` check, so `pickExecutor()` cannot mis-route across chains.
- Implementation lives in `tools/ais-runner/src/executors.ts`.

---

### RUN-008 (P0) Strict success wrapper (receipt/confirmation checks)

Status: `done`

Deliverable:
- `StrictSuccessExecutor` wrapper:
  - For EVM `evm_call`: if receipt is present, require `status` truthy/`0x1` (handle hex string).
  - For Solana instruction: if confirmation indicates error, treat as failure.

Done when:
- Failed transactions surface as `EngineEvent.error` with actionable message.

Progress:
- Implemented `StrictSuccessExecutor` wrapper in runner (`tools/ais-runner/src/executor-wrappers.ts`).
- EVM: if `outputs.receipt.status` indicates failure, throw error.
- Solana: if `outputs.confirmation.value.err` is present, throw error.

---

### RUN-009 (P0) Wire engine run (checkpoint + trace + safety flags)

Status: `done`

Steps:
- Create `FileCheckpointStore` using SDK codec:
  - `serializeCheckpoint()` / `deserializeCheckpoint()`
- Configure `runPlan(plan, ctx, options)` with:
  - `solver` (default `solver` or `createSolver({auto_fill_contracts:true})`)
  - `executors` (via `ChainRouterExecutor` + strict wrapper)
  - `detect` optional (future tasks)
  - `trace` via `createJsonlTraceSink({file_path})`
- Implement safety:
  - `--dry-run`: do not allow executors to broadcast (wrap signer away)
  - `--broadcast`: required for write nodes to actually broadcast

Done when:
- Runner can execute `examples/bridge-send-wait-deposit.ais-flow.yaml` in dry-run mode and show node scheduling.

Progress:
- Runner now supports a non-`--dry-run` path that calls `runPlan()` and streams engine events.
- Executors are created from config RPC URLs; write nodes still require signers (currently results in `need_user_confirm`).
- Added `--checkpoint` + `--resume` support via `FileCheckpointStore` (`tools/ais-runner/src/checkpoint-store.ts`).
- Added broadcast gate wrapper so writes require explicit `--broadcast` (emits `need_user_confirm` with a compiled tx preview).
- Added broadcast fail-fast: when `--broadcast` is set, runner requires `config.chains[chain].signer` for any chain that has write nodes in the plan.

---

### RUN-010 (P0) Evaluate workflow outputs at end

Status: `done`

Deliverable:
- After plan completion, compute `workflow.outputs` (ValueRefs):
  - Use `evaluateValueRef()` against final `ctx`
  - Print to stdout and optionally write JSON file

Done when:
- Example workflows print output fields matching `outputs:` block.

Progress:
- Runner evaluates `workflow.outputs` after `runPlan()` completes without pause/error and prints a `workflow_outputs` JSON object.
- Optional `--out <path>` writes the same payload to disk.

---

### RUN-011 (P1) QueryResult fanout: `nodes.*` outputs -> `runtime.query[...]`

Status: `done`

Motivation:
- Some protocol actions/calculated fields reference `query["<queryId>"]` (see `examples/uniswap-v3.ais.yaml`),
  while workflow execution writes query outputs to `nodes.<nodeId>.outputs`.

Deliverable:
- A small wrapper around event handling:
  - On `EngineEvent.query_result` when `node.source.query` exists:
    - `ctx.runtime.query[node.source.query] = outputs`

Done when:
- Action execution that expects `runtime.query[...]` can read required query results when workflow includes those query nodes.

Progress:
- Runner now applies a side-effect on `query_result` events: when `node.source.query` exists, it writes outputs to `ctx.runtime.query[queryId]`.
- Implementation in `tools/ais-runner/src/run.ts`.

---

### RUN-012 (P1) Action preflight: ensure `requires_queries` satisfied

Status: `done`

Deliverable:
- Preflight layer for write nodes:
  - Resolve the action spec from `node.source.protocol/action`
  - If action has `requires_queries`:
    - If those queries exist in `ctx.runtime.query` already, ok
    - Else if `--auto-required-queries` is enabled:
      - For each required query, build and execute a synthetic query node (read-only) using action params
      - Write results into `ctx.runtime.query[queryId]`
    - Else emit `need_user_confirm` explaining missing required queries

Done when:
- Runner can execute actions that declare required queries without manual runtime patching (at least for simple cases).

Progress:
- Implemented `ActionPreflightExecutor` wrapper (`tools/ais-runner/src/executor-wrappers.ts`) that blocks write nodes when `requires_queries` are missing in `ctx.runtime.query`.
- Works with `RUN-011` fan-out so workflow query nodes satisfy `requires_queries` once executed.

---

### RUN-013 (P1) calculated_fields evaluation

Status: `done`

Deliverable:
- Evaluate protocol action `calculated_fields`:
  - Determine an evaluation order:
    - Use each field’s declared `inputs` list as dependencies
    - Toposort, detect cycles
  - Evaluate `expr` (ValueRef):
    - Use `evaluateValueRefAsync` for consistent behavior (runner-side)
  - Write results:
    - `ctx.runtime.calculated[<name>] = value`
    - `ctx.runtime.nodes[<nodeId>].calculated[<name>] = value` (optional but useful)

Done when:
- `examples/swap-to-token.ais-flow.yaml` can run (at least until it hits actual quote/detect providers) without missing `calculated.*` references.

Progress:
- Added a calculated_fields-aware solver wrapper that patches `runtime.calculated` before readiness re-check.

Note:
- Dynamic `{ detect: ... }` (provider IO for quotes/routes) is tracked separately in `RUN-019`.

---

### RUN-014 (P1) Policy gates: risk approvals + broadcast safety

Status: `done`

Deliverable:
- If workflow has `requires_pack`, load pack and its policy:
  - For each write node (action_ref / composite step), check action `risk_level` vs:
    - `policy.approvals.auto_execute_max_risk_level`
    - `policy.approvals.require_approval_min_risk_level`
  - If gate triggers:
    - emit `need_user_confirm` unless `--yes` is set
- Enforce `--broadcast` required for any write node.

Done when:
- Runner never broadcasts by default.
- Runner blocks high-risk actions unless explicitly approved.

Progress:
- Added `PolicyGateExecutor` (`tools/ais-runner/src/executor-wrappers.ts`) that reads pack policy (when workflow has `requires_pack`) and pauses write nodes with `need_user_confirm` unless `--yes` is set.

Verification (no real RPC required; uses a dummy localhost RPC URL):
1. Default safety (no broadcast): should pause on broadcast gate.
   - `node tools/ais-runner/dist/main.js run workflow --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml --workspace . --config tools/ais-runner/fixtures/policy-gate.config.yaml`
2. Policy gate (broadcast on, approval required): should pause with `policy approval required`.
   - `node tools/ais-runner/dist/main.js run workflow --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml --workspace . --config tools/ais-runner/fixtures/policy-gate.config.yaml --broadcast`
3. Auto-approve policy (broadcast on, `--yes`): policy gate should not pause (subsequent failure may occur when trying to send to localhost RPC).
   - `node tools/ais-runner/dist/main.js run workflow --file tools/ais-runner/fixtures/policy-gate-flow.ais-flow.yaml --workspace . --config tools/ais-runner/fixtures/policy-gate.config.yaml --broadcast --yes`

---

### RUN-015 (P2) Config schema + validation

Status: `done`

Deliverable:
- Define a small JSON schema or Zod schema for runner config.
- Validate at startup with good error messages.

Done when:
- Misconfigured `chains` section fails fast with pinpointed paths.

Progress:
- Added Zod-based validation in `tools/ais-runner/src/config-validate.ts` (loaded via `ts-sdk` dependency tree; no extra installs required).
- Wired validation into `loadRunnerConfig()` so errors include dot-paths like `chains.eip155:1.rpc_url`.

---

### RUN-016 (P1) Unit tests (config/runtime/router)

Status: `done`

Add tests for:
- `${ENV}` expansion correctness
- BigInt coercion rules (no accidental JS number)
- ChainRouterExecutor exact-match routing

Done when:
- `npm test` (runner package) passes in CI/local.

Progress:
- Added `node:test` based unit tests under `tools/ais-runner/test/` covering env expansion + config validation, BigInt coercion, and exact chain routing.

---

### RUN-017 (P1) Integration tests (mock RPC/signers)

Status: `done`

Approach:
- Reuse pattern from `ts-sdk/examples/engine-runner-demo.mjs`:
  - mock JSON-RPC transport for EVM
  - mock Solana connection for reads
- Execute a small workflow and assert:
  - events order includes `node_ready`, `query_result`, `tx_sent` (when broadcast enabled)
  - checkpoint is written and can resume

Done when:
- Integration test covers at least one EVM read+write and one polling `until` read node.

Progress:
- Added `tools/ais-runner/test/integration-evm.test.js`:
  - mock EVM JSON-RPC transport + signer, asserts `query_result`/`tx_sent`/`tx_confirmed`, and verifies checkpoint resume skips execution
  - mock EVM `until/retry` polling emits `node_waiting` and re-executes until true

---

### RUN-018 (P1) Tool README

Status: `done`

Deliverable:
- `tools/ais-runner/README.md`:
  - installation/build
  - config example
  - running examples from `/examples`
  - safety flags explained (`--dry-run`, `--broadcast`, `--yes`)

Done when:
- New developer can run `bridge-send-wait-deposit` dry-run from README steps.

Progress:
- Added a detailed, actionable README at `tools/ais-runner/README.md`.

---

### RUN-019 (P2) Dynamic detect providers integration (quotes/routes)

Status: `todo`

Deliverable:
- Add a runner-side detect provider system that can perform real IO (quote/route) using configured endpoints:
  - Support `best_quote` / `best_path` / `protocol_specific`
  - Respect pack `providers.detect.enabled` selection and priority
  - Allow disabling detect IO by default (safety)
- Wire the detect resolver into:
  - `runPlan(..., { detect })` readiness + polling `until`
  - calculated_fields evaluation path
  - EVM/Solana async compilers where needed

Done when:
- `examples/swap-to-token.ais-flow.yaml` can resolve `best_quote` detect and progress past quote-dependent nodes when RPCs are configured.
