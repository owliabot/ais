# AIS Policy Gate Input/Output 规范（AGT005A）

日期：2026-02-12  
范围：`ts-sdk`（`extractPolicyGateInput` / `enforcePolicyGate`）  
目标：把 Policy Gate 的输入输出定义成可校验、可解释、可审计的稳定契约。

## 1. 设计结论

- `PolicyGateInput` 是执行前风控输入快照，面向 runner/agent 与审计系统。
- `PolicyGateOutput` 是标准化决策结果，仅允许三种 `kind`：
  - `ok`
  - `need_user_confirm`
  - `hard_block`
- 输入字段统一区分两类“不完整”：
  - `missing_fields`：当前动作语义下必须存在但缺失（强缺失）。
  - `unknown_fields`：允许缺失但语义不确定（软缺失）。

## 2. 输入结构（PolicyGateInput）

核心字段：

- 身份与定位：`node_id`、`workflow_node_id`、`step_id`、`action_ref`、`action_key`
- 上下文：`chain`、`params`、`preview`
- 风险主字段：`risk_level`、`risk_tags`
- 资产与约束：`token_address`、`token_symbol`、`spend_amount`、`approval_amount`、`slippage_bps`、`unlimited_approval`
- 额外参与方：`spender_address`、`owner_address`、`mint_address`
- 可解释性：`field_sources`、`missing_fields`、`unknown_fields`

字段表示规则（精度）：

- 金额类（`spend_amount`/`approval_amount`）统一为“整数 base-unit 字符串”，避免浮点误差。
- `slippage_bps` 为整数（bps）。
- `chain` 使用 CAIP-2 字符串。
- 地址/公钥按链原生字符串表示，不做跨链归一化。

## 3. 空值语义（missing vs unknown）

- `required_missing`：字段在当前动作语义下必须存在，缺失进入 `missing_fields`。
  - 例如：`swap` 场景的 `slippage_bps`、`spend_amount`。
  - 例如：`approve` 场景的 `approval_amount`、`spender_address`。
- `allowed_unknown`：字段可缺失，但会进入 `unknown_fields`，用于触发额外确认或审计标记。
  - 例如：资产身份无法从 params/preview 推断时的 `token_identity`。
- `not_applicable`：当前动作不需要该字段。

## 4. 风险字段来源优先级

- `risk_level`：`action metadata` 优先，缺失时回退默认值（当前实现默认 `3`）。
- `risk_tags`：`action metadata` 与 `pack overrides` 合并去重。
- 资产与交易字段：`params` 优先于 `preview`（避免编译/推断覆盖用户显式输入）。

## 5. 输出结构（PolicyGateOutput）

- `ok=true` 时：`kind=ok`
- `ok=false` 时：
  - `kind=need_user_confirm`：可继续但必须显式确认
  - `kind=hard_block`：必须阻断
- `reason`：人类可读主因
- `details`：机器可读证据（例如 `gate_input`、`missing_fields`、`unknown_fields`、`violations`、`approval_reasons`）

## 6. 审计用途

- `field_sources` 提供每个关键字段的来源链路（`params`/`preview`/`action`/`pack_override`），便于复盘“为什么得到这个 gate 决策”。
- `missing_fields` 与 `unknown_fields` 分离后，可以区分“硬缺失导致不安全”与“信息不充分导致需确认”。

## 7. SDK 落地点

- Schema 与字典：`ts-sdk/src/policy/schema.ts`
- 提取与决策：`ts-sdk/src/policy/enforcement.ts`
- 导出入口：`ts-sdk/src/index.ts`
