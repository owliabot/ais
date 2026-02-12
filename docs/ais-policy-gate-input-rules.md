# AGT010A: Policy Gate Input 提取规则

本文件定义 `extractPolicyGateInput(...)` 的字段来源优先级，以及“不确定字段”处理策略。

## 1. 字段可得性分级

- `hard_block_fields`:
  - 执行前必须可得；非空时 `enforcePolicyGate` 直接 `hard_block`。
  - 当前默认包含：
    - `chain` 缺失
    - `action_ref` 缺失
    - `preview_compile`（预览编译失败）
- `missing_fields`:
  - 业务上应提供但可人工确认继续；默认触发 `need_user_confirm`。
- `unknown_fields`:
  - 无法确认完整语义（如 token identity 不明确）；默认触发 `need_user_confirm`。

## 2. 风险字段来源优先级

`risk_level`:

1. `runtime_risk_level`（运行时附加）
2. `pack.overrides.actions.*.risk_level`
3. `action_risk_level`（action 元信息）
4. 默认 `3`

`risk_tags`:

- 合并来源：`action_risk_tags + pack override risk_tags + runtime_risk_tags`
- 去重后写入 `risk_tags`

## 3. slippage/spend/approval 取值优先级

以下字段按统一优先级提取：

- `slippage_bps`
- `spend_amount`
- `approval_amount`

优先级：

1. `params`（显式参数）
2. `calculated`（计算结果）
3. `detect_result`（detect 输出）
4. `preview`（执行预览推断，作为兜底）

并在 `field_sources` 中记录命中来源，便于审计追踪。

## 4. 运行时附加输入建议

- 推荐 runner 在调用 `extractPolicyGateInput` 时传入：
  - `runtime_risk_level`
  - `runtime_risk_tags`
  - `detect_result`

这样可以避免各执行器自行拼接 gate 字段导致语义漂移。
