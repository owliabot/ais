# Runner local fixture: offchain APY plugin config template

This fixture provides a runner config template for `execution.type = offchain_apy_query`.

## Layout

- `config/offchain-apy.config.yaml`
- `plan/offchain-apy.plan.json`

## Use

1. Replace `chains.*.rpc_url` with your target chain RPC.
2. Replace `plugins.execution.offchain_apy_query.allowed_domains` with your trusted API domains.
3. Replace `plan/offchain-apy.plan.json` endpoint with your real APY API URL.
4. (Optional) tune `timeout_ms`, `max_retries`, `retry_backoff_ms`.

Example:

```bash
cargo run -p ais-runner -- run plan \
  --plan fixtures/runner-local/offchain-apy/plan/offchain-apy.plan.json \
  --config fixtures/runner-local/offchain-apy/config/offchain-apy.config.yaml \
  --format json
```
