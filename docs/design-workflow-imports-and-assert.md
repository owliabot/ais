# Design: Explicit Protocol Imports + Assert Checks (AIS Spec + ts-sdk)

Status: Proposal  
Target: AIS workflow spec + `ts-sdk` (parser/loader/validator/plan/engine)  
Motivation: Make workflows reproducible, auditable, and less error-prone by:
- requiring explicit protocol imports (unless builtin or manually registered), and
- providing a first-class `assert` check (fail-fast, readable errors).

This document describes *what to change* and *how to implement it*.

---

## 1. Goals

### 1.1 Explicit imports (protocols)

Provide a workflow-level mechanism to declare which protocols (protocol specs) the workflow depends on, including where to load them from.

Desired properties:
- Deterministic resolution: same workflow + same imports => same protocol set.
- Less accidental coupling: workflows do not depend on “whatever happens to be in the workspace”.
- Security/policy-friendly: easier to review allowed protocols/versions.
- Minimal runtime friction: a small set of “builtin” protocols can be available without import.

### 1.2 `assert` (fail-fast checks)

Provide a clear “this must be true” mechanism, distinct from `until` polling:
- `assert` evaluates once and fails immediately if false.
- `until` continues to exist for polling (read nodes) and asynchronous convergence.

---

## 2. Non-goals

- Not trying to replace `requires_pack` (packs remain the primary policy boundary).
- Not introducing a full module system, remote registries, or network fetching in the spec.
- Not making `evm_rpc` “unsafe unlimited RPC”: RPC escape hatches must remain gated (pack allowlist + executor allowlist).

---

## 3. Spec Changes

Because AIS schemas are strict, new fields should be introduced via a version bump.

### 3.1 Workflow schema version bump

Introduce a new workflow schema version:
- `schema: "ais-flow/0.0.3"` (proposed)

Engines/sdks should keep supporting `ais-flow/0.0.2` workflows unchanged.

### 3.2 `imports` at workflow root

Add an optional `imports` field to workflow root:

```yaml
schema: "ais-flow/0.0.3"
meta: { name: "...", version: "0.0.3" }
default_chain: "eip155:1"

imports:
  protocols:
    - protocol: "erc20@0.0.2"
      path: "examples/erc20.ais.yaml"
      integrity: "sha256-..."   # optional (recommended)
    - protocol: "uniswap-v3@0.0.2"
      path: "examples/uniswap-v3.ais.yaml"
```

Proposed semantics (spec-level, engine-agnostic):
- `imports.protocols[]` is declarative dependency metadata.
- Each entry maps a `protocol` (`<protocol>@<version>`) to a local file `path`.
- `integrity` (optional) is an integrity hint for toolchains (recommended for production).

Resolution rules (recommended for engines):
- A workflow’s effective protocol set is:
  1. builtin protocols (engine/sdk-defined),
  2. manually registered protocols (host-defined),
  3. imported protocols in `workflow.imports.protocols`.

If a workflow references a protocol not in the effective set, engines should fail with a clear error.

### 3.3 `assert` at node level

Add optional `assert` + `assert_message` to workflow nodes:

```yaml
nodes:
  - id: q_balance
    type: query_ref
    protocol: "erc20@0.0.2"
    query: "balance-of"
    args: { ... }
    assert: { cel: "nodes.q_balance.outputs.balance > 0" }
    assert_message: "balance must be > 0"
```

Semantics:
- `assert` is evaluated **after** the node executes successfully and after its outputs have been written to runtime.
- If `assert` is falsy, the node fails immediately (engine emits an error event and stops that branch).
- `assert` is NOT polling. It is a single-shot condition.

Interaction with `condition`/`until`:
- `condition` (pre): if false => node is skipped, `assert` is not evaluated.
- `assert` (post): evaluated once, fail-fast.
- `until` (post, read-only): if present, node can be retried until truthy.
  - If both `assert` and `until` exist, recommended evaluation order per attempt:
    1. run node execution
    2. apply outputs patch
    3. evaluate `assert` (fail-fast)
    4. evaluate `until` (poll or complete)

### 3.4 Execution plan IR (`ais-plan/0.0.3` or compatible extension)

The engine executes the *plan*, not the workflow. To support `assert`, the plan node must carry it.

Option A (recommended): bump plan schema version and add fields:
- `ExecutionPlanNode.assert?: ValueRef`
- `ExecutionPlanNode.assert_message?: string`

Option B: keep plan version but store under `extensions` (not recommended; weak typing and easy to miss).

---

## 4. ts-sdk Changes

### 4.1 Schema updates

Update:
- `ts-sdk/src/schema/workflow.ts`
  - Add `imports` to workflow root (for `ais-flow/0.0.3`).
  - Add `assert` + `assert_message` to `WorkflowNode`.
- `ts-sdk/src/execution/plan.ts`
  - Add `assert` + `assert_message` to `ExecutionPlanNodeSchema`.
  - Ensure workflow->plan copying:
    - For `query_ref` nodes: copy `assert` fields directly.
    - For `action_ref` nodes:
      - If action expands to composite steps, attach assert to the *last* plan node (the parent id node).

Update module READMEs per `ts-sdk/AGENTS.md` rule:
- `ts-sdk/src/schema/README.md`
- `ts-sdk/src/execution/README.md`
- `ts-sdk/src/engine/README.md`

### 4.2 Builtin protocols + manual registration

Current `ts-sdk` already supports manual registration:
- `registerProtocol(ctx, spec)` registers a protocol in memory.

To enforce “import required unless builtin or manually registered”, `ts-sdk` needs to distinguish protocol sources.

Proposed approach:
- Extend resolver context protocol registry to store metadata:
  - `source: "builtin" | "manual" | "import" | "workspace_scan"`
  - `origin_path?: string`
- Add API:
  - `registerBuiltinProtocol(ctx, spec)`
  - `registerImportedProtocol(ctx, spec, { path })`
  - keep `registerProtocol` as “manual”.

Then strict import enforcement becomes possible even if the loader scans a directory:
- scanning can still load `workspace_scan`, but it is *not allowed* unless the workflow imports it.

### 4.3 Loader: load workflow + imports

Add a new loader entry point (recommended):

```ts
loadWorkflowBundle(workflowPath, {
  enforce_imports: true,
  builtin_protocols: [...],
  pre_registered_protocols: [...], // optional
})
```

Behavior:
1. Parse workflow.
2. Register builtin + pre-registered protocols.
3. If `workflow.imports.protocols` exists, load those protocol spec files and register them as `source:"import"`.
4. Validate that every node protocol resolves to an allowed protocol:
   - allowed if source is builtin/manual/import
   - disallowed if only present via workspace_scan (when strict mode enabled)

Keep existing behavior as a “legacy convenience mode”:
- `loadDirectoryAsContext(dir)` remains for workspace-wide validation/testing.

### 4.4 Validator: enforce import policy

Add validation option:

```ts
validateWorkflow(workflow, ctx, { enforce_imports: true })
```

In strict mode:
- If node protocol is not present in ctx, error as today.
- Additionally, if protocol exists but `source === "workspace_scan"`, error:
  - “protocol used but not imported (and not builtin/manual)”

### 4.5 Engine: implement `assert`

Update `ts-sdk/src/engine/runner.ts`:
- After a node produces outputs and patches are applied, evaluate `node.assert` (if present).
- If falsy:
  - emit `error` event with message:
    - `assert_message` if present
    - else a default message including the CEL expression
  - mark as non-retryable

Important: `assert` should be supported for both reads and writes.

### 4.6 CLI / Runner UX (optional but recommended)

Runner (`tools/ais-runner`) improvements:
- Add a `--strict-imports` flag (default true for `ais-flow/0.0.3`).
- Add `--imports-only` mode that disables directory scan and relies solely on imports + builtins.

---

## 5. Backward Compatibility and Migration

### 5.1 Existing workflows (0.0.2)

- Continue to support `ais-flow/0.0.2`.
- No new fields allowed.
- Behavior remains “load workspace, resolve protocols from loaded protocols”.

### 5.2 New workflows (0.0.3)

- Encourage toolchains to:
  - require `imports` in CI/release builds, OR
  - require strict import mode when `imports` is present.

### 5.3 Suggested migration tool

Add a helper CLI command:
- `ais workflow add-imports --file <workflow> --workspace <dir>`

It can:
- scan node protocols
- locate matching protocol specs in workspace
- write `imports.protocols[]` entries

---

## 6. Security Considerations

Explicit imports reduces accidental dependency injection from untrusted files in a workspace.

If combined with RPC escape hatches like `evm_rpc`:
- `evm_rpc` must remain:
  - a plugin execution type (pack allowlist applies), and
  - executor-enforced to a safe allowlist of methods.

Do NOT allow `evm_rpc` to call write methods like `eth_sendRawTransaction` (those are already covered by `evm_call` and broadcast gates).

---

## 7. Concrete Examples

### 7.1 Workflow using `imports` + `assert`

```yaml
schema: "ais-flow/0.0.3"
meta: { name: demo, version: "0.0.3" }
default_chain: "eip155:1"

imports:
  protocols:
    - { protocol: "erc20@0.0.2", path: "examples/erc20.ais.yaml" }

inputs:
  token: { type: asset, required: true }
  owner: { type: address, required: true }

nodes:
  - id: q_bal
    type: query_ref
    protocol: "erc20@0.0.2"
    query: "balance-of"
    args:
      token: { ref: "inputs.token" }
      owner: { ref: "inputs.owner" }
    assert: { cel: "nodes.q_bal.outputs.balance > 0" }
    assert_message: "owner must have positive balance"
```

---

## 8. Implementation Checklist (ts-sdk)

1. Add `ais-flow/0.0.3` schema support + zod schemas.
2. Add `imports` and `assert` fields to workflow schema.
3. Add `assert` fields to plan IR and copy from workflow to plan nodes.
4. Implement engine evaluation of `assert`.
5. Implement “strict import mode”:
   - protocol source metadata
   - validation option
   - loader helper `loadWorkflowBundle(...)`
6. Update docs (schema/execution/engine/plugins/specs).
7. Add tests:
   - strict import rejects unimported protocol (even if present in scanned workspace)
   - assert fails with message
   - assert on write node (post-tx) works

---

## 9. Tracking TODO

Status legend:
- `[ ]` pending
- `[~]` in progress
- `[x]` done

TODOs:
- [x] IMP-001 Rename workflow node key: `skill` -> `protocol` (spec + SDK + runner + tests + docs)
  - Done when: repo has no `nodes[].skill` in any `ais-flow/0.0.3` workflow, and runner + engine resolve `node.source.protocol`.
- [x] IMP-002 Implement `imports.protocols` end-to-end
  - Done when: workflows using workspace-scanned protocols fail validation unless imported, while builtin/manual/import sources pass.
- [x] IMP-003 Implement engine evaluation of `assert` (fail-fast)
  - Done when: `node.assert` is evaluated after outputs patch; falsy emits `error` with `assert_message` (or a default) and stops branch.
- [x] IMP-004 Update examples/fixtures to `ais-flow/0.0.3` + `protocol` + `imports`
  - Done when: all `.ais-flow.yaml` under `examples/` and `tools/ais-runner/fixtures/` validate under the new schema.
- [x] IMP-005 Add loader entrypoint: `loadWorkflowBundle(...)` (imports-only load)
  - Done when: caller can load workflow + only imported protocols (plus builtin/manual) without directory scan.
- [x] IMP-006 Add runner flags: `--strict-imports` and `--imports-only`
  - Done when: CLI supports imports-only mode and strict import enforcement can be toggled.
- [x] IMP-007 Update conformance vectors and golden files (`specs/conformance/*`)
  - Done when: vectors + golden plan JSON reflect `protocol` and new schema versions.
- [x] IMP-008 Update docs to reflect `protocol` terminology
  - Done when: spec/docs no longer instruct the legacy node field in workflows; all examples use `protocol:`.
