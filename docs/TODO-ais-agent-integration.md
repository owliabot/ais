# AIS x Agent 集成改造 TODO（不考虑历史兼容）

日期：2026-02-12  
范围：AIS 规范（specs/schemas/examples）、TS SDK（ts-sdk）、Runner（tools/ais-runner）  
目标：让 AIS 成为 Agent 工具链中的“可靠执行组件”，以 **plan-first + 事件驱动 + 增量补丁** 的交互模式运行（详见 `docs/ais-agent-best-interaction.md`）。

本文件是一个“聚焦 agent 互操作”的 TODO 清单，不替代全仓库权威 TODO（`docs/TODO.md`），但条目写法保持同样的可追踪格式，便于后续挑选合并上游。

---

## 0) 追踪方式（必须遵守）

- 每个条目有唯一 ID：`AGD#`（决策）/ `AGT###`（任务）。
- 进度用 Markdown checkbox：`[ ]` 未完成、`[x]` 已完成。
- 依赖用 `Deps:` 标注前置条目 ID。
- 验收标准用 `AC:`（Acceptance Criteria）定义“完成的定义”（可自动/手动验证）。
- 优先级标签：
  - `P0`：阻塞（不做无法形成 agent->runner 闭环或安全边界无法落地）
  - `P1`：重要增强（显著降低 token/错误率、提升可观测与可审计）
  - `P2`：可选优化（体验/性能/生态）

---

## 1) 决策（先定边界再做实现）

### AGD1（已采用 A）Runner 主输入：ExecutionPlan vs Workflow

- [x] AGD1 = A
- A（推荐）ExecutionPlan 为主输入：agent/工具链主要产出 plan（或 plan patch）；workflow 作为可选导出物（模板/分享/审计/发布）。
- B Workflow 为主输入：runner 接受 workflow 并在运行时规划为 plan；对人类友好但 agent 侧容易陷入“生成整份 YAML”的高 token/高错误面。
- AC:
  - Runner 支持 `run plan`（plan JSON/YAML）端到端（dry-run + 执行模式）。
  - Workflow 模式仍保留但从“主路径”降级为“兼容/导出路径”（不影响现有 demo）。

### AGD2（已采用 A）交互协议：事件流 + 命令（JSONL） vs 纯 CLI 文本

- [x] AGD2 = A
- A（推荐）JSONL 双向协议：runner 输出 events（JSONL），并从 stdin 接收 commands（JSONL）。
- B 仅 CLI 文本输出：适合人看，不适合 agent-loop 与自动化集成。
- AC:
  - 有一份稳定的事件/命令 schema（可校验、可版本化）。
  - runner 能在 `engine_paused` 状态下接收 patch/confirm 并继续推进。

### AGD3（已采用 A）策略 gate 语义：hard block vs soft confirm

- [x] AGD3 = A
- A（推荐）默认“软确认优先，硬约束必须阻断”：风险阈值/非 allowlist 等触发 `need_user_confirm`，硬约束（如禁无限授权、滑点上限）直接阻断或要求显式 override。
- B 一律 hard block：更安全但用户体验差，且对长尾意图无法迭代。
- AC:
  - 每条策略产出结构化原因、阈值、命中证据（可用于 UI 与审计）。
  - 同一策略在 dry-run 与执行模式下行为一致（仅差是否实际做链 IO）。

---

## 2) P0：必须完成（形成 agent->runner 安全闭环）

### AGT001 规范化 ExecutionPlan（使其成为互操作契约）`P0`

- [x] AGT001
- Deps: 无
- Scope: Specs + Schemas + Examples + TS SDK
- Problem: plan 已经是执行契约，但如果不进入规范层，就无法跨实现互操作，也难以约束生态工具链。
- AC:
  - 新增 plan 规范文档（建议单独一篇）并加入 spec index。
  - 发布 `ais-plan/<ver>` 的权威 JSON Schema（含 nodes/deps/writes/source/until/retry/assert/timeout/bindings）。
  - 明确 plan 的稳定序列化规则（字段排序建议、BigInt 表示、extensions 扩展槽）。

### AGT001A Plan 最小字段集与版本策略定稿 `P0`

- [x] AGT001A
- Deps: AGT001
- Scope: Specs + Schemas
- AC:
  - 定义 plan 的版本号策略：何时 bump `schema`，何时仅 bump `meta` 或 `extensions`。
  - 明确 plan node 的 `kind` 枚举与语义（至少区分 read/write/execution 或等价分类）。
  - 明确 `writes` 的语义：`set` vs `merge`、允许写入的默认路径集合、冲突处理（多节点写同一路径时必须定义行为）。

### AGT001B Plan 与 Workflow 的关系（导出/反导入边界）`P0`

- [x] AGT001B
- Deps: AGD1, AGT001
- Scope: Specs + Docs
- AC:
  - 明确 workflow 是“内容资产/模板”，plan 是“执行契约”；两者可互相导出但不保证无损往返（写清限制项）。
  - 明确 runner 在 `run workflow` 时生成 plan 的 determinism 要求（相同输入得到同 plan，除 `created_at` 等非决定字段）。

### AGT002 规范化 Engine Events（事件协议）`P0`

- [x] AGT002
- Deps: AGT001
- Scope: Specs + Schemas + TS SDK + Runner
- Problem: agent-loop 需要稳定事件协议，否则 UI/agent 集成会碎片化，无法回放/审计。
- AC:
  - 定义最小事件集合：`plan_ready`、`node_ready`、`node_blocked`、`need_user_confirm`、`query_result`、`tx_prepared`、`tx_sent`、`tx_confirmed`、`node_waiting`、`checkpoint_saved`、`engine_paused`、`error`。
  - 规定 `need_user_confirm.reason/details` 的结构化字段（至少包含 node_id、action_ref、pack/policy 摘要、命中原因列表）。
  - runner 支持 `--events-jsonl <path|stdout>` 输出原始事件 JSONL；默认文本输出保持不变。

### AGT002A 事件 schema 与字段最小集合定稿（含兼容扩展槽）`P0`

- [x] AGT002A
- Deps: AGT002
- Scope: Specs + Schemas
- AC:
  - 每个事件统一包含：`schema`（或 `type` + version）、`ts`、`run_id`、`seq`、`node_id?`、`data`、`extensions?`。
  - `error` 事件定义 `retryable` 的判断字段与原因（避免 runner/agent 各自猜测）。
  - 明确哪些字段允许出现在 `extensions`，哪些必须进入规范字段（避免语义漂移）。

### AGT002B 事件 redact 策略定稿（安全默认）`P0`

- [x] AGT002B
- Deps: AGT002
- Scope: Specs + TS SDK + Runner
- AC:
  - 定义默认 redact：私钥/seed、完整 RPC payload、原始签名材料、用户 PII。
  - 定义“审计级 trace”可选白名单字段（由 config 显式开启）。
  - runner 支持 `--trace-redact <mode>`（或等价配置），并在 events 中标注当前 mode。

### AGT003 规范化 Runner Commands（patch/confirm/provider selection）`P0`

- [x] AGT003
- Deps: AGT002, AGD2
- Scope: Specs + Schemas + Runner + TS SDK
- Problem: 没有命令协议，runner 无法在暂停后被 agent 驱动继续执行，只能人工重启/重跑。
- AC:
  - 定义命令集合：`apply_patches`、`user_confirm`、`select_provider`、`cancel`。
  - `apply_patches` 有 schema 且限制可写命名空间（至少 `inputs/ctx/contracts/policy`；禁止任意写 `nodes.*` 除非白名单）。
  - runner 在 `engine_paused` 状态可消费命令并继续推进到下一个事件边界。

### AGT003A Commands schema 定稿 + 关联事件（command_accepted/command_rejected）`P0`

- [x] AGT003A
- Deps: AGT003
- Scope: Specs + Schemas + Runner
- AC:
  - 命令必须有 `id`（去重/幂等）、`ts`、`kind`、`payload`、`extensions?`。
  - runner 对每条命令输出 `command_accepted` 或 `command_rejected` 事件（含原因与字段路径）。
  - runner 支持 `--commands-stdin-jsonl` 开关：开启后才读取 stdin（避免误读管道）。

### AGT003B 幂等与去重策略（避免重复 patch/confirm）`P0`

- [x] AGT003B
- Deps: AGT003A
- Scope: Runner + TS SDK
- AC:
  - 定义命令幂等：同 `command.id` 重放必须无副作用或明确拒绝。
  - checkpoint 中记录已处理 command ids（或等价机制），支持 resume 后继续读取 commands。

### AGT004 Pack allowlist 执行期强制（detect/providers/plugins）`P0`

- [x] AGT004
- Deps: AGD1
- Scope: Specs + TS SDK + Runner
- Problem: workspace 校验不足以覆盖运行时选择（detect provider、plugin execution type、链维度），必须在执行期强制。
- AC:
  - detect resolution 必须校验 `(kind, provider, chain)` 是否被 pack 允许；不满足时产生结构化 `need_user_confirm` 或 `error`（按 AGD3）。
  - plugin execution types 必须校验 `(type, chain)` 是否被 pack 允许；不满足时同上。
  - 提供最少 6 个 fixtures 覆盖：允许/禁止 detect provider、允许/禁止 plugin type、链维度限制。

### AGT004A Detect provider allowlist 规则与优先级定稿 `P0`

- [x] AGT004A
- Deps: AGT004
- Scope: Specs + TS SDK
- AC:
  - 当 detect 未指定 provider 时的选择规则：必须在 allowlist 内选；多候选时按 priority/chain 匹配（规则写死）。
  - 当 detect 指定 provider 时：不在 allowlist 内必须阻断或确认（按 AGD3）。
  - 失败时的结构化细节：候选列表、pack 启用列表、链信息。

### AGT004B Plugin execution allowlist 规则与链维度定稿 `P0`

- [x] AGT004B
- Deps: AGT004
- Scope: Specs + TS SDK
- AC:
  - 明确 “type-only allowlist” vs “(type, chain) allowlist” 的最终选择，并统一到 spec 与实现。
  - 失败时结构化细节：node execution.type、chain、允许列表、pack 元信息。

### AGT005 Pack policy gate 升级为“可执行风控阀”（不仅 risk-level）`P0`

- [x] AGT005
- Deps: AGD3, AGT002
- Scope: Specs + TS SDK + Runner
- Problem: 仅基于 risk_level 的审批不足以覆盖真实风险；需要把 token/slippage/approval/spend 等纳入可执行 gate，并能解释给用户。
- AC:
  - 定义标准化 “Policy Gate Input” 结构（至少含 chain、action_ref、risk、assets、slippage、approvals、spend）。
  - 定义 gate 输出结构：`hard_block` vs `need_user_confirm`，附原因、阈值与证据。
  - runner 在写节点执行前产出 gate input（dry-run 也产出），并据 gate 输出阻断或请求确认。
  - 覆盖最少 10 条测试：超滑点、无限授权、非 allowlist token、风险等级、组合命中多条规则。

### AGT005A PolicyGateInput/Output 的 schema 与字段字典定稿 `P0`

- [x] AGT005A
- Deps: AGT005
- Scope: Specs + Schemas
- AC:
  - 定义字段字典：每个字段的来源、允许为空的语义（unknown vs missing）、以及审计用途。
  - 明确 risk 信息来源：action risk_level/risk_tags + pack overrides + 运行时附加标签（若允许）。
  - 明确 token/slippage/approval/spend 的表示形式（字符串/BigInt/资产对象）与精确度要求。

### AGT005B Gate 结果与用户确认 UX 的映射规则 `P0`

- [x] AGT005B
- Deps: AGT005A, AGD3
- Scope: Docs + Runner
- AC:
  - 定义默认文案模板字段（action、链、风险等级、命中规则、阈值、建议操作）。
  - 明确“用户确认”粒度：按 workflow node、按 actionKey、还是按交易 hash（避免过度确认或漏确认）。

### AGT006 Runner 支持 `run plan` 端到端（dry-run + execute）`P0`

- [x] AGT006
- Deps: AGD1, AGT001
- Scope: Runner
- Problem: plan-first 无法落地时，agent 仍会被迫生成 workflow 文本，回到高 token/高错误面。
- AC:
  - `run plan --file <plan>`：可执行 dry-run（输出 readiness/缺失 refs/需要 detect/策略 gate 结果）。
  - `run plan --file <plan> --config <cfg> --broadcast`：执行模式可跑通参考 EVM/Solana executor。
  - plan schema 校验失败时给出可机器消费的错误列表（路径/原因）。

### AGT007 TS SDK 输出/输入统一 JSON 编解码（plan/events/patch/checkpoint）`P0`

- [x] AGT007
- Deps: AGT001, AGT002, AGT003
- Scope: TS SDK + Runner
- Problem: agent 互操作依赖稳定的 JSON 表示；BigInt/bytes/Error 等若不统一，会导致跨进程/跨语言解析失败或语义漂移。
- AC:
  - 定义并导出统一的 JSON codec（至少覆盖 BigInt、Uint8Array、Error、undefined 的拒绝策略）。
  - runner 的 `--events-jsonl` 与 commands 输入使用同一 codec（保证可 roundtrip）。
  - 10+ 单测：plan/event/patch/checkpoint 的 encode->decode 等价性与负例（非法值拒绝）。

### AGT007A JSON 表示 Profile 定稿（BigInt/bytes/Error）`P0`

- [x] AGT007A
- Deps: AGT007
- Scope: Specs + TS SDK
- AC:
  - BigInt 表示必须唯一（例如 `{ "$bigint": "123" }` 或 decimal string）；禁止多种表示并存。
  - Uint8Array/bytes 必须唯一表示（base64 或 hex 二选一），并明确大小写/前缀规则。
  - Error 表示明确字段（name/message/stack?）与隐私默认（stack 是否默认剥离）。

### AGT007B Runner 与 SDK 的 codec 统一接入点收敛 `P0`

- [x] AGT007B
- Deps: AGT007A
- Scope: TS SDK + Runner
- AC:
  - runner 所有 JSONL 输出（events/trace/checkpoint）都走同一个 codec 入口。
  - runner 所有 JSONL 输入（commands/inputs/ctx）都走同一个 codec 入口（失败给结构化错误）。

### AGT008 TS SDK 规范化 RuntimePatch（schema + 命名空间防护 + 可审计）`P0`

- [x] AGT008
- Deps: AGT003
- Scope: TS SDK + Runner
- Problem: patch 是 agent 改变系统状态的唯一入口；必须做到可校验、可限制、可审计，否则等同于越权写任意状态。
- AC:
  - 定义 `RuntimePatch` 的权威 schema（含 `op/set|merge`、`path`、`value`、`extensions`）。
  - 提供“可写命名空间”策略（默认仅允许 `inputs/ctx/contracts/policy`；可配置扩展白名单）。
  - `applyRuntimePatches` 在启用 guard 时拒绝越权 path，并返回结构化错误（含 path 与原因）。
  - runner 在 agent-mode 下强制启用 guard，并把拒绝信息以事件输出。

### AGT008A Patch Guard 的可写路径策略定稿（白名单/正则）`P0`

- [x] AGT008A
- Deps: AGT008
- Scope: TS SDK + Runner + Docs
- AC:
  - 给出默认 allowlist：`inputs.*`、`ctx.*`、`contracts.*`、`policy.*`。
  - 明确是否允许写 `nodes.*`（默认禁止；如允许，必须限定子路径且给出理由）。
  - guard 策略可配置（按 runner config 或 pack policy），但必须有“安全默认”。

### AGT008B Patch 审计事件（patch_applied/patch_rejected）`P0`

- [x] AGT008B
- Deps: AGT003A, AGT008A
- Scope: TS SDK + Runner
- AC:
  - 每次 patch 应产生审计事件，包含：command_id、patch 列表摘要、影响路径集合、是否部分成功。
  - 支持把 patch 摘要 hash 写入 trace（便于对账/复盘）。

### AGT009 TS SDK 提供 Pack 执行期强制库（可被 runner/engine 复用）`P0`

- [x] AGT009
- Deps: AGT004, AGT005
- Scope: TS SDK
- Problem: pack enforcement 不能散落在 runner wrapper 里；需要 SDK 级库，保证多入口一致（CLI/runner/嵌入式 engine）。
- AC:
  - 提供 `enforcePackAllowlist(...)`：对 detect/providers/plugins 做一致性检查，产出标准化结果（ok/block/need_confirm）。
  - 提供 `enforcePolicyGate(...)`：接收标准化 gate input，产出标准化 gate output（hard_block/need_user_confirm/ok）。
  - 提供 `explainGateResult(...)`：生成可给 UI/agent 的可解释 payload（原因、阈值、证据、建议动作）。
  - runner 改为调用 SDK 库而不是自定义逻辑（减少重复与漂移）。

### AGT009A Pack allowlist enforcement API 设计定稿 `P0`

- [x] AGT009A
- Deps: AGT009
- Scope: TS SDK + Docs
- AC:
  - 定义最小 API：
    - `checkDetectAllowed(pack, { kind, provider?, chain })`
    - `pickDetectProvider(pack, { kind, chain, candidates })`
    - `checkExecutionPluginAllowed(pack, { type, chain })`
  - 返回值统一结构：`{ ok: true } | { ok: false, kind: 'hard_block'|'need_user_confirm', reason, details }`。

### AGT009B 在 detect resolver 中强制 allowlist（运行时）`P0`

- [x] AGT009B
- Deps: AGT004A, AGT009A
- Scope: TS SDK + Runner
- AC:
  - 当 detect 需要 provider 选择时，必须通过 pack allowlist 选择或阻断。
  - 当 detect 指定 provider 但不被允许时，必须阻断或确认（按 AGD3），并携带结构化 details。

### AGT009C 在 plugin execution 路径中强制 allowlist（运行时）`P0`

- [x] AGT009C
- Deps: AGT004B, AGT009A
- Scope: TS SDK + Runner
- AC:
  - 执行任意非 core execution.type 前必须校验 pack allowlist（含链维度）。
  - 失败时产生结构化 `need_user_confirm` 或 `error`，并能被事件协议承载。

### AGT009D Runner 迁移：去掉自定义 allowlist/gate 分叉 `P0`

- [x] AGT009D
- Deps: AGT009B, AGT009C
- Scope: Runner
- AC:
  - runner wrappers 不再自己实现 allowlist 逻辑，仅负责把上下文与节点信息传给 SDK enforcement。
  - 提供 6 个回归 fixtures：allow/deny detect、allow/deny plugin、链维度 deny。

### AGT010 TS SDK 提供 “Policy Gate Input 提取器”（从 plan/node/runtime 生成 gate 输入）`P0`

- [x] AGT010
- Deps: AGT005, AGT009
- Scope: TS SDK + Runner
- Problem: gate 输入如果由各个 executor/runner 自行拼，会导致字段缺失与不一致，最终策略无法稳定生效。
- AC:
  - 定义提取规则：从 action/query 元数据、已解析 params、执行类型、detect 结果、预览交易信息中提取 gate input。
  - 对不可得字段明确策略：unknown vs require_confirm（避免静默跳过）。
  - 提供 dry-run 兼容：即使不广播，也能产出尽可能完整的 gate input（并标注缺失来源）。
  - 10+ fixtures：swap/approve/bridge wait 等场景 gate input 结构一致且可追踪到来源。

### AGT010A GateInput 提取规则与“不确定字段”策略定稿 `P0`

- [x] AGT010A
- Deps: AGT005A
- Scope: Docs + TS SDK
- AC:
  - 明确哪些字段必须在写节点执行前可得（否则 hard block），哪些字段可 unknown（但必须触发确认）。
  - 明确从哪里取 risk：action 元信息、pack overrides、运行时附加。
  - 明确 slippage/spend/approval 的取值优先级（params vs calculated vs detect result）。

### AGT010B EVM 写交易预览与资产抽取（用于 gate）`P0`

- [x] AGT010B
- Deps: AGT010A, AGT007A
- Scope: TS SDK + Runner
- AC:
  - 在 dry-run 下也能尽量构造 tx preview（to/data/value/函数签名/关键参数摘要）。
  - 能从常见 ERC20 approve/swap 类动作抽取：token、spender、amount、是否无限授权等字段（若可得）。
  - 提供 6 个 fixtures：approve（有限/无限）、swap（含 slippage）、未知 token（触发确认）。

### AGT010C Solana 指令预览与资产抽取（用于 gate）`P0`

- [x] AGT010C
- Deps: AGT010A, AGT007A
- Scope: TS SDK + Runner
- AC:
  - 指令预览包含：program id、accounts 摘要、data/discriminator 摘要（不泄露敏感）。
  - 对 SPL Token 常见指令（transfer/approve 等）能抽取 token/mint/owner/amount 等字段（若可得）。

---

## 3) P1：显著降低 token 与错误率（让 agent “更像在调用工具”）

### AGT101 Catalog（卡片摘要）规范 + 导出（供检索而非全量 spec）`P1`

- [x] AGT101
- Deps: 无
- Scope: Specs + TS SDK + Runner/CLI（可选）
- Goal: agent 不应吃全量 spec 原文；应吃“可检索卡片摘要”，再基于 pack/能力边界做选择与编排。
- AC:
  - 定义标准 `ActionCard/QueryCard/PackCard` 字段集合（签名/风险/能力/执行类型摘要/依赖摘要/链支持摘要）。
  - SDK 提供导出：workspace -> `catalog.json`（稳定排序、可 hash、可增量更新）。
  - CLI 增加 `catalog` 命令（或 runner 子命令）输出 JSON。

### AGT102 “骨架 -> plan”编排接口（避免生成整份 workflow 文本）`P1`

- [x] AGT102
- Deps: AGT001, AGD1
- Scope: TS SDK + Specs（可选）
- Problem: agent 最擅长输出结构化 skeleton（节点/依赖/引用），不擅长长 YAML；需要一个中间契约降低失败面。
- AC:
  - 定义最小 `PlanSkeleton`（建议直接以 plan 为目标，不再引入另一套 workflow 语法）：
    - nodes: action/query refs、chain、args(ValueRef)、deps
    - policy hints: 风险偏好/确认策略（不等于绕过 pack）
  - 编译 skeleton -> ExecutionPlan，失败时返回结构化错误（用于 patch 修复）。
  - 提供 3 个 skeleton 示例：swap、approve-if-needed、wait-until。

### AGT103 微模板/片段库（fragments）与组合规则（覆盖“意图千变万化”）`P1`

- [x] AGT103
- Deps: AGT102, AGT005
- Scope: Docs + Examples +（可选）TS SDK
- Problem: 完整 workflow 难模板化，但可复用的是稳定的 DAG 结构与控制语义；片段库能显著降低长尾组合错误率。
- AC:
  - 定义至少 10 个 fragments（例如 read-then-write、quote-then-swap、approve-if-needed、guardrail-gate、wait-until、two-phase-commit 等）。
  - 每个 fragment 明确：输入槽位、输出槽位、失败策略、风险标签、适用前置条件。
  - 提供 5 个“长尾意图”示例，展示通过 fragments 组合得到 plan（不输出整 workflow 文本）。

### AGT104 Validator/Planner 错误结构化（让 patch 修复可收敛）`P1`

- [x] AGT104
- Deps: AGT001
- Scope: TS SDK + Runner
- Problem: 人类可读错误不利于 agent 做增量修复；需要结构化错误（路径/原因/建议修复）。
- AC:
  - 所有校验错误统一结构：`{ kind, severity, node_id?, field_path, message, reference?, related? }`
  - runner 的 dry-run 输出同时支持文本与 JSON（JSON 用于 agent）。
  - 提供 10 个失败用例：引用缺失、action 不存在、deps 环、链缺失、pack includes 不匹配等。

#### AGT104A StructuredIssue schema + converters（SDK）

- [x] AGT104A
- Scope: TS SDK
- Notes:
  - 新增 `StructuredIssue` 统一结构与 zod schema。
  - 提供 converters：workspace/workflow/zod/planner 错误 -> StructuredIssue。

#### AGT104B Runner dry-run JSON 输出模式

- [x] AGT104B
- Scope: Runner
- Notes:
  - CLI 增加 `--dry-run-format text|json`。
  - dry-run 增加 `dryRunCompilePlanJson()` 输出 `{ kind, plan_summary, nodes[], issues[] }`。

#### AGT104C 失败用例覆盖（10 cases）

- [x] AGT104C
- Scope: TS SDK + Runner
- Notes:
  - 增加 10 个覆盖用例：引用缺失、action 不存在、deps 环、链缺失、pack includes 不匹配、protocol version mismatch、workflow imports enforcement、execution chain 不匹配、composite step args 错误、plan schema 校验失败。

### AGT105 TS SDK 提供 Catalog Index + pack/capabilities 过滤（为 agent 检索服务）`P1`

- [x] AGT105
- Deps: AGT101, AGT004
- Scope: TS SDK
- Problem: 即使导出 catalog，agent 仍需要“在 pack/能力边界内”的候选集合，否则会反复生成不可执行的计划。
- AC:
  - 提供 `buildCatalogIndex(catalog)`：稳定排序、可按 protocol/version/action/risk/capability 快速筛选。
  - 提供 `filterByPack(index, pack)`：剔除不在 includes/chain_scope/allowlist 内的候选 action/query/provider/plugin。
  - 提供 `filterByEngineCapabilities(index, capabilities)`：剔除引擎不支持的 execution types/detect kinds。
  - 5+ fixtures：同一意图在不同 pack 下候选集变化可追踪且稳定。

### AGT105A “可执行候选集”接口（一次性返回候选 actions/queries/providers）`P1`

- [x] AGT105A
- Deps: AGT105, AGT009A
- Scope: TS SDK
- AC:
  - 提供 `getExecutableCandidates({ catalog, pack, engine_capabilities, chain_scope })`：
    - 返回 actions/queries 列表（含 risk/签名摘要）
    - 返回 detect provider 列表（按 kind/chain/priority）
    - 返回 plugin execution types 列表（按 chain）
  - 输出排序稳定（可 hash），便于缓存与对比。

### AGT106 TS SDK 提供 “解释性摘要生成”（用于 need_user_confirm/审计/UI）`P1`

- [x] AGT106
- Deps: AGT002, AGT009, AGT010
- Scope: TS SDK + Runner
- Problem: agent/用户确认点如果只有原始技术字段，会导致体验差且难以审计；需要可解释摘要。
- AC:
  - 对 plan node 生成摘要：动作/链/关键参数/风险标签/策略命中/交易预览（若可得）。
  - 摘要输出必须稳定、可 hash（便于缓存与审计对比）。
  - runner 在 `need_user_confirm` 事件中携带该摘要（而非仅 reason 字符串）。

### AGT106A 解释性摘要的稳定性与可对账 hash `P1`

- [x] AGT106A
- Deps: AGT106, AGT007A
- Scope: TS SDK + Runner
- AC:
  - 摘要字段顺序稳定、无随机字段（除显式 `ts` 外），可计算摘要 hash。
  - 摘要 hash 写入事件与 trace，便于“用户确认内容”与“实际执行内容”对账。

### AGT107 TS SDK 提供 Agent 端 “最小决策循环”参考实现（非产品，但可测试）`P1`

- [x] AGT107
- Deps: AGT002, AGT003, AGT006
- Scope: TS SDK + Runner fixtures
- Problem: 没有参考 loop，很难验证事件/命令协议是否真的可用；需要一个最小端到端回归。
- AC:
  - 提供一个“无 LLM”的 deterministic agent stub：根据 `node_blocked/need_user_confirm` 自动应用预设 patches/confirm。
  - 用 fixtures 证明：缺 inputs、缺 contracts、策略 gate 三类场景都能闭环推进到完成（至少 dry-run）。
  - CI（或本地）可一键跑这些回归（确保协议不回退）。

### AGT107A Runner fixtures：为 agent-loop 设计的最小三件套 `P1`

- [x] AGT107A
- Deps: AGT006, AGT003
- Scope: Runner + Examples
- AC:
  - Fixture 1：缺 inputs（必须通过 patch 填 `inputs.*` 才能继续）。
  - Fixture 2：缺 contracts（必须通过 solver/patch 填 `contracts.*` 才能继续）。
  - Fixture 3：策略 gate 触发（必须 user_confirm 才能继续，且确认内容可解释）。

---

## 4) P2：可选优化（体验/生态/长期演进）

### AGT201 Trace/Checkpoint 的隐私与最小泄露（agent 场景）`P2`

- [x] AGT201
- Deps: AGT002
- Scope: TS SDK + Runner
- AC:
  - 定义事件与 trace 的 redact 策略（默认不输出私钥、seed、完整 RPC payload、敏感个人信息）。
  - 支持“可审计但不泄露”的 trace 模式（hash/摘要/可选白名单字段）。

### AGT202 计划差分（plan diff）与回放（replay）工具 `P2`

- [x] AGT202
- Deps: AGT001, AGT002
- Scope: Runner/CLI
- AC:
  - `plan diff`：比较两个 plan 的结构差异（节点增删改、deps、writes、执行类型变化）。
  - `replay`：从 checkpoint/trace 回放到某个节点，支持只读回放（无广播）。

---

## 5) 快速落地顺序（推荐）

1. `AGT001/AGT002/AGT006` 让 plan-first 跑通（dry-run 与 execute）。  
2. `AGT004/AGT005` 把 allowlist + policy gate 做成“可执行安全闭环”。  
3. `AGT007/AGT008` 收敛 codec 与 patch guard，稳定跨进程/跨语言互操作。  
4. `AGT101/AGT102/AGT103` 做 token 与错误率的结构性优化。  
