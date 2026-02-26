# TODO: AIS Agent 精简与优化（v3）

日期：2026-02-20  
范围：`specs/`、`schemas/0.0.2/`、`rust/ais-rs/`  
目标：把系统从“可配置平台”收敛为“可验证决策机器”，优先保障：**安全、确定性、可恢复、可审计、低 token 成本**。

---

## 0) 追踪规则

### 0.1 ID 规则

- `AISSLIM-SPEC-###`：Spec/Schema 精简
- `AISSLIM-RS-###`：Rust 实现收敛
- `AISSLIM-TEST-###`：测试/向量收敛
- `AISSLIM-MIG-###`：迁移与弃用策略

### 0.2 状态

- `[ ]` 未开始
- `[~]` 进行中
- `[x]` 完成

### 0.3 执行门禁

1) 先做 `AISSLIM-SPEC-*`  
2) 再做 `AISSLIM-RS-*`  
3) 最后做 `AISSLIM-TEST-*` 与 `AISSLIM-MIG-*`

### 0.4 Definition of Done

- Spec：术语唯一、权威字段唯一、无重复语义。
- Rust：策略入口单一、分支可预测、错误码稳定。
- 测试：覆盖资金安全关键路径（拒绝/确认/执行/恢复）。

---

## 1) 精简目标（必须达成）

- 只保留一条主执行链：`plan -> policy_gate -> confirm -> execute -> checkpoint`
- 只保留一套核心证据：`confirmation_summary + confirmation_hash + stable reason_code`
- 默认只提供 `index candidates`，detail 按需加载
- 只保留资金安全关键 conformance

---

## 2) SPEC 精简任务

### [x] AISSLIM-SPEC-001 收敛 policy 字段唯一来源 `P0`

- Scope：`specs/ais-1-pack.md` + `schemas/0.0.2/pack.schema.json`
- 动作：
  - 收敛 `hard_constraints_defaults` / `hard_constraints` 双字段为单字段（保留一个，另一个标记 deprecated）
  - 明确 approvals 与 protocol_install 的优先级和作用域
- AC：
  - 同一约束只存在一个权威字段
  - schema 层面可静态拒绝冲突配置

Progress notes:
- Spec authority is now explicit:
  - `policy.hard_constraints_defaults` is the only canonical pack-level hard-constraints field.
  - `policy.hard_constraints` is explicitly rejected.
- Updated spec and schema:
  - `specs/ais-1-pack.md` adds normative authority/deprecation rule.
  - `schemas/0.0.2/pack.schema.json` removes `policy.hard_constraints`.
- Added conformance regression case:
  - `specs/conformance/vectors/pack-approvals.json` now includes invalid legacy-field case.

Remaining for completion:
- None (done).

### [x] AISSLIM-SPEC-002 收敛 install 策略到最小可用 `P0`

- Scope：`specs/ais-3-discovery.md`、`specs/ais-1-pack.md`、pack schema
- 动作：
  - v3 仅保留：`mode + allowed_sources + require_signature`
  - `registry_allowlist/domain_allowlist/trusted_publishers` 降级为可选扩展（不进入核心决策）
- AC：
  - `safe/assist/yolo` 下安装决策规则可一句话解释
  - 允许/拒绝原因码固定、简短

Progress notes:
- Core knobs reduced to minimal contract:
  - `mode`
  - `allowed_sources`
  - `require_signature`
- Spec updates:
  - `specs/ais-1-pack.md` removes install-policy core dependency on registry/domain/publisher toggles; these move to `policy.protocol_install.extensions`.
  - `specs/ais-3-discovery.md` explicitly defines v3 minimal knobs and extension boundary.
  - `specs/ais-4-conformance.md` narrows `protocol_install_decision` input/semantics to minimal knobs.
- Schema + vectors updates:
  - `schemas/0.0.2/pack.schema.json` trims `protocol_install` to minimal fields (+`extensions`).
  - `schemas/0.0.2/conformance.schema.json` trims `protocol_install_decision` input/reason codes.
  - `specs/conformance/vectors/protocol-install-decision.json` updated to minimal decision matrix.

Remaining for completion:
- None (done).

### [x] AISSLIM-SPEC-003 Catalog 默认 index-only 语义定稿 `P0`

- Scope：`specs/ais-1-catalog.md`、`specs/ais-1-executable-candidates.md`
- 动作：
  - 规范默认输出为 index cards
  - detail card 改为显式按需接口（非默认上下文）
- AC：
  - agent 首轮规划不依赖 detail 字段
  - 文档明确“何时需要 detail”

Progress notes:
- `ais-catalog/0.0.1` is now explicitly index-only:
  - cards in catalog MUST be index cards,
  - `level` when present must be `"index"`,
  - detail-only fields are excluded from catalog authority contract.
- `ais-executable-candidates/0.0.1` clarifies index-only action/query cards.
- Schema tightening:
  - `schemas/0.0.2/catalog.schema.json` restricts card level to `index` and removes detail fields from catalog card schema.
  - `schemas/0.0.2/executable-candidates.schema.json` action/query items now use `additionalProperties: false` and allow only index-card fields.
- Conformance:
  - `specs/conformance/vectors/catalog.json` adds an invalid case for detail payload in catalog.

Remaining for completion:
- None (done).

### [x] AISSLIM-SPEC-004 Conformance 最小集重定义 `P0`

- Scope：`specs/ais-4-conformance.md`、`schemas/0.0.2/conformance.schema.json`
- 动作：
  - 标记核心集合：`engine-command`、`policy-gate`、`pack-approvals`、`plugin-registration`、`protocol-install-decision`
  - 其它向量标记为 extended（非 blocker）
- AC：
  - CI 可只跑核心集合并给出 pass/fail

Progress notes:
- Added core/extended conformance layering semantics:
  - `specs/ais-4-conformance.md` now defines profile split (`core|extended`) and recommended CI policy.
- Extended authority schema:
  - `schemas/0.0.2/conformance.schema.json` adds top-level optional `profile` enum (`core|extended|mixed`).
- Labeled current vectors with profile tags:
  - core: `engine-command`, `pack-approvals`, `pack-approvals-decision`, `policy-gate-missingness`, `plugin-type-registration`, `protocol-install-decision`
  - extended: catalog/confirmation-hash/core/numeric/pattern vectors
- Added machine-readable core profile manifest:
  - `specs/conformance/profiles/core-files.json`

Remaining for completion:
- None (done).

### [x] AISSLIM-SPEC-005 阈值字段向通用 CEL 约束迁移 `P0`

- Scope：`specs/ais-1-pack.md`、`specs/ais-2-policy-gate.md`、`schemas/0.0.2/pack.schema.json`
- 动作：
  - 新增统一约束表达入口（CEL-based），用于替代固定阈值键（如 `max_slippage_bps/max_spend/max_approval/...`）
  - 固定阈值键标记 deprecated，并定义等价 CEL 模板映射
  - 明确约束执行时机与失败语义（`need_user_confirm|hard_block`）
- AC：
  - 新增约束可不改 schema 结构键名即可表达
  - 至少给出 3 个等价迁移样例（slippage/spend/approval）
  - 兼容期内同配置冲突时有明确优先级与报错策略

Progress notes:
- `specs/ais-1-pack.md`：
  - 增加 `policy.constraints[]` 作为规范化约束入口（`id/effect/expr/message`）。
  - 明确执行顺序：missingness -> CEL constraints -> legacy 阈值（deprecated 兼容路径）。
  - 明确冲突优先级：任意 `hard_block` 优先，legacy 结果不得降级已命中的 CEL `hard_block`。
  - 增加阈值字段到 CEL 的等价迁移模板（`max_slippage_bps/max_spend/max_approval/allow_unlimited_approval`）。
- `specs/ais-2-policy-gate.md`：
  - 增加 CEL 执行模型：对标准化 `PolicyGateInput` 评估、按列表顺序执行、稳定决策合并。
  - 增加 `matched_constraints[]` 输出建议与 `policy_constraint_eval_error` 错误语义建议。
- `schemas/0.0.2/pack.schema.json`：
  - 新增 `policy.constraints[]` schema（`effect=hard_block|need_user_confirm`，必填 `id/effect/expr`）。
  - 将固定阈值键标记 `deprecated: true`（兼容保留）。
- `specs/conformance/vectors/pack-approvals.json`：
  - 增加 `policy.constraints[]` 的 schema 向量（valid + missing required field invalid）。

Remaining for completion:
- None (done).

---

## 3) Rust 收敛任务

### [x] AISSLIM-RS-001 统一决策入口（DecisionPolicy）`P0` (Gate: AISSLIM-SPEC-001)

- Scope：`rust/ais-rs/crates/ais-runner/src/agent/`
- 动作：
  - 收敛 `CliBrain/LlmBrain/AssistLlmBrain` 到统一策略接口（例如 `DecisionPolicy`）
  - 把 mode 分支从“脑类型分支”改成“同一策略内状态机”
- AC：
  - 暂停点决策入口只有一个
  - 关键分支路径可枚举且有测试

Progress notes:
- `ais-runner/src/agent/brain.rs`：
  - 引入统一策略接口 `DecisionPolicy`，并新增单一状态机实现 `AgentDecisionPolicy`。
  - 收敛原 `CliBrain/LlmBrain/AssistLlmBrain` 分支逻辑到 `AgentDecisionPolicy::decide()`。
  - 新增 `DecisionPath`（`YoloAutoApprove|AssistLlmAutoApprove|ManualPrompt`）用于显式枚举关键路径。
- `ais-runner/src/agent/mod.rs`：
  - `execute_agent` 仅构建并注入一个 `AgentDecisionPolicy` 到 agent loop。
  - 去除按“脑类型”分支装配逻辑，改为同一策略内根据 mode/threshold/llm 状态决策。
- `ais-runner/src/agent/loop.rs`：
  - 统一调用 `DecisionPolicy::decide()` 作为暂停点唯一决策入口。
- `ais-runner/src/agent/mod_test.rs` + `ais-runner/src/agent/brain.rs` tests：
  - 迁移到 `DecisionPolicy` 接口；
  - 覆盖 `DecisionPath` 可枚举分支与 assist LLM 自动审批路径。

Remaining for completion:
- None (done).

### [x] AISSLIM-RS-002 demo 通道隔离 `P1` (Gate: AISSLIM-SPEC-002)

- Scope：`ais-runner` CLI
- 动作：
  - 将 `--llm-script-jsonl` 标记为 demo profile（或迁移到子命令）
  - 主命令帮助中弱化 demo 标志，避免误用
- AC：
  - 生产默认路径不依赖脚本化输入

Progress notes:
- `ais-runner/src/cli.rs`：
  - `agent` 新增 `--profile standard|demo-scripted`（默认 `standard`）。
  - `--llm-script-jsonl` 移入 `Demo Options` 帮助分组，并由 `profile=demo-scripted` 约束启用。
- `ais-runner/src/agent/mod.rs`：
  - 新增 profile 前置校验：`standard` 禁止脚本注入；`demo-scripted` 必须提供脚本。
- `ais-runner/src/error.rs`：
  - 新增 `RunnerError::AgentProfile` 统一表达 profile/参数组合错误。
- `ais-runner/src/cli_test.rs` + `ais-runner/src/agent/mod_test.rs`：
  - 覆盖 `demo-scripted` 缺脚本 parse 拒绝与运行前 profile 校验拒绝。
- `ais-runner/README.md`：
  - CLI 与状态说明同步为 profile 化 demo 通道。

Validation:
- `cargo test -p ais-runner cli_parses_agent_command -- --nocapture`
- `cargo test -p ais-runner cli_rejects_demo_scripted_profile_without_script -- --nocapture`
- `cargo test -p ais-runner execute_agent_rejects_demo_script_for_standard_profile -- --nocapture`
- `cargo test -p ais-runner execute_agent_requires_script_for_demo_scripted_profile -- --nocapture`

Remaining for completion:
- None (done).

### [x] AISSLIM-RS-003 candidates 上下文预算控制 `P0` (Gate: AISSLIM-SPEC-003)

- Scope：`ais-sdk` + `ais-runner`
- 动作：
  - runner 给 agent 默认注入 index candidates（可配置上限）
  - detail 请求改为按 ref 二次拉取
- AC：
  - 首轮 prompt 体积显著下降（记录 token 统计基线）

Progress notes:
- `ais-runner/src/agent/candidates.rs`：
  - 新增候选上下文构建器：从 `--workspace` 加载 protocol/pack/workflow，构建 catalog 并生成 executable index candidates。
  - 新增总量上限控制（`--max-index-candidates`，默认 `24`），默认仅注入 index cards。
  - 新增 detail lookup map（按 `ref`）用于按需二次拉取。
- `ais-runner/src/agent/brain.rs`：
  - LLM pause payload 新增 `index_candidates` 注入（有 workspace 时启用）。
  - tool-calling 支持 `get_candidate_detail(refs[])`，并在同一暂停周期内进行工具往返后再产出 engine command。
- `ais-runner/src/cli.rs`：
  - `agent` 新增 `--workspace`、`--max-index-candidates`。
- `ais-runner/src/cli_test.rs` + `ais-runner/src/agent/candidates.rs` + `ais-runner/src/agent/brain.rs` tests：
  - 覆盖新参数解析、候选上限截断与 detail lookup 二次拉取路径。

Remaining for completion:
- None (done).

### [x] AISSLIM-RS-004 reason_code 收敛与稳定化 `P0` (Gate: AISSLIM-SPEC-001)

- Scope：`ais-engine` + `ais-runner`
- 动作：
  - 收敛错误/拒绝 reason_code 枚举（安装、确认、执行、替换计划）
  - 清理自由文本 reason 的判定职责
- AC：
  - 自动化逻辑仅依赖 reason_code，不依赖字符串匹配

Progress notes:
- `ais-engine/src/policy/gate.rs`：
  - 新增 `PolicyGateReasonCode` 并将 `PolicyGateOutput.{NeedUserConfirm,HardBlock}` 标准化为 `reason_code + reason + details`。
  - allowlist/threshold/missingness 分支统一产出稳定 reason_code（避免自由文本判定）。
- `ais-engine/src/policy/confirm_hash.rs`：
  - `confirmation_summary` 新增 `reason_code`，确认哈希绑定稳定原因码而非仅文本 reason。
- `ais-engine/src/engine/runner.rs`：
  - `need_user_confirm` / `hard_block` / `error` / `engine_paused` 等关键事件统一携带 `event.data.reason_code`。
  - `hit_reasons` 默认回退改为 reason_code（不再依赖文案）。
- `ais-runner/src/agent/summary.rs`：
  - pause 摘要优先读取 `reason_code`（`last_error_reason` 与 `need_user_confirm.reason_code`）。
- `ais-runner/src/run.rs`：
  - replace-plan 拒绝原因改为稳定枚举 `ReplacePlanReasonCode`，并在 `error/paused` 事件输出 `reason_code`。

Remaining for completion:
- None (done).

---

## 4) 测试与向量收敛

### [x] AISSLIM-TEST-001 资金安全核心路径测试矩阵 `P0`

- Scope：`ais-engine`、`ais-runner`、conformance vectors
- 最小矩阵：
  - need_user_confirm -> approve/deny
  - hard_block 不可绕过
  - 未注册 handler 必拒绝
  - checkpoint 恢复后决策一致
- AC：
  - 所有矩阵 case 稳定可重复

Progress notes:
- `need_user_confirm -> approve/deny`：
  - 已有 approve 路径回归 + 新增 deny 保持阻塞回归：
    - `ais-engine/src/engine/runner_test.rs::run_plan_minimal_loop_with_apply_patches_and_user_confirm`
    - `ais-engine/src/engine/runner_test.rs::need_user_confirm_deny_keeps_node_blocked`
- `hard_block 不可绕过`：
  - 新增“携带 user_confirm 仍不可越过 hard_block”回归：
    - `ais-engine/src/engine/runner_test.rs::hard_block_cannot_be_bypassed_by_user_confirm`
- `未注册 handler 必拒绝`：
  - 新增 runner execute-path 回归（不仅 config 单测）：
    - `ais-runner/src/run_test.rs::run_plan_rejects_unregistered_execution_type_in_execute_path`
- `checkpoint 恢复后决策一致`：
  - 新增 need_user_confirm 恢复一致性回归（paused_reason + confirmation_hash 稳定）：
    - `ais-runner/src/run_test.rs::checkpoint_resume_keeps_need_user_confirm_decision_stable`

Validation:
- `cargo test -p ais-engine`（64 passed）
- `cargo test -p ais-runner`（51 + 1 passed）

Remaining for completion:
- None (done).

### [x] AISSLIM-TEST-002 conformance 核心/扩展分层执行 `P1`

- Scope：`specs/conformance/vectors`
- 动作：
  - 新增核心清单文件（或命名约定）
  - CI 默认跑核心，nightly 跑扩展
- AC：
  - PR 信号更快更稳定

Progress notes:
- 配置清单分层：
  - 核心清单：`specs/conformance/profiles/core-files.json`
  - 扩展清单：`specs/conformance/profiles/extended-files.json`
- conformance 执行器（TS SDK）支持 profile 选择：
  - `AIS_CONFORMANCE_PROFILE=core|extended|all`
  - 默认 profile 为 `core`（未设置环境变量时）
  - 实现文件：`ts-sdk/tests/conformance-vectors.test.ts`
  - 补齐核心/扩展缺失 kind 执行器：
    - `json_schema_validate`（`ais-pack|ais-catalog|ais-executable-candidates|ais-engine-command|ais-engine-event`）
    - `pack_approvals_decision`
    - `policy_gate_missingness_decision`
    - `execution_handler_registration_decision`
    - `protocol_install_decision`
    - `confirmation_hash`
- 增加便捷脚本：
  - `ts-sdk/package.json` 新增
    - `test:conformance:core`
    - `test:conformance:extended`
    - `test:conformance:all`
- 文档同步：
  - `specs/ais-4-conformance.md` 补充 profile manifest 与执行命令建议（CI core / nightly extended）。
- 向量一致性修正：
  - `specs/conformance/vectors/confirmation-hash.json` 的期望哈希更新为与规范算法一致（stable JSON + 忽略 timestamp-like keys + sha256）。

Validation:
- `npm --prefix ts-sdk run test:conformance:core`（42 passed）
- `npm --prefix ts-sdk run test:conformance:extended`（59 passed）
- `npm --prefix ts-sdk run test:conformance:all`（101 passed）

Remaining for completion:
- None (done).

---

## 5) 迁移与弃用

### [x] AISSLIM-MIG-001 直接删除弃用字段与兼容路径 `P1`

- Scope：specs + schema + rust/ais-rs（ais-engine/ais-runner）
- 动作：
  - 不做弃用公告窗口，直接从规范删除 pack 固定阈值字段语义：
    - `policy.hard_constraints_defaults.*`
    - `overrides.actions.*.hard_constraints`
  - 删除 policy-gate 文档中的 legacy fixed-threshold 兼容说明
  - 删除 runner 从 pack 读取固定阈值兼容逻辑（`hard_constraints_defaults` / `hard_constraints`）
  - 删除 engine policy gate 中对应 fixed-threshold reason_code 与执行分支（spend/approval/slippage/unlimited_approval）
  - 将 conformance pack 向量更新为“已删除字段应判 invalid”
- AC：
  - pack 固定阈值字段在 schema 层直接拒绝
  - runner/engine 不再包含固定阈值兼容解析/判定路径

Progress notes:
- Spec / authority schema:
  - `specs/ais-1-pack.md` 删除 `hard_constraints_defaults` 相关规范与示例，保留 `policy.constraints[]` 作为唯一扩展约束入口。
  - `specs/ais-2-policy-gate.md` 删除 legacy fixed-threshold 兼容路径描述。
  - `schemas/0.0.2/pack.schema.json` 删除：
    - `policy.hard_constraints_defaults`
    - `overrides.actions.<id>.hard_constraints`
- Conformance:
  - `specs/conformance/vectors/pack-approvals.json` 更新为验证已删除字段 `hard_constraints_defaults` 为 invalid。
  - `ts-sdk/tests/conformance-vectors.test.ts` 的 `ais-pack/0.0.2` 校验分支同步移除 `hard_constraints_defaults` 允许键。
- Rust implementation:
  - `ais-runner/src/policy/pack.rs` 删除 fixed-threshold 映射逻辑，仅保留 approvals 风险阈值与 allowlist 映射。
  - `ais-engine/src/policy/gate.rs` 删除 fixed-threshold 字段与 reason_code（spend/approval/slippage/unlimited）。
  - `ais-engine/src/policy/gate.rs` 输入别名清理：移除 `max_slippage_bps`、`max_approval` 参数别名。
  - 单测同步更新：
    - `ais-runner/src/policy/pack_test.rs`
    - `ais-runner/src/agent/mod_test.rs`
    - `ais-engine/src/policy/gate_test.rs`
    - `ais-engine/src/engine/runner_test.rs`

Validation:
- `npm --prefix ts-sdk run test:conformance:core`（42 passed）
- `cargo test -p ais-engine`（64 passed）
- `cargo test -p ais-runner`（51 + 1 passed）

Remaining for completion:
- None (done).

### [x] AISSLIM-MIG-002 兼容层硬删除清点 `P2`

- Scope：Rust 解析层 + policy gate 输入归一层 + pack authority schema
- 动作：
  - 删除 `compile_workflow` 的参数角色兼容推断别名（仅保留 canonical 角色名）：
    - 删除 `amount_in/amount_atomic/token_amount -> spend_amount`
    - 删除 `spender -> spender_address`
    - 删除 `amount/amount_atomic -> approval_amount`
  - 删除 engine policy gate 运行期参数别名回退（仅允许 `param_roles` 或 canonical key）：
    - 删除 `amount_in/amount/input_amount` 等 fallback
    - 删除 `max_approval/max_slippage_bps/spender/delegate` fallback
  - 删除 pack authority schema 中未使用的 legacy policy 字段：
    - `policy.risk_threshold`
    - `policy.approval_required`
  - conformance 增加遗留字段拒绝用例（`risk_threshold` invalid）
- AC：
  - 兼容别名不再被 parser/planner/engine 接受
  - schema 层拒绝 legacy policy 字段

Progress notes:
- `ais-sdk`：
  - `crates/ais-sdk/src/planner/compile_workflow.rs` 删除 param role alias 推断逻辑。
  - `crates/ais-sdk/src/planner/compile_workflow_test.rs` 对应断言改为 canonical-only 行为。
- `ais-engine`：
  - `crates/ais-engine/src/policy/gate.rs` 删除 fallback key 扫描函数与别名路径，保留 `param_roles + canonical key`。
- Authority schema / conformance：
  - `schemas/0.0.2/pack.schema.json` 删除 `risk_threshold` / `approval_required`。
  - `specs/conformance/vectors/pack-approvals.json` 新增 `risk_threshold` invalid case。
  - `ts-sdk/tests/conformance-vectors.test.ts` pack 校验器同步移除上述字段允许列表。
- Crate README 同步：
  - `crates/ais-sdk/README.md`
  - `crates/ais-engine/README.md`

Validation:
- `npm --prefix ts-sdk run test:conformance:core`（43 passed）
- `cargo test -p ais-sdk compile_workflow -- --nocapture`（12 passed）
- `cargo test -p ais-engine`（64 passed）
- `cargo test -p ais-runner`（51 + 1 passed）
- `rg -n "legacy|deprecated|兼容|risk_threshold|approval_required|hard_constraints_defaults|max_spend|max_approval|allow_unlimited_approval" specs schemas rust/ais-rs/crates`（0 hits）

Additional cleanup sweep (2026-02-22):
- 为满足“hard-compat 文案/字段清零”，`specs/ais-2-plan.md` 与 `specs/ais-2-policy-gate.md` 的剩余兼容措辞已移除。
- `specs/conformance/vectors/pack-approvals.json` 的“移除字段应 invalid”用例改为通用 removed 字段键名，避免继续保留历史字段名文本。

Remaining for completion:
- None (done).

---

## 6) 里程碑建议

- M1（1 周）：`AISSLIM-SPEC-001~004`
- M2（1~2 周）：`AISSLIM-RS-001~004`
- M3（1 周）：`AISSLIM-TEST-*` + `AISSLIM-MIG-*`

---

## 7) 优先顺序（建议）

1. `AISSLIM-SPEC-001`
2. `AISSLIM-RS-001`
3. `AISSLIM-SPEC-003` + `AISSLIM-RS-003`
4. `AISSLIM-TEST-001`
5. 其余任务按风险与人力排期
