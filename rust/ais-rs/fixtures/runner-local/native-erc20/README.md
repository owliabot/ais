# Runner local fixture: native + ERC20 transfer assert

This fixture bundle is copied from `tools/ais-runner/fixtures` for direct use under `rust/ais-rs`.

## Layout

- `workspace/native-and-erc20-transfer-assert.ais-flow.yaml`
- `workspace/evm-native-utils.ais.yaml`
- `workspace/erc20.ais.yaml`
- `workspace/safe-defi.ais-pack.yaml`
- `config/policy-gate.config.yaml`

## Run (from `rust/ais-rs`)

Export the local signer key first:

```bash
export AIS_RUNNER_LOCAL_EVM_PRIVATE_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
```

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

## Troubleshooting

- `connection refused`: local RPC (`127.0.0.1:8545`) 未启动。
- `execution.type ... unregistered`: 检查 `--workspace` 是否传入本目录 `workspace/`。
- token transfer revert: 检查 token 地址、钱包余额与 signer 私钥是否匹配本地链账户。
- `missing env var for placeholder`: 检查 `AIS_RUNNER_LOCAL_EVM_PRIVATE_KEY` 是否已导出。

Intent + agent 剧本请参考：
`rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/README.md`
