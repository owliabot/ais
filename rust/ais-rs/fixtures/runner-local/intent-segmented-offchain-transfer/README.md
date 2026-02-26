# intent-segmented-offchain-transfer

End-to-end segmented intent fixture for `AISNEXT-TEST-004`:

- segment-1: query native/token balances via `offchain_apy_query`
- handoff: persist balance outputs in checkpoint/runtime
- segment-2: build transfer step and pause at `need_user_confirm`

This fixture is intended for automated tests with deterministic in-process executor outputs (no HTTP dependency).

Additional scripted template for `AISRS-CTRL-010`:

- `llm/segmented.until-retry.repair.template.jsonl`
- flow: `plan.propose_segment` emits invalid `until` shape with `retry/timeout_ms`, then `plan.revise_segment` repairs `until` to valid ValueRef and keeps runtime controls.

Additional scripted template for `AISRS-MIN-008`:

- `llm/segmented.format-repair.template.jsonl`
- flow: malformed string `segment` output triggers planner revise; then cross-segment `depends_on` compile error triggers second revise; repaired segment compiles with `assert/branch` control step kinds and local dependencies.
