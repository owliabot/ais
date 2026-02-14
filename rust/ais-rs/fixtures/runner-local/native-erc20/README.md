# Runner local fixture: native + ERC20 transfer assert

This fixture bundle is copied from `tools/ais-runner/fixtures` for direct use under `rust/ais-rs`.

## Layout

- `workspace/native-and-erc20-transfer-assert.ais-flow.yaml`
- `workspace/evm-native-utils.ais.yaml`
- `workspace/erc20.ais.yaml`
- `workspace/safe-defi.ais-pack.yaml`
- `config/policy-gate.config.yaml`

## Run (from `rust/ais-rs`)

```bash
cargo run -p ais-runner -- run workflow \
  --workflow fixtures/runner-local/native-erc20/workspace/native-and-erc20-transfer-assert.ais-flow.yaml \
  --workspace fixtures/runner-local/native-erc20/workspace \
  --config fixtures/runner-local/native-erc20/config/policy-gate.config.yaml \
  --format json
```

Dry-run:

```bash
cargo run -p ais-runner -- run workflow \
  --workflow fixtures/runner-local/native-erc20/workspace/native-and-erc20-transfer-assert.ais-flow.yaml \
  --workspace fixtures/runner-local/native-erc20/workspace \
  --dry-run --format json
```
