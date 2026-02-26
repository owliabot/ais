# TODO: AIS Runner Intent → Plan → Execute 闭环

日期：2026-02-23  
范围：`specs/`、`schemas/0.0.2/`、`rust/ais-rs/crates/{ais-runner,ais-sdk,ais-llm,ais-engine}`、`rust/ais-rs/fixtures/runner-local/`
目标：让 `ais-runner` 支持一句自然语言 `intent`，自动完成 **规划、执行、确认、恢复** 的端到端演示链路。

---

## 0) 追踪规则

### 0.1 ID 规则

- `AISINT-SPEC-###`：规格与 schema
- `AISINT-RS-###`：Rust 实现
- `AISINT-TEST-###`：测试与 fixture
- `AISINT-DOC-###`：文档与示例

### 0.2 状态

- `[ ]` 未开始
- `[~]` 进行中
- `[x]` 完成

### 0.3 执行门禁

1) 先定 `SPEC`（intent 输入、planner 输出、错误码）  
2) 再做 `Runner` CLI 与 agent loop 改造  
3) 最后补测试矩阵与完整 fixture

---

## 1) 交付定义（Definition of Done）

- 用户可执行：  
  `ais-runner agent --intent "<自然语言>" --workspace <dir> --config <runner.yaml> --profile standard`
- Runner 能：
  - 加载 workspace 的 protocol/pack/workflow/catalog
  - 让 LLM 通过 tool-calling 产出可执行 plan（或 workflow 编译输入）
  - 进入执行循环，遇到 `need_user_confirm` 在 CLI 等待用户确认
  - 执行失败时将错误回传 LLM，支持多轮修正计划
- 端到端 fixture 覆盖：  
  “检查 native + erc20 余额都 >100，则给 B 转 5 native + 10 erc20”

---

## 2) SPEC 任务

### [x] AISINT-SPEC-001 `agent intent` 命令输入规范化 `P0`

- Scope：`specs/ais-2-plan.md`、`specs/ais-2-engine-commands.md`（必要时新增 `specs/ais-2-agent-intent.md`）
- 动作：
  - 定义 `intent` 输入最小 contract（`intent`, `constraints`, `target_chains` 可选）
  - 定义 `intent -> plan draft` 的结构化输出 contract
  - 定义失败分类：`intent_parse_error` / `intent_plan_unavailable` / `intent_plan_invalid`
- AC：
  - LLM 输出可被 schema 校验
  - 错误分类稳定可机读

Progress notes:

- 新增规范文档：`specs/ais-2-agent-intent.md`
  - 定义 `ais-agent-intent/0.0.1` 输入 contract
  - 定义 `ais-intent-plan-draft/0.0.1` 输出 contract
  - 固定失败 reason_code：`intent_parse_error|intent_plan_unavailable|intent_plan_invalid`
- 新增权威 schema：
  - `schemas/0.0.2/agent-intent.schema.json`
  - `schemas/0.0.2/intent-plan-draft.schema.json`
- 交叉引用补齐：
  - `specs/index.md` 加入 Agent Intent 文档入口
  - `specs/ais-2-plan.md`、`specs/ais-2-engine-commands.md` 增加 intent contract 引用

Validation:

- `jq . schemas/0.0.2/agent-intent.schema.json`
- `jq . schemas/0.0.2/intent-plan-draft.schema.json`

Remaining for completion:

- None (done).

### [x] AISINT-SPEC-002 tool-calling 规划协议定稿 `P0`

- Scope：`specs/ais-1-executable-candidates.md`、`specs/ais-2-engine-commands.md`
- 动作：
  - 规范 planner 可调用工具：`list_candidates` / `get_candidate_detail` / `propose_plan` / `revise_plan`
  - 规范单轮与多轮终止条件
  - 规范 plan 修订与 replace_plan 的边界（不可改已完成节点）
- AC：
  - 工具 I/O 可直接映射到现有 runner 结构
  - 与 `replace_plan` 语义一致，无冲突

Progress notes:

- `specs/ais-1-executable-candidates.md` 增加 planner 工具协议章节：
  - `list_candidates`
  - `get_candidate_detail`
  - 单轮确定性边界（snapshot hash）
- `specs/ais-2-engine-commands.md` 增加 intent planner 工具映射章节：
  - `propose_plan`
  - `revise_plan`
  - 与 `replace_plan` 的强约束衔接（完成节点不可改）
  - 规划回合终止建议（max rounds + repeated invalid early stop）

Validation:

- 规范交叉一致性检查：
  - planner tools 只负责产出 draft，不直接写引擎状态
  - 执行层统一通过既有 `replace_plan|cancel|user_confirm` 命令通道

Remaining for completion:

- None (done).

### [x] AISINT-SPEC-003 安全确认语义与风险分级对齐 `P0`

- Scope：`specs/ais-2-policy-gate.md`、`specs/ais-1-pack.md`
- 动作：
  - 明确 intent 模式下 `need_user_confirm` 默认行为
  - 明确 `safe|assist|yolo` 在 intent 模式下的差异
  - 明确 LLM 自动批准阈值与 reason_code
- AC：
  - 不出现“无确认自动转账”歧义
  - reason_code 可稳定回归测试

Progress notes:

- `specs/ais-2-policy-gate.md` 新增 intent-mode 规范章节：
  - `safe|assist|yolo` 在 transfer/write 下的默认确认行为
  - `constraints.must_confirm=true` 的强制人工确认优先级
  - 稳定 reason_code 建议：`intent_need_user_confirm` / `intent_assist_auto_approved` / `intent_yolo_auto_approved` / `intent_must_confirm`
- `specs/ais-1-pack.md` 增加 intent-mode overlay：
  - pack approvals mode 与 intent constraints 的叠加规则
  - assist 阈值与 yolo hard_block 不可绕过边界
- `specs/ais-2-agent-intent.md` 明确 `constraints.approvals_mode` 与 `must_confirm` 的优先级关系。

Validation:

- 规范一致性检查：
  - `must_confirm=true` 在三份文档中语义一致（强制人工确认）
  - `assist|yolo` 均不允许绕过 `hard_block`

Remaining for completion:

- None (done).

---

## 3) Rust 实现任务

### [x] AISINT-RS-001 CLI 增加 `--intent` 入口并与 `--plan` 互斥 `P0`

- Scope：`ais-runner/src/cli.rs`、`ais-runner/src/main.rs`
- 动作：
  - `agent` 子命令新增 `--intent <text>`
  - 与 `--plan` 做互斥与至少一项必填校验
  - `--intent-file`（可选）用于长文本
- AC：
  - CLI 帮助清晰可读
  - 参数错误在 parse 阶段即可拒绝

Progress notes:

- `ais-runner/src/cli.rs`：
  - `agent` 输入收敛为三选一：`--plan|--intent|--intent-file`
  - 使用 `ArgGroup(agent_input)` 强制“至少一个 + 互斥”
- `ais-runner/src/agent/mod.rs`：
  - 兼容现阶段执行能力：`--intent*` 暂返回 `NotImplemented`（后续 `AISINT-RS-002+` 落地）
- 测试补齐：
  - `cli_parses_agent_intent_command`
  - `cli_rejects_agent_with_both_plan_and_intent`
  - `cli_rejects_agent_without_plan_or_intent`
  - `execute_agent_intent_mode_is_not_implemented_yet`
- README 同步：
  - `ais-runner` command synopsis 改为 `(--plan|--intent|--intent-file)` 三选一。

Validation:

- `cargo test -p ais-runner cli_ -- --nocapture`
- `cargo test -p ais-runner execute_agent_intent_mode_is_not_implemented_yet -- --nocapture`

Remaining for completion:

- None (done).

### [x] AISINT-RS-002 Intent Planner 编排器（LLM tool-calling）`P0`

- Scope：`ais-runner/src/agent/`（建议拆 `intent.rs`）
- 动作：
  - 新增 `IntentPlanner` trait
  - 实现 `LlmIntentPlanner`：从 intent + candidates 生成初始 plan
  - 失败时接受执行错误上下文，生成 `replace_plan`
- AC：
  - 多轮收敛可控（max rounds）
  - 与现有 `AgentDecisionPolicy` 解耦

Progress notes:

- `ais-runner/src/agent/intent.rs`：
  - 新增 `IntentPlanner` trait（`propose_plan` / `revise_plan`）。
  - 新增 `LlmIntentPlanner<P: LlmProvider>`，基于 tool-calling 规划：
    - `list_candidates`
    - `get_candidate_detail`
    - `propose_plan`
    - `revise_plan`
  - 新增 `IntentPlanDraft` 结构化结果与计划解析（校验 `ais-plan/0.0.3`）。
- `ais-runner/src/agent/mod.rs`：
  - 新增 `resolve_agent_plan()`：当输入为 `--intent|--intent-file` 时，先走 intent 规划，再进入既有执行环。
  - 新增 `resolve_intent_text()`：统一处理 intent 文本/文件读取与非空校验。
  - 保持与 `DecisionPolicy` 解耦：intent 规划阶段与执行阶段（pause 决策）分离。
- `ais-runner/src/agent/mod_test.rs`：
  - 更新 intent 模式测试为“无 llm provider 时拒绝”。
- `ais-runner/src/agent/intent.rs` tests：
  - 覆盖候选查询后成功产出计划
  - 覆盖 invalid 状态返回解码

Validation:

- `cargo test -p ais-runner intent:: -- --nocapture`
- `cargo test -p ais-runner`

Remaining for completion:

- None (done).

### [x] AISINT-RS-003 规划结果校验与编译落地 `P0`

- Scope：`ais-sdk` + `ais-runner`
- 动作：
  - 对 LLM 生成计划执行 schema 校验
  - 接入 compile/validate 流程，失败回传结构化错误给 planner
  - 失败重试策略（上限 + 可观测原因）
- AC：
  - 非法计划不会进入执行引擎
  - 错误可被 LLM消费并修正

Progress notes:

- draft schema 校验：
  - `ais-schema` 注册新增：
    - `ais-agent-intent/0.0.1`
    - `ais-intent-plan-draft/0.0.1`
  - `ais-runner/src/agent/intent.rs` 在解析 `propose_plan/revise_plan` 输出时，先按 `intent-plan-draft` schema 校验，再进入具体状态分支。
- 计划编译/校验落地：
  - `proposed` 分支必须提供 `plan`，并通过 `ais-sdk parse_document(validate_schema=true)` 校验为 `ais-plan/0.0.3` 后才可执行。
- 失败重试策略（上限 + 可观测）：
  - `resolve_agent_plan()` 支持规划轮次重试：
    - 第一轮 `propose_plan`
    - 后续轮次 `revise_plan`
  - 新增 CLI 参数 `--max-planner-rounds`（默认 `3`，最小 `1`）。
  - 每轮失败通过结构化 `previous_error` 回传给 planner，并输出 round/reason 观测日志。
- 文档同步：
  - `ais-runner/README.md` 命令参数新增 `--max-planner-rounds`。

Validation:

- `cargo test -p ais-runner`
- 关键新增回归：
  - `agent::tests::execute_agent_intent_mode_retries_and_succeeds`

Remaining for completion:

- None (done).

### [x] AISINT-RS-004 执行中动态修订计划闭环 `P0`

- Scope：`ais-runner/src/agent/loop.rs`、`ais-runner/src/run.rs`
- 动作：
  - 当 executor/condition/assert 失败时，触发 planner 修订分支
  - 自动生成并发送 `replace_plan` command（符合现有保护）
  - 保留 checkpoint 一致性（`plan_epoch`、hash history）
- AC：
  - 失败→修订→继续执行链路可跑通
  - 已完成节点保护不被破坏

Progress notes:

- `ais-runner/src/agent/mod.rs`：
  - `execute_agent` 改为可重入执行环：当运行暂停且 `paused_reason` 命中可修复类型（`executor_error|assert_failed|condition_failed`）时，触发 intent planner `revise_plan`。
  - 新增 intent 修订上下文（`IntentRepairContext`），携带 intent 文本、planner 实例、修订轮次上限与计数。
  - 将执行失败上下文结构化为 `previous_error`（含 `paused_reason`、最后一条 `error` event、round）回传给 planner。
  - 自动构造并注入 `replace_plan` command（`confirmed=true`），复用既有 replace-guard。
- `ais-runner/src/run.rs`：
  - `process_replace_plan_commands` 与其返回结构提升为 `pub(crate)`，供 `agent` 与 `run plan` 共用同一 replace-plan 保护逻辑。
- `ais-runner/src/agent/loop.rs`：
  - `CommandBuilder` 改为由外层持有并跨多次 loop 调用复用，避免修订后命令 ID 重置导致 dedupe 冲突。
- checkpoint 一致性：
  - `agent` checkpoint 现在保存 `plan_snapshot`，并在 plan hash 变化恢复时回放 snapshot（保留 `plan_epoch` / `plan_hash_history` 连续性）。

Validation:

- `cargo test -p ais-runner agent::tests::execute_agent_intent_mode_can_repair_after_executor_error -- --nocapture`
- `cargo test -p ais-runner`

### [x] AISINT-RS-005 用户确认 CLI 体验收敛 `P1`

- Scope：`ais-runner/src/agent/brain.rs`、`summary.rs`
- 动作：
  - need_user_confirm 展示关键信息（资产、数量、目标地址、风险级别、reason_code）
  - 输入支持：`approve` / `deny` / `always_approve_this_run`（可选）
- AC：
  - 确认信息足够做安全判断
  - 输入行为稳定并可测试

Progress notes:

- `ais-runner/src/agent/summary.rs`：
  - `render_for_humans()` 在 `need_user_confirm` 场景增加关键确认信息展示：
    - `chain` / `action_ref` / `execution_type` / `risk_level`
    - 从 `confirmation_summary.details` 提取并展示 `amount` / `asset` / `target`（若可识别）
- `ais-runner/src/agent/brain.rs`：
  - CLI 交互新增 `always_approve_this_run|aa` 命令：
    - 当前节点立即 approve
    - 后续 `need_user_confirm` 节点在本次进程内自动 approve（仅本次 run，非持久化）
  - `help` 文案同步新增该命令说明。

Validation:

- `cargo test -p ais-runner agent::summary::tests:: -- --nocapture`
- `cargo test -p ais-runner agent::brain::tests:: -- --nocapture`

---

## 4) 测试与 Fixture 任务

### [x] AISINT-TEST-001 CLI/参数层测试 `P0`

- Scope：`ais-runner/src/cli_test.rs`
- 动作：
  - 覆盖 `--intent` 与 `--plan` 互斥
  - 覆盖 `--intent` 单独运行
  - 覆盖 `--intent-file` 优先级（如同时给定）
- AC：
  - 参数行为无歧义

Progress notes:

- `ais-runner/src/cli_test.rs` 新增：
  - `cli_parses_agent_intent_file_command`
  - `cli_rejects_agent_with_both_intent_and_intent_file`
- 现有互斥行为已覆盖：
  - `--plan` 与 `--intent` 互斥
  - 缺失 `--plan|--intent|--intent-file` 时报错

Validation:

- `cargo test -p ais-runner cli_ -- --nocapture`

### [x] AISINT-TEST-002 planner 回路测试 `P0`

- Scope：`ais-runner/src/agent/*_test.rs`、`ais-llm`
- 动作：
  - 脚本化 provider 模拟：首次计划失败、二次修订成功
  - 覆盖空工具调用、非法 plan、超出轮次
- AC：
  - 多轮规划行为可重复

Progress notes:

- `ais-runner/src/agent/intent.rs` 新增回归：
  - `llm_intent_planner_rejects_empty_tool_calls`
  - `llm_intent_planner_rejects_invalid_plan_payload`
  - `llm_intent_planner_fails_when_tool_round_limit_reached`
- 覆盖点：
  - 空工具调用
  - 非法 plan schema
  - tool round 上限耗尽

Validation:

- `cargo test -p ais-runner agent::intent::tests:: -- --nocapture`

### [x] AISINT-TEST-003 资金安全核心路径回归 `P0`

- Scope：`ais-runner` + `ais-engine`
- 动作：
  - 覆盖转账类节点强制确认（safe）
  - 覆盖 deny 后停留 blocked
  - 覆盖 assist 阈值内自动确认、阈值外人工确认
- AC：
  - 不可绕过确认门禁

Progress notes:

- `ais-runner/src/agent/mod_test.rs` 新增：
  - `agent_loop_stays_paused_after_user_deny`（deny 后保持 paused，节点不完成）
- `ais-runner/src/agent/brain.rs` 新增：
  - `assist_threshold_outside_range_falls_back_to_manual_path`
- 结合既有用例：
  - `llm_brain_can_auto_approve_need_user_confirm`
  - `assist_policy_uses_llm_for_low_risk_confirm`
  - `decision_path_is_enumerable`

Validation:

- `cargo test -p ais-runner agent::tests::engine_stays_paused_when_user_denies_confirmation -- --nocapture`
- `cargo test -p ais-runner agent::brain::tests:: -- --nocapture`

### [x] AISINT-TEST-004 端到端 fixture（native + erc20）`P0`

- Scope：`rust/ais-rs/fixtures/runner-local/`
- 动作：
  - 新增 `intent-native-erc20-transfer/` 目录
  - 包含 workspace、pack、runner config（含 llm）、runtime 示例
  - 提供 dry-run 与 execute 命令脚本
- AC：
  - 一条命令可复现 demo

Progress notes:

- 新增目录：`rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/`
  - `workspace/`：`evm-native-utils`、`erc20`、`safe-defi pack`
  - `config/runner.local.yaml`：本地链+demo signer 配置
  - `runtime/runtime.local.json`：wallet/target/token 示例输入
  - `intent/intent.txt`：自然语言意图样例
  - `plan/intent-native-erc20.plan.json`：目标计划基线
  - `llm/intent-native-erc20.success.jsonl`：scripted LLM `propose_plan` 响应
  - `README.md`：一键 YOLO 与手动确认两套命令
- 演示路径覆盖：
  - `intent -> tool-calling propose_plan -> execute`
  - `safe` 手动确认与 `yolo` 自动确认

Validation:

- `cargo run -p ais-runner -- run plan --plan fixtures/runner-local/intent-native-erc20-transfer/plan/intent-native-erc20.plan.json --dry-run --format json`

---

## 5) 文档任务

### [x] AISINT-DOC-001 Runner README Intent 使用指南 `P0`

- Scope：`rust/ais-rs/crates/ais-runner/README.md`
- 动作：
  - 增加 `--intent` 模式说明
  - 增加标准 provider + demo scripted 对比
  - 增加安全提示与风险建议
- AC：
  - 新用户按 README 可跑通最小 demo

Progress notes:

- `rust/ais-rs/crates/ais-runner/README.md` 新增 `Intent mode quick guide`：
  - intent 模式完整链路说明（planning/execution/confirm/repair）
  - `demo-scripted` 确定性示例命令（对接 `intent-native-erc20-transfer` fixture）
  - `standard` 实际 provider 示例命令（对接 `llm-providers` 配置模板）
  - `safe|assist|yolo` 风险与确认行为说明
  - 手动确认输入集（`approve|deny|always_approve_this_run|cancel`）与安全提示

Validation:

- README 中命令路径均使用现有 fixture：
  - `fixtures/runner-local/intent-native-erc20-transfer/...`
  - `fixtures/runner-local/llm-providers/config/...`

### [x] AISINT-DOC-002 演示剧本与故障排查 `P1`

- Scope：`rust/ais-rs/fixtures/runner-local/*/README.md`
- 动作：
  - 增加 intent 场景剧本（成功、confirm、失败修订）
  - 增加常见错误与排查步骤
- AC：
  - 演示链路可重复、可解释

Progress notes:

- `fixtures/runner-local/intent-native-erc20-transfer/README.md`
  - 新增三条演示剧本：
    - Scenario A：成功路径（yolo）
    - Scenario B：手动确认路径（safe）
    - Scenario C：失败后自动修订（`propose_plan` 失败 -> `revise_plan` -> `replace_plan`）
  - 新增故障排查清单（RPC、workspace、token 地址/余额、确认阻塞）
- 新增 `fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.repair.jsonl`
  - 用于失败修订闭环演示
- `fixtures/runner-local/llm-providers/README.md`
  - 增加 intent 示例与 provider 常见错误排查
- `fixtures/runner-local/native-erc20/README.md`
  - 增加常见错误排查与 intent fixture 互链

Validation:

- `jq -c . fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.repair.jsonl >/dev/null`
- README 中新增命令与路径均指向现有 fixture 文件。

---

## 6) 推荐实施顺序（可直接开工）

1. `AISINT-SPEC-001`  
2. `AISINT-RS-001`  
3. `AISINT-RS-002` + `AISINT-RS-003`  
4. `AISINT-RS-004`  
5. `AISINT-TEST-001~004`  
6. `AISINT-DOC-001~002`
