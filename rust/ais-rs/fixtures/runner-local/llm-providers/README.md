# runner-local LLM provider configs

These templates are for `ais-runner agent --profile standard` and demonstrate
how to configure different LLM backends in `ais-runner/0.0.1` config.

## Usage

Pick one config file under `config/` and pass it to `--config`:

- `openrouter.config.yaml`
- `groq.config.yaml`
- `anthropic.config.yaml`

All templates use `${...}` env placeholders for API keys.

Example:

`ais-runner agent --plan <plan.json> --config rust/ais-rs/fixtures/runner-local/llm-providers/config/openrouter.config.yaml --profile standard`

Intent example (with local fixture workspace):

`ais-runner agent --intent-file rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt --workspace rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace --pack rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml --config rust/ais-rs/fixtures/runner-local/llm-providers/config/openrouter.config.yaml --profile standard`

## Troubleshooting

- `llm provider failed: missing api key`: set corresponding env var (`OPENROUTER_API_KEY` / `GROQ_API_KEY` / `ANTHROPIC_API_KEY`).
- `runner config load failed`: confirm `${EVM_RPC_URL}` is exported.
- Provider returns no tool calls: switch to a tool-calling capable model and keep prompt focused on plan output.
