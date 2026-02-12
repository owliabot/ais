# AIS Policy Gate 确认交互映射（AGT005B）

日期：2026-02-12  
范围：`tools/ais-runner` 的 `need_user_confirm.details` 结构  
目标：统一策略 gate 到用户确认 UI 的映射，减少前端/agent 自行拼装文案导致的漂移。

## 1. 默认确认粒度

- 默认粒度：`workflow_node`
- 结构：`confirmation_scope = { mode: 'workflow_node', key, alternatives }`
- 含义：
  - `key`：`workflow_node_id`（缺省回退 `node_id`）
  - `alternatives`：声明可选粒度（当前为 `action_key`、`tx_hash`）

该默认策略用于避免“一次确认放大到整类 action”导致越权，同时又避免“仅 tx hash”在预执行阶段无法稳定命中的问题。

## 2. 默认文案模板字段

`need_user_confirm.details.confirmation_template` 包含：

- `title`：确认标题（allowlist 或 policy gate）
- `summary`：摘要文案（可直接展示）
- `action`：
  - `action_ref`
  - `action_key`
  - `chain`
  - `execution_type`
  - `workflow_node_id`
- `risk`：
  - `level`（来自 gate input）
  - `hit_rules`（命中原因列表）
  - `thresholds`（pack policy 阈值摘要）
- `recommendation`：建议动作（继续前检查项/改参建议）

## 3. 映射规则

- `policy_allowlist`：
  - `title`: “需要确认：执行类型不在 allowlist”
  - `recommendation`: 引导改用允许的 provider/type 或取消
- `policy_gate`：
  - `title`: “需要确认：策略规则触发”
  - `recommendation`: 核对风险与阈值后确认，异常则取消并调整参数

## 4. runner 落地点

- 组装逻辑：`tools/ais-runner/src/runner/executors/wrappers/policy-gate.ts`
- 测试覆盖：`tools/ais-runner/test/executor-wrappers.test.js`
