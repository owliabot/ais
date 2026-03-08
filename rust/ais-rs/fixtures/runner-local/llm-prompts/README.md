# runner-local prompt overrides

This fixture shows how to externalize `ais-runner` prompts via markdown files.

Set `llm.controller_prompts_dir` in runner config:

```yaml
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: ${OPENROUTER_API_KEY}
  controller_prompts_dir: rust/ais-rs/fixtures/runner-local/llm-prompts/prompts
  operator_templates_dir: rust/ais-rs/fixtures/runner-local/llm-prompts/operator-templates
chains:
  eip155:1:
    rpc_url: ${EVM_RPC_URL}
```

Supported controller prompt files in `prompts/`:

- `agent.controller.system.md`
- `segmented.base_rules.md`
- `segmented.contracts_summary.md`
- `segmented.phase.begin.md`
- `segmented.phase.grounding.md`
- `segmented.phase.todos.md`
- `segmented.phase.propose.md`
- `segmented.phase.revise.md`
- `segmented.begin.patch.md` (JSON object, deep-merged into begin prompt payload)
- `segmented.grounding.patch.md` (JSON object, deep-merged into grounding prompt payload)
- `segmented.todos.patch.md` (JSON object, deep-merged into todo planning payload)
- `segmented.segment.patch.md` (JSON object, deep-merged into propose/revise payload)

Supported operator template files in `operator-templates/`:

- `operator.missing_input.header.md`
- `operator.missing_input.question.md`
- `operator.need_user_confirm.help.md`
- `operator.output.summary.md`

Notes:

- Missing/invalid files do not break runtime; runner falls back to built-in prompts.
- For list prompts, each non-empty line becomes one rule (markdown bullet prefixes are normalized).
- These prompt overrides do not own the stable issue taxonomy. Operator/audit semantics such as `missing_action_gate_dep`, `missing_gate_data_backing`, `stale_volatile_fact`, and `missing_token_decimals` are runner contracts documented in `rust/ais-rs/crates/ais-runner/README.md`, not freeform prompt text.
