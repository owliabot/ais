# runner-local prompt overrides

This fixture shows how to externalize `ais-runner` prompts via markdown files.

Set `llm.prompts_dir` in runner config:

```yaml
schema: ais-runner/0.0.1
llm:
  provider: openrouter
  model: openai/gpt-4.1-mini
  api_key: ${OPENROUTER_API_KEY}
  prompts_dir: rust/ais-rs/fixtures/runner-local/llm-prompts/prompts
chains:
  eip155:1:
    rpc_url: ${EVM_RPC_URL}
```

Supported prompt files in `prompts/`:

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

Notes:

- Missing/invalid files do not break runtime; runner falls back to built-in prompts.
- For list prompts, each non-empty line becomes one rule (markdown bullet prefixes are normalized).
