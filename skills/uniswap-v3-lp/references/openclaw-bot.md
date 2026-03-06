# OpenClaw Bot 集成 — 自动调仓

## 架构

```
OpenClaw (oliwa bot)
  └── cron job (定时触发)
        └── exec: ais-runner run workflow  ← AIS 组装交易
              └── clawlet (signer)         ← 签名 + 广播到链
```

OpenClaw bot 通过 `cron` 工具定期调用 `ais-runner run workflow`，
AIS 组装并验证 calldata，交给 clawlet 签名发送。bot 本身不接触私钥。

---

## 设置 Cron 自动调仓

在 OpenClaw 中用 `cron` 工具创建定时任务：

```json
{
  "name": "uniswap-v3-lp-rebalance",
  "schedule": { "kind": "every", "everyMs": 300000 },
  "sessionTarget": "isolated",
  "payload": {
    "kind": "agentTurn",
    "message": "Run the Uniswap V3 LP rebalance workflow for token ID 12345 on Sepolia. Use exec to run: ais-runner run workflow --workflow /path/to/workspace/uniswap-v3-lp-rebalance.ais-flow.yaml --config /path/to/config/runner.sepolia.yaml --inputs '{\"token_id\": 12345, \"range_width_ticks\": 600, \"slippage_bps\": 50}' --format json. Report whether rebalancing occurred and any errors.",
    "timeoutSeconds": 120
  },
  "delivery": { "mode": "announce" }
}
```

关键字段说明：
- `everyMs: 300000` → 每 5 分钟触发一次（按需调整）
- `sessionTarget: "isolated"` → 在独立 session 中执行，不干扰主会话
- `payload.kind: "agentTurn"` → bot 执行 agentTurn，在其中 exec ais-runner
- `delivery.mode: "announce"` → 有结果时通知到 chat（调仓成功/失败均可见）

---

## 初始化步骤（由 bot 执行一次）

1. **确认 ais-runner 在 PATH 上**
   ```bash
   which ais-runner
   # 或从 rust/ais-rs 构建：
   # cargo install --path rust/ais-rs/crates/ais-runner
   ```

2. **准备 workspace**
   ```bash
   mkdir -p /path/to/workspace
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais.yaml         /path/to/workspace/
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp-rebalance.ais-flow.yaml /path/to/workspace/
   cp skills/uniswap-v3-lp/assets/uniswap-v3-lp.ais-pack.yaml    /path/to/workspace/
   cp skills/uniswap-v3-lp/assets/runner.sepolia.yaml             /path/to/config/
   # 填写 runner.sepolia.yaml 中的 rpc_url / clawlet endpoint / wallet_address
   ```

3. **手动跑一次验证**
   ```bash
   ais-runner run workflow \
     --workflow /path/to/workspace/uniswap-v3-lp-rebalance.ais-flow.yaml \
     --config /path/to/config/runner.sepolia.yaml \
     --inputs '{"token_id": 12345, "range_width_ticks": 600}' \
     --format json
   ```
   确认输出包含 `rebalanced: false`（in-range）或完整调仓结果。

4. **注册 cron job**（由 bot 调用 cron 工具，见上方 JSON）

---

## 多个 Position 管理

如需监控多个 position，为每个 token_id 分别注册一个 cron job，
或在 agentTurn message 里循环：

```
Run rebalance workflow for each of these positions on Sepolia:
- token_id: 12345, range_width_ticks: 600
- token_id: 67890, range_width_ticks: 1200
Use the same config and workspace paths. Report status for each.
```

---

## 输出解读

`ais-runner run workflow --format json` 输出示例：

```json
{
  "status": "ok",
  "outputs": {
    "rebalanced": false,
    "current_tick": -74832,
    "old_tick_lower": -75600,
    "old_tick_upper": -74400,
    "new_token_id": 12345
  }
}
```

- `rebalanced: false` → 价格在范围内，无链上操作
- `rebalanced: true` → 已调仓，`new_token_id` 为新 position 的 NFT ID

---

## 错误处理

| 错误 | 原因 | 处理 |
|------|------|------|
| `clawlet connection refused` | clawlet daemon 未运行 | 检查 clawlet 进程 |
| `insufficient funds` | 钱包 ETH 不足 | 充值测试 ETH |
| `approval failed` | token approve 失败 | 检查 token 余额 |
| `liquidity is 0` | position 已无流动性 | 跳过该 token_id |
