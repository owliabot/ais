# AIS Document Types Overview

Source specs directory: `/home/ocbot/.openclaw/workspace/repos/ais/specs/`

This summary covers the six AIS document families used by agents and runners.

## 1) Protocol (`ais/0.0.2`)

Spec: `specs/ais-1-protocol.md`

Purpose:
- Describe one protocol's actions, queries, deployments, and optional metadata.

Key shape:
- `schema: "ais/0.0.2"`
- `meta` (protocol id/version and metadata)
- `deployments[]` (chain + contract addresses)
- `actions` (required)
- `queries` (optional)
- optional `capabilities_required`, `supported_assets`, `risks`, `tests`

Notes:
- Strict schema; unknown fields are rejected.
- Extension data must be inside `extensions`.
- Actions/queries map to AIS-2 execution specs by chain.

## 2) Pack (`ais-pack/0.0.2`)

Spec: `specs/ais-1-pack.md`

Purpose:
- Bundle protocol includes with policy, approvals, constraints, token policy, providers/plugins, and overrides.

Key shape:
- `schema: "ais-pack/0.0.2"`
- `meta`
- `includes[]` (protocol/version/source/chain scope)
- `policy` (approval and install policy, CEL constraints)
- optional `token_policy`, `providers`, `plugins`, `overrides`

Notes:
- Strict schema; unknown fields rejected.
- `plugins.execution.enabled` acts as allowlist for plugin execution types.
- Approval mode (`safe|assist|yolo`) changes confirmation behavior but does not bypass hard blocks or allowlists.

## 3) Workflow (`ais-flow/0.0.3`)

Spec: `specs/ais-1-workflow.md`

Purpose:
- Define DAG orchestration over query/action operations with dependencies and control checks.

Key shape:
- `schema: "ais-flow/0.0.3"`
- `meta`
- `nodes[]`
- optional `default_chain`, `imports.protocols[]`, `requires_pack`, `inputs`, `policy`, `preflight`, `outputs`

Node types:
- `query_ref`
- `action_ref`
- `assert`
- `branch`

Notes:
- Workflow is strict; use `extensions` for host-specific data.
- Chain resolution must yield one concrete CAIP-2 chain.
- Cycles are invalid; engines must reject them.
- `assert`/`branch` are planner-readable control labels, lowered by compile to executable refs.

## 4) Plan (`ais-plan/0.0.3`)

Spec: `specs/ais-2-plan.md`

Purpose:
- Runner/engine execution contract (plan-first runtime artifact).

Key shape:
- `schema: "ais-plan/0.0.3"`
- `nodes[]`
- optional `meta`, `extensions`

Execution node essentials:
- `id`, `chain`, `kind`, `execution`
- optional lifecycle/control fields (`deps`, `condition`, `assert`, `until`, `retry`, `timeout_ms`)
- optional runtime fields (`writes`, `bindings`, `source`)

Notes:
- `execution.type` is routed through registered handlers.
- Core vs plugin execution types are enforced with runtime registry and pack allowlist.
- `replace_plan` mutation should maintain auditable plan hash lineage.

## 5) Plan Sketch (`ais-plan-sketch/0.1.0`)

Spec: `specs/ais-2-plan-sketch.md`

Purpose:
- LLM-facing, segmented planning IR used before deterministic compilation to `ais-plan/0.0.3`.

Key shape:
- `schema: "ais-plan-sketch/0.1.0"`
- `intent`
- `pack_snapshot`, `catalog_snapshot`
- `segments[]`
- optional `chain_scope`, `session`, `meta`, `extensions`

Segment and step model:
- segment: `segment_id`, `cursor_in`, `cursor_out`, `done`, `steps[]`
- step: `id`, `kind` (`query|action|assert|branch`), `candidate_ref`, `inputs`
- optional: `depends_on`, `stores`, `when`, `until`, `retry`, `timeout_ms`, `constraint_templates`

Notes:
- Not directly executable by engine.
- Host must compile to `ais-plan` deterministically.
- Unknown fields are rejected at root/segment/step levels.

## 6) Catalog (`ais-catalog/0.0.1`)

Spec: `specs/ais-1-catalog.md`

Purpose:
- Search/index cards for actions, queries, and packs so agents can discover capabilities without loading full specs.

Key shape:
- `schema: "ais-catalog/0.0.1"`
- `created_at`, `hash`
- optional `documents`
- `actions[]`, `queries[]`, `packs[]`

Notes:
- Authority schema is index-only; detail payloads are fetched separately by `ref`.
- Requires stable sorting and canonical hashing for cache/diff reliability.

## Related Specs

- Core index: `specs/index.md`
- Overview: `specs/ais-0-overview.md`
- Types and expressions: `specs/ais-1-types.md`, `specs/ais-1-expressions.md`
- Execution specifics: `specs/ais-2-*.md`
