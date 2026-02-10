# Workflow Module (AIS 0.0.2)

This module contains helpers for working with `ais-flow/0.0.2` workflows as a DAG.

## DAG utilities

- `buildWorkflowDag(workflow, { include_implicit_deps })`
  - Produces a stable topological order.
  - Includes explicit `node.deps` plus inferred dependencies from `ValueRef` references to `nodes.*` (when `include_implicit_deps=true`).
  - Implicit deps are inferred from `args`, `condition`, `calculated_overrides`, and `until`.
  - Throws `WorkflowDagError` on cycles, unknown deps, or duplicate node ids.
