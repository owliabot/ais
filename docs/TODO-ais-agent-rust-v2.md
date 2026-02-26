# TODO: AIS 作为可组合 Agent Tool（Spec → Rust）改造清单（v2，无历史兼容）

日期：2026-02-14  
范围：AIS 规范（`specs/` + `schemas/`）、Rust 实现（`rust/ais-rs`）  
参考：`ref/ironclaw/docs/AGENT_ARCHITECTURE.md`、`docs/ais-agent-tool-v2.md`  
目标：先把 **SPEC 定死**，再在 `rust/ais-rs` 落地一个 **优雅、简洁、人类可读、模块化、解耦** 的 agent/runner 闭环（tool-calling、多轮、可审计、一定程度安全）。

> 本 TODO 只覆盖 “AIS x Agent（Rust）” 的闭环；不替代全仓库权威 TODO（如 `docs/TODO-rust-rewrite-ais-sdk-runner.md`），但写法保持一致：每个任务可追踪、可验收、可测试。

---

## 0) 追踪规范（必须遵守）

### 0.1 ID 规则

- `AISSPEC-###`：规范与 schema（`specs/`、`schemas/`、conformance vectors）
- `AISRS-AGENT-###`：Rust agent/runner 集成（`rust/ais-rs` 各 crates）
- `AISRS-PLUG-###`：Rust 执行插件/handler 体系（core vs plugin executors、allowlist、注册表）
- `AISRS-POL-###`：Rust policy/confirm/yolo（pack→policy gate→events→commands）
- `AISRS-CAT-###`：Rust catalog/cards/candidates（省 token 与检索）
- `AISRS-CMD-###`：Rust engine commands 协议与实现（含 plan mutation）
- `AISFIX-###`：fixtures / conformance / golden tests

### 0.2 状态字段

- `[ ]` 未开始
- `[~] 进行中
- `[x] 完成

### 0.3 必须遵守的顺序（硬门禁）

1) **先完成 `AISSPEC-*` 并达成评审一致**（含 JSON Schema 与 conformance fixtures）。  
2) **再开始任何 Rust 代码改造任务**（`AISRS-*`）。  

> Rust 任务在标题后标记 `Gate: AISSPEC-*`，表示未满足 gate 时不得开工。

### 0.4 Definition of Done（统一验收线）

- **Spec**：文字 + 权威 JSON Schema + conformance fixtures 对齐；关键语义可测试；版本策略明确。
- **Rust**：
  - 模块边界清晰（core / sdk / engine / runner / plugins / llm）
  - 单文件过长要拆（建议 >400 LOC 拆分；极端文件必须写出拆分理由）
  - 单函数过长要拆（>60 行必须拆分；>80 行禁止）
  - 多用 `trait + typed structs/enums` 表达协议边界，避免到处传 `serde_json::Value`
  - API 清晰：入口少、命名一致、错误类型可读（`thiserror`）
  - 测试完备：单测 + fixture/快照 + 集成测试；输出稳定（stable_json/stable_hash）
- **ironclaw 对齐点**（至少落地 3 个）：
  - 外层 agent loop 状态机（多轮、可暂停、可恢复）
  - 事件压缩/摘要（context compaction / memory pressure 思路）
  - 审批闸门（need_user_confirm + confirmation_hash + scope）

---

## 1) 里程碑（建议顺序）

### S0：Spec 定稿（阻塞 Rust）

- S0.1 执行插件/handler 语义（core vs plugin，注册与拒绝规则）
- S0.2 Engine commands 协议（含 plan mutation）
- S0.3 Pack approvals/yolo 与 policy gate I/O 规范化
- S0.4 Catalog cards 分层（index/detail）+ candidates 输出契约
- S0.5 Conformance/fixtures（覆盖上述关键语义）

### R1：Rust 最小可用 agent demo（基于 tool-calling、多轮、可暂停）

- R1.1 runner 增加 `agent` 子命令（外层 loop）
- R1.2 catalog/candidates 检索化输入（省 token）
- R1.3 policy gate 贯通（pack→plan→engine→need_user_confirm）
- R1.4 插件 executor 注册（先实现一个 offchain query 插件作为示例）
- R1.5 plan mutation（replace plan + diff + 再确认）

---

## 2) SPEC（先做这些，做完评审通过才能动 Rust）

### [x] AISSPEC-001 执行类型与 handler 注册语义定稿（core vs plugin）`P0`

- Scope：`specs/ais-1-capabilities.md`、`specs/ais-2-evm.md`、`specs/ais-2-solana.md`、（必要时新增）`specs/ais-2-execution-plugins.md`
- Problem：EVM/Solana 是默认支持；除此之外（offchain API、新链 BTC/Sui/Aptos…）必须通过注册的 handler 执行；spec 需要明确“未知 type 必须拒绝”的规则与 pack allowlist 关系。
- Spec 内容建议（最小规范）：
  - core execution types 列表（AIS-2 已定义者）
  - plugin execution types：`execution.type` 为 opaque string；只有当 **(a) handler 已注册** 且 **(b) pack allowlist 允许** 时可执行
  - engine 必须暴露“已注册 plugin execution types”用于规划期过滤（capabilities/discovery）
- AC：
  - 文档中明确：未注册 handler 的 plugin type 必须 fail-fast（规划期或执行前）
  - 文档中明确：pack `plugins.execution.enabled` 是 allowlist（链范围匹配）
  - 给出 2 个示例：`offchain_apy_query`、`sui_tx`（或 `aptos_tx`）
- Tests：
  - conformance：未注册 handler → 必须报错（稳定错误码/issue reference）

Progress notes:
- Updated spec text to make handler registration rules explicit:
  - `specs/ais-1-capabilities.md`
  - `specs/ais-1-pack.md`
  - `specs/ais-2-plan.md`
  - `specs/ais-2-evm.md`
- Added portable conformance kind + vectors for handler registration / plugin allowlist decisions:
  - `specs/ais-4-conformance.md`
  - `schemas/0.0.2/conformance.schema.json`
  - `specs/conformance/vectors/plugin-type-registration.json`

Handoff:
- Plugin execution examples (offchain APY / Sui/Aptos) are intentionally deferred until after `AISSPEC-002` lands, to keep the command protocol unblocked.
- Plan schema already permits unknown `execution.type` values (open union). The normative enforcement is: *unknown type is valid syntax but invalid execution unless handler is registered and (when pack active) allowlisted*.
- When we add the conformance fixture for “unregistered plugin type must be rejected”, we need to decide whether:
  - to extend `schemas/0.0.2/conformance.schema.json` to validate that fixture file shape, or
  - to treat it as an implementation fixture not covered by the conformance schema.

Completion note:
- Decision finalized: conformance fixture shape is now covered by `schemas/0.0.2/conformance.schema.json` (portable conformance kind).

### [x] AISSPEC-002 Engine Commands 规范化（JSONL envelope + 命令集合）`P0`

- Scope：新增 `specs/ais-2-engine-commands.md`；新增 `schemas/0.0.2/engine-command.schema.json`；更新 `specs/index.md`
- Problem：当前 Rust 已实现 commands（apply_patches/user_confirm/select_provider/cancel），但 spec 层缺失；agent 多轮闭环需要稳定命令协议。
- Spec 内容建议：
  - `EngineCommandEnvelope`：`schema + command { id, type, data }`
  - 命令：`apply_patches`、`user_confirm`、`select_provider`、`cancel`
  - 命令去重（idempotent）：`id` 的语义与重复处理
- AC：
  - schema 与示例齐全；版本策略明确（何时 bump）
  - 与 `specs/ais-2-engine-events.md` 的事件字段对齐（run_id/seq/ts 等）
- Tests：
  - conformance：命令 decode/encode、unknown type 拒绝、重复 id 行为说明

Progress notes:
- Added spec doc: `specs/ais-2-engine-commands.md`
- Added authority JSON Schema: `schemas/0.0.2/engine-command.schema.json`
- Updated spec index: `specs/index.md`
- Extended conformance schema with `json_schema_validate` and added vectors:
  - `schemas/0.0.2/conformance.schema.json`
  - `specs/conformance/vectors/engine-command.json`
- Tightened engine event schema for command ack events:
  - `specs/ais-2-engine-events.md`
  - `schemas/0.0.2/engine-event.schema.json`

Remaining for completion:
- None (done).

### [x] AISSPEC-003 Plan Mutation 的规范化最小集（replace plan / plan epoch）`P0`

- Scope：`specs/ais-2-plan.md` + `specs/ais-2-engine-commands.md` + schemas
- Problem：你要求 agent 能在执行中修改 plan；如果不规范化，checkpoint/replay/审计链会碎。
- Spec 内容建议（最小可落地）：
  - 引入 `plan_epoch`（或 `plan_revision`）概念（可放 plan.extensions）
  - 新命令 `replace_plan`（或 `set_plan`）：
    - 输入：新 plan（或 plan hash + plan body）
    - 约束：不得修改已执行节点的语义（默认策略）；高风险 diff 必须 need_user_confirm
  - 计划切换必须在 events 中记录（例如 `plan_replaced` 事件，或在 `engine_paused` data 中提供）
- AC：
  - 明确“允许/禁止”的变更集合（新增节点、调整未执行节点参数等）
  - 明确 plan diff 的最小输出字段（便于确认）
- Tests：
  - conformance：replace_plan 后 run_id 连续、审计可追踪；禁止的变更必须拒绝

Progress notes:
- Extended engine commands spec + schema with `replace_plan`:
  - `specs/ais-2-engine-commands.md`
  - `schemas/0.0.2/engine-command.schema.json`
- Documented plan mutation traceability guidance:
  - `specs/ais-2-plan.md`
- Extended engine events “optional events” set with `plan_replaced`:
  - `specs/ais-2-engine-events.md`
  - `schemas/0.0.2/engine-event.schema.json`
- Added schema validation vectors for replace_plan:
  - `specs/conformance/vectors/engine-command.json`

Remaining for completion:
- None (done).

Completion note:
- Decision finalized: `replace_plan` forbidden-mutation behavior tests live in implementation-level integration fixtures (stateful), while portable conformance keeps schema-contract validation only.
- Spec updates:
  - `specs/ais-2-engine-commands.md` (Conformance boundary + recommended fixture contract)
  - `specs/ais-4-conformance.md` (portable vs implementation test boundary)

### [x] AISSPEC-004 Pack approvals/yolo 模式与确认策略（按 risk_level）`P0`

- Scope：`specs/ais-1-pack.md` + `schemas/0.0.2/pack.schema.json`
- Problem：demo 要“一定程度安全”，同时你希望可配置 yolo；需要把“确认策略”规范化为可审计配置，而不是实现细节。
- Spec 内容建议：
  - pack.policy.approvals 增补（或明确）：
    - `mode: safe|assist|yolo`
    - `auto_execute_max_risk_level`
    - `require_approval_min_risk_level`
    - （可选）`llm_may_approve_max_risk_level`
  - 强制：所有自动批准必须产出可追踪证据（confirmation_hash + 决策来源）
- AC：
  - safe/assist/yolo 行为在 spec 中可解释且可测试
  - yolo 不是绕过 allowlist/handler 注册，只是降低确认门槛
- Tests：
  - conformance：risk_level 触发 need_user_confirm/hard_block 的决策矩阵

Progress notes:
- Added approval modes + optional LLM auto-approve threshold to pack spec + schema:
  - `specs/ais-1-pack.md`
  - `schemas/0.0.2/pack.schema.json`
- Added schema validation vectors:
  - `specs/conformance/vectors/pack-approvals.json`
- Added normative decision algorithm + portable conformance vectors:
  - `specs/ais-1-pack.md`
  - `schemas/0.0.2/conformance.schema.json`
  - `specs/conformance/vectors/pack-approvals-decision.json`
  - `specs/ais-4-conformance.md`

### [x] AISSPEC-005 Policy Gate I/O 进入 SPEC（从 docs 升级为规范）`P0`

- Scope：新增 `specs/ais-2-policy-gate.md`；对齐 `schemas/0.0.2/*`（如需要新增 schema）
- Problem：policy gate 若不规范化，need_user_confirm 无法稳定；LLM/CLI 无法一致展示与审计。
- Spec 内容建议（参考 `docs/ais-policy-gate-schema.md`）：
  - `PolicyGateInput`：missing vs unknown vs hard_block 字段语义；金额/滑点/授权等字段的表示规则
  - `PolicyGateOutput`：`ok|need_user_confirm|hard_block` + 结构化 details（证据）
  - `confirmation_summary` 与 `confirmation_hash` 的最小字段集与稳定哈希规则
- AC：
  - `need_user_confirm.details` 至少包含：`confirmation_summary`、`confirmation_hash`、`hit_reasons`（或等价）
  - 明确字段来源（field_sources）建议项（便于解释）
- Tests：
  - conformance：hash 稳定（忽略 ts 等）；缺字段时 missing/unknown 分流一致

Progress notes:
- Added policy gate spec doc and indexed it:
  - `specs/ais-2-policy-gate.md`
  - `specs/index.md`
- Tightened `need_user_confirm` event schema to require confirmation fields:
  - `schemas/0.0.2/engine-event.schema.json`
- Added authority schemas for policy gate types:
  - `schemas/0.0.2/policy-gate-input.schema.json`
  - `schemas/0.0.2/policy-gate-output.schema.json`
  - `schemas/0.0.2/confirmation-summary.schema.json`
- Added confirmation hash portable conformance kind + vectors:
  - `schemas/0.0.2/conformance.schema.json`
  - `specs/conformance/vectors/confirmation-hash.json`
  - `specs/ais-4-conformance.md`
- Added missing/unknown/hard_block portable conformance kind + vectors:
  - `specs/ais-2-policy-gate.md`
  - `schemas/0.0.2/conformance.schema.json`
  - `specs/conformance/vectors/policy-gate-missingness.json`
  - `specs/ais-4-conformance.md`

Remaining for completion:
- None (done).

### [x] AISSPEC-006 Catalog cards 分层（index/detail）+ candidates 输出契约 `P1`

- Scope：`specs/ais-1-catalog.md`、新增 `specs/ais-1-executable-candidates.md`（或扩展 catalog spec）、新增/更新 schemas
- Problem：省 token 的关键是“先 index cards 决策，再 detail cards 填参”；以及把 pack+capabilities 过滤后的候选集作为 agent 的最小上下文输入。
- Spec 内容建议：
  - Index Card 最小字段（可检索、扁平）：ref/protocol/version/id/description?/risk/execution_types/execution_chains/capabilities_required
  - Detail Card：params/returns/requires_queries/hard_constraints（可选）
  - `ExecutableCandidates` 文件契约（hash、created_at、pack identity、chain_scope、actions/queries/providers/plugins）
- AC：
  - 候选集可 stable_hash；支持缓存与 diff
  - 允许 engines 按 pack+capabilities+chain_scope 产生一致候选
- Tests：
  - fixtures：同一输入多次生成 candidates hash 一致；忽略 created_at

Progress notes:
- Clarified index vs detail card levels and added authority schema reference:
  - `specs/ais-1-catalog.md`
- Added executable candidates spec:
  - `specs/ais-1-executable-candidates.md`
  - `specs/index.md`
- Added authority JSON Schemas:
  - `schemas/0.0.2/catalog.schema.json`
  - `schemas/0.0.2/executable-candidates.schema.json`
- Added schema validation conformance vectors:
  - `specs/conformance/vectors/catalog.json`

### [x] AISSPEC-007 动态协议导入/安装/生成的分级治理（safe/assist/yolo）`P1`

- Scope：`specs/ais-3-discovery.md`、`specs/ais-3-registry.md`、`specs/ais-1-pack.md`
- Problem：你希望动态安装/生成可配置；需要明确“哪些模式允许、需要什么证据（integrity/来源/签名）”。
- AC：
  - safe：禁止动态生成；仅允许 registry/本地
  - assist：允许安装但必须 need_user_confirm + 记录 integrity
  - yolo：允许任意来源但仍记录来源与 hash；不得绕过 allowlist
- Tests：
  - conformance：pack 配置不同，允许/拒绝行为可预测

Recommended additions (to make this spec *agent-suitable*, auditable, and robust):

- Define a normative `ProtocolSource` model (discovery/installation input):
  - `local_path`（workspace 内文件）
  - `registry_ref`（`registry + package + version + digest` 的不可变引用）
  - `remote_url`（明确 domain allowlist + https-only；必须携带 digest）
  - `llm_generated`（必须携带生成上下文摘要与 `generator_id`，并强制落地为“临时本地文件”再参与后续流程）
- Define a `ProtocolInstallRecord` (installation output, MUST be persisted into trace/events):
  - `source`（以上 `ProtocolSource`）
  - `integrity`（至少 `sha256`，建议 `content_digest` + `schema_digest`）
  - `resolved_identity`（最终 protocol id/version）
  - `timestamp`（可选；不可参与哈希）
  - `policy_decision`（`ok|need_user_confirm|hard_block` + reasons）
- Make “installation” an explicit policy-gated step:
  - 动态安装/生成不应被 engine 隐式网络访问完成；应经由 runner/agent 的 policy gate 决策后，把“安装结果（本地 protocol 文档）”作为后续规划/执行输入。
  - `assist/yolo` 仍不得绕过 `AISSPEC-001` 的 handler 注册与 pack allowlist（安装≠可执行）。
- Add pack knobs that are *actionable* and testable:
  - `pack.policy.protocol_install.mode: safe|assist|yolo`
  - `allowed_sources`（比如只允许 `registry_ref` + `local_path`）
  - `registry_allowlist`（publisher/package 前缀/签名者）
  - `domain_allowlist`（remote url 场景）
  - `require_signature: true|false` + `trusted_publishers`
  - `llm_generated.require_user_confirm: true`（建议默认 true，即使 yolo 也要保留可追踪摘要与 hash）
- Security defaults recommendation:
  - 默认 `safe`；`assist` 仅允许 registry 安装（带 digest + signature）并强制 need_user_confirm；`yolo` 允许 remote/llm_generated，但必须落盘、记录来源、展示 diff/摘要，并提供一键回滚（删除该临时协议）。

Progress notes:
- Added normative protocol install/discovery model:
  - `specs/ais-3-discovery.md` defines `ProtocolSource` kinds (`local_path|registry_ref|remote_url|llm_generated`) and per-kind required fields/rules.
  - `specs/ais-3-registry.md` defines `ProtocolInstallRecord` audit contract (source, integrity, resolved identity, policy decision, installed path).
- Extended pack policy with install governance knobs:
  - `specs/ais-1-pack.md` now specifies `policy.protocol_install.*` semantics (`mode`, source allowlist, registry/domain allowlists, signature and publisher requirements).
  - `schemas/0.0.2/pack.schema.json` adds schema coverage for `policy.protocol_install`.
- Added portable conformance support:
  - `specs/ais-4-conformance.md` adds kind `protocol_install_decision`.
  - `schemas/0.0.2/conformance.schema.json` adds validation shape for this kind.
  - Added vectors: `specs/conformance/vectors/protocol-install-decision.json`.

Remaining for completion:
- None (done).

---

## 3) Rust（SPEC 定稿后再做；所有任务必须标注 Gate）

### [x] AISRS-AGENT-001 `ais-runner agent` 外层 agent loop（tool-calling，多轮）`P0`  (Gate: AISSPEC-002)

- Crates：`rust/ais-rs/crates/ais-runner`
- Deliverables：
  - 新 CLI 子命令：`ais-runner agent ...`
  - 外层 loop：订阅 engine events → 压缩摘要 → 调用 LLM tools → 发送 engine commands → 继续
  - CLI 阻塞确认：遇到 need_user_confirm 按 pack/runner mode 决定（human/llm/yolo）
- API/设计要求（ironclaw 对齐）：
  - 显式状态机：Idle/Running/AwaitingConfirm/AwaitingPatch/Failed/Completed
  - 事件 compaction：把长事件流压成“小摘要 + 必要引用”，避免 token 洪水
  - 可恢复：支持从 checkpoint 继续（复用现有 `replay_from_checkpoint`）
- Tests：
  - 集成测试：模拟一条 plan 触发 need_user_confirm → 发送 user_confirm → 完成

Progress notes:
- Implemented `ais-runner agent` with a typed outer loop + pause summary + interactive brain:
  - CLI: `rust/ais-rs/crates/ais-runner/src/cli.rs` (`agent`, `ApprovalsMode`)
  - Entry: `rust/ais-rs/crates/ais-runner/src/agent/mod.rs` (`execute_agent`)
  - Loop: `rust/ais-rs/crates/ais-runner/src/agent/loop.rs` (`run_agent_loop`)
  - Brain: `rust/ais-rs/crates/ais-runner/src/agent/brain.rs` (`Brain` trait + `CliBrain`)
  - Pause summary: `rust/ais-rs/crates/ais-runner/src/agent/summary.rs`
- Added unit/integration test for multi-round pause→confirm→continue:
  - `rust/ais-rs/crates/ais-runner/src/agent/mod_test.rs`
- Updated crate README with the new command:
  - `rust/ais-rs/crates/ais-runner/README.md`

### [x] AISRS-CAT-001 Catalog cards 扩展到 spec 约定字段 + stable hash `P0` (Gate: AISSPEC-006)

- Crates：`ais-sdk`
- Deliverables：
  - `build_catalog` 输出补齐 risk/description/params/returns（按 spec）
  - index/detail card 分离（避免默认输出巨卡）
  - `get_executable_candidates` 作为 agent 输入（pack+capabilities+chain_scope 过滤）
- 设计要求：
  - 类型化 card structs（尽量避免 `Value`）
  - stable_sort/stable_hash 覆盖 created_at 等非决定字段
- Tests：
  - fixture：catalog hash 稳定；candidates hash 稳定

Progress notes:
- `ais-sdk` catalog card model upgraded with typed structs:
  - `CatalogCardLevel` (`index|detail`)
  - `ActionCard` / `QueryCard` / `CatalogParam` / `CatalogReturn`
- `build_catalog` now emits spec-aligned fields:
  - actions: `description`, required `risk_level` (default=3 when missing), `risk_tags`,
    `execution_types`, `execution_chains`, `capabilities_required`
  - detail-level adds `params` / `returns` / `requires_queries`
  - packs now include policy summary fields (`policy`, `token_policy`, `providers`, `plugins`, `overrides`)
- Determinism:
  - keeps stable sort and stable hash behavior, still ignores `created_at`/`hash`
  - detail payloads normalize list order for deterministic output
- Candidates path:
  - `get_executable_candidates` continues as agent-input contract with pack/capabilities/chain_scope filtering and stable hash.
  - catalog/filter tests updated to include spec-required action fields (`risk_level`, `level`).

Remaining for completion:
- None (done).

### [x] AISRS-POL-001 pack→engine policy 映射（运行时硬 enforce）`P0` (Gate: AISSPEC-004, AISSPEC-005)

- Crates：`ais-runner` + `ais-engine` + `ais-sdk`
- Deliverables：
  - runner 解析 active pack（workflow.requires_pack 或 CLI 显式指定）
  - pack policy → `PolicyEnforcementOptions`（allowlist + thresholds + approvals mode）
  - need_user_confirm 输出包含 confirmation_summary/hash（稳定）
- 设计要求：
  - policy gate 输入必须携带 action_ref/risk 信息（见 AISRS-POL-002）
  - yolo/assist/safe 行为可配置且可审计
- Tests：
  - 单测：risk_level 阈值矩阵
  - 集成：pack 禁止某 action_ref → 必须 hard_block

Progress notes:
- Started CLI-based pack wiring for `ais-runner agent --pack <file>` (demo-first, workflow.requires_pack later).
- `ais-runner agent` now loads pack and maps into `ais-engine` `PolicyEnforcementOptions`:
  - `includes[*].chain_scope` → chain allowlist
  - `plugins.execution.enabled` → plugin execution type allowlist (unlisted plugin types hard-block by default when pack is active)
  - `policy.hard_constraints_defaults.{max_spend,max_approval,max_slippage_bps,allow_unlimited_approval}` → thresholds/hard blocks
  - `policy.approvals.auto_execute_max_risk_level` → `thresholds.max_risk_level` (no approvals config => conservative confirm-by-default)
  - validates `auto_execute_max_risk_level < require_approval_min_risk_level` when both set
- `ais-engine` now emits schema-compatible `need_user_confirm` details (required fields + `confirmation_summary`/`confirmation_hash`) for both:
  - solver-driven pauses (missing runtime refs / detect selection / readiness errors)
  - policy-gate-driven pauses (missing/unknown/threshold/allowlist)
- `assist` mode LLM auto-approval threshold is now wired:
  - pack field `policy.approvals.llm_may_approve_max_risk_level` is parsed by runner
  - `agent --approvals-mode assist` + `--llm-script-jsonl <file>` enables scripted LLM tool-calling
  - only `need_user_confirm` with `confirmation_summary.risk_level <= threshold` is eligible for auto-approve; others remain manual
  - LLM/tool failure falls back to manual confirm (auditable stderr notice)

Remaining for completion:
- None (done).

### [x] AISRS-POL-002 plan node 贯通 action_ref / risk_level / risk_tags（拒绝启发式猜测）`P0` (Gate: AISSPEC-005)

- Crates：`ais-sdk`（planner）+ `ais-engine`（policy gate input extract）
- Deliverables：
  - planner 在 plan node.source 或 node.extensions 写入规范字段（action_ref/query_ref、risk）
  - policy gate 从该字段读取（不再依赖 method contains("swap") 等启发式）
- Tests：
  - fixture：同一 workflow 编译的 plan 节点 risk 字段稳定

Progress notes:
- `ais-sdk` compile_workflow copies action `risk_level`/`risk_tags` into plan node `extensions`, and emits `extensions.policy.{param_roles,required_fields}` derived from action schema (no swap/approve string heuristics).
- `ais-engine` policy gate input extraction reads `extensions.policy` to determine missing/unknown fields; removes `method contains(\"swap\"|\"approve\")` heuristics.
- `ais-runner` agent loop tests now model confirm pauses via `node.extensions.policy.required_fields` instead of execution method-name heuristics.

Remaining for completion:
- None (done).

### [x] AISRS-PLUG-001 插件 executor 注册表（core vs plugin）`P0` (Gate: AISSPEC-001)

- Crates：`ais-engine`（router/executor）+ `ais-runner`（装配）
- Deliverables：
  - 明确的 `ExecutorRegistry`/`HandlerRegistry`：
    - core：evm/solana 默认注册
    - plugin：按配置/feature 注册额外 handler
  - 执行时：未注册的 execution.type → 明确错误（规划期或执行前）
  - pack allowlist：plugin execution types 必须在 pack.plugins.execution.enabled 中
- 设计要求：
  - 用 trait 表达 handler 边界：`ExecutionHandler::execute(&ExecutableNode, &mut Runtime) -> ExecutorOutput`
  - handler 自己负责其 config 校验（例如 endpoint allowlist）
- Tests：
  - 未注册 handler → 错误事件稳定

Progress notes:
- `ais-engine` router registration now carries explicit `execution_types` and `kind` (`core|plugin`), and routes by `chain + execution.type` instead of chain-only.
- Runtime routing now distinguishes:
  - `chain` not configured → `ChainMismatch`
  - `execution.type` not registered on configured chain → `UnregisteredExecutionType`
- `ais-runner` router assembly now registers:
  - EVM core (`evm_read`, `evm_call`)
  - EVM plugin (`evm_rpc`)
  - Solana core (`solana_read`, `solana_instruction`)
- `build_router_executor_for_plan` now fail-fast validates each node `execution.type` against registered handlers (`runner.config.execution_type_unregistered`).
- policy gate core-type判定收敛为显式集合（`evm_read|evm_call|solana_read|solana_instruction`），避免 `evm_rpc` 等非 core 类型绕过 plugin allowlist。

Remaining for completion:
- None (done).

### [x] AISRS-PLUG-002 Offchain APY query 插件（作为插件体系样例）`P1` (Gate: AISSPEC-001)

- Crates：建议新增 `crates/ais-offchain-executor`（或放在 `ais-engine` 的 plugins 子模块，但建议独立 crate）
- Deliverables：
  - 一个示例 execution.type（例如 `offchain_apy_query`）
  - 支持 pack/runner 的域名 allowlist、超时、重试
  - 输出写入 `nodes.<id>.outputs`
- Tests：
  - 纯单测：response decode 映射 outputs
  - 集成测：用本地 stub server（或 fixture mock）验证 handler 行为

Progress notes:
- Added new crate `rust/ais-rs/crates/ais-offchain-executor` with plugin handler `offchain_apy_query`.
- Executor behavior:
  - enforces endpoint domain allowlist (exact + wildcard rules),
  - supports GET/POST JSON with timeout and bounded retry,
  - normalizes response into `result.outputs` for query output projection.
- Runner integration:
  - `RunnerConfig.plugins.execution.offchain_apy_query` added (enabled/chains/allowed_domains/timeout/retry),
  - router registers offchain plugin handler per configured chain,
  - config validation enforces non-empty `chains`/`allowed_domains` and positive timeout/backoff when enabled.
- Tests:
  - plugin unit tests (allowlist reject, wildcard allow, retry success),
  - runner config tests for plugin registration + invalid config rejection.

Remaining for completion:
- None (done).

### [x] AISRS-CMD-001 Engine command：`replace_plan`（plan epoch + diff + 再确认）`P0` (Gate: AISSPEC-003)

- Crates：`ais-engine` + `ais-runner`
- Deliverables：
  - 新命令类型 + schema decode/encode
  - runner 对新 plan 做：pack/capability 校验 + diff 输出 +（必要时）need_user_confirm
  - plan epoch 记录在 checkpoint/trace 中可追踪
- Tests：
  - fixture：replace_plan 流程可 replay；禁止变更被拒绝（稳定原因）

Progress notes:
- Engine command/runtime:
  - `ais-engine` command type新增 `replace_plan`，event type新增 `plan_replaced`。
  - runner在每轮执行前预处理 `replace_plan` 命令：解析并校验 plan、做 diff、应用命令去重。
- Safety/confirmation:
  - 若改动触及已完成节点（删除或语义变更）则拒绝，产出稳定原因码：
    - `replace_plan_forbidden_completed_node_removed`
    - `replace_plan_forbidden_completed_node_mutated`
  - 高风险 diff（`removed>0` 或 `changed>0`）在未 `confirmed=true` 时触发 `need_user_confirm` 并暂停。
- Traceability/checkpoint:
  - checkpoint engine_state 增加 `plan_epoch`、`plan_hash_history`。
  - checkpoint 文档增加 `plan_snapshot`，用于 plan hash 不一致时的恢复。
  - runner在 plan 替换成功时递增 `plan_epoch` 并记录 `plan_hash_history`，同时发出 `plan_replaced` 事件（含 before/after hash、diff、command_id）。
- Tests:
  - `ais-runner` 新增 `replace_plan` 成功替换与“已完成节点变更拒绝”单测。
  - command JSONL 解析测试覆盖 `replace_plan`。
  - `ais-engine` checkpoint/replay 相关测试同步覆盖新字段与新签名。

Remaining for completion:
- None (done).

### [x] AISRS-AGENT-002 LLM Provider 抽象与 tool-calling 适配（最小实现）`P0` (Gate: AISSPEC-002)

- Crates：建议新增 `crates/ais-llm`（避免把 LLM 逻辑塞进 runner）
- Deliverables：
  - `LlmProvider` trait：`complete_with_tools(messages, tools) -> tool_calls`
  - tool schema 与调用结果的 typed model（避免拼 JSON 字符串）
  - runner 的 agent loop 只依赖 trait，不绑定某家 API
- Tests：
  - mock provider：给定输入返回固定 tool_calls（用于集成测）

Progress notes:
- Added new crate `rust/ais-rs/crates/ais-llm`:
  - typed tool-calling model: `LlmMessage`, `ToolSpec`, `ToolCall`, `CompleteWithToolsRequest/Response`
  - provider trait: `LlmProvider::complete_with_tools(...)`
  - deterministic test adapter: `ScriptedLlmProvider`
- Runner agent integration:
  - `LlmBrain<P: LlmProvider>` added in `rust/ais-rs/crates/ais-runner/src/agent/brain.rs`
  - `LlmBrain` converts typed tool calls into engine commands (`confirm`, `cancel`, `send_engine_command`)
  - pause context is serialized into typed message payload for provider planning
- Tests:
  - `ais-llm` unit test for scripted provider sequencing
  - `ais-runner` agent tests:
    - LLM auto-approve flow completes run
    - unknown tool call is rejected with deterministic error

Remaining for completion:
- None (done).

---

## 4) 附录：PR/评审建议

- 每个 PR 标题带 ID（例如 `[AISSPEC-002] Add engine commands spec + schema`）。
- Spec PR 必须同时更新：
  - `specs/*.md`
  - `schemas/0.0.2/*.schema.json`
  - 至少 1 个 conformance fixture（或示例文档）
- Rust PR 必须同时更新：
  - 对应 crate 的 `README.md`（若改动在 `rust/ais-rs/crates/<crate>/` 下）
  - 单测/集成测/fixtures 至少一个
