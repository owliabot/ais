# AIS 作为“多链可组合 Agent Tool”的最佳实践方案（v2，无历史兼容负担）

日期：2026-02-14  
范围：AIS specs（`specs/`）、Rust 实现（`rust/ais-rs`）、参考对照（`ref/ironclaw/*`）  
目标读者：AIS 规范/实现维护者、Runner/Agent 开发者、需要做 demo 的工程同学

> 结论先行：AIS 目前的核心方向（plan-first、事件/命令、checkpoint、ValueRef/CEL）是对的；但要把它变成“对接多链、可组合、可多轮修复、同时省 token 的 agent tool”，必须把 **Pack 从“静态校验提示”升级为“运行时硬边界”**，并把 Agent 的工作面从“生成长 workflow 文本”改为“检索 cards → 输出 plan-skeleton/fragments → 引擎事件驱动的 patch/confirm/plan-mutation 闭环”。

---

## 0. 需求与约束（来自当前讨论）

你期望的 demo 形态：

- CLI 里输入自然语言 intent（例：“如果当前 Aave 里 USDC supply APY > 3%，则在 Uniswap 将 10 ETH 换成 USDC 并全部存入 Aave；出错交给 LLM 继续处理；需要确认就阻塞等待用户确认。”）
- LLM 使用 tool-calling（函数调用），支持多轮（执行过程会暂停/报错/缺信息）
- Runner 可直接用私钥签名（仅 demo）
- Aave APY 可用 offchain API；Protocol plugin 不一定是链上合约
- `need_user_confirm` 粒度可按 risk_level 配置；也要支持 yolo 模式（把确认交给 AI 或直接自动批准）
- Agent 允许在执行过程中修改 plan（因为 plan 就是 agent 构造物）
- EVM 不同链逻辑层面一致（但 RPC/链参数仍需配置）
- 动态安装/生成 protocol 的边界可配置：yolo 随便，安全模式需要用户确认/禁止

---

## 1. 现状审视：为什么“spec 本身不一定适合 agent”

AIS spec 的“可执行闭环”本质上是：

- Protocol/Pack/Workflow：内容资产（可审计、可发布、可复用）
- Plan：执行契约（plan-first）
- Engine：执行状态机（事件流 + 命令输入 + checkpoint）

这套分层对“可靠执行”非常友好，但对 agent 直接使用有两个结构性摩擦点：

1) **输入面太大**：让 LLM 直接吞全量 protocol specs 再生成 workflow/plan，token 成本高、错率高。  
2) **修复回路不收敛**：一旦生成的 workflow/plan 有小错误，LLM 往往会“重写整份文档”，导致 diff 巨大、审计困难、再次出错概率更高。

因此，面向 agent 的最佳实践不是让 LLM 直接“写 workflow.yaml”，而是：

- 让 LLM 在 **极小、可校验、可补丁化** 的决策面里工作；
- 把“执行”交给 deterministic engine；
- 把“多轮修复”变成结构化命令（patch/confirm/provider-select/plan-mutation），而不是自然语言反复解释。

这与 ironclaw 的核心优点一致：**显式状态机 + 工具调用 + 审批闸门 + 上下文压缩/自愈**。

---

## 2. Pack 设计评估：现在“思路对”，但“落地不足”

### 2.1 Pack 当前设计的优点

- Pack 把 allowlist（protocol 版本、链范围、providers/plugins）与 policy（审批阈值、硬约束默认值）放在一起，这是正确的“风险边界”抽象。
- Pack 与 Workflow 的闭环校验（requires_pack + includes + chain_scope）能在编译期阻止明显越界。
- Pack 天然适合做“环境/地区/用户等级”的分层配置：同一 workflow 在不同 pack 下被收紧或放开。

### 2.2 Pack 当前最致命的问题（以 `rust/ais-rs` 为例）

如果 Pack 只参与“静态校验”，而没有在执行时变成硬边界，会出现：

- 编译期看似合规，但运行期仍可能：
  - 执行未 allowlist 的执行类型/provider
  - 在关键信息缺失时“默认放行”（policy gate 提取不到字段）
  - 产生不可解释的确认点（confirm hash 不稳定/无法绑定 action 身份）

**结论**：Pack 必须成为 engine 的运行时输入，且必须在执行前/执行中持续 enforce。

---

## 3. “每个 protocol 用 CEL 写动态约束，会不会更好？”

你提出的方向是对的，但需要精确定义“动态约束”的边界，否则会变成不可审计的业务逻辑脚本。

### 3.1 推荐的约束分层（Protocol vs Pack）

1) **Protocol 层（作者提供“可提取、可验证”的安全语义）**
   - `risk_level`、`risk_tags`
   - `hard_constraints`（结构化字段，不是任意字符串脚本）
   - `calculated_fields`（可用 CEL，纯计算，决定性）
   - 可选：`policy_inputs`（见下文），声明 policy gate 所需字段从哪里来

2) **Pack 层（部署/风控方收紧“允许范围”）**
   - allowlists（chains/execution/protocol versions/providers/plugins/tokens）
   - approvals（risk_level 阈值、确认策略、yolo 配置）
   - hard constraints defaults（例如 max_slippage_bps、禁止无限授权）
   - overrides（对特定 action 的更严格约束）

### 3.2 CEL 应该放在哪里

- **适合**：表达“可解释、决定性、只读”的约束/计算，例如：
  - `spend_amount_atomic <= policy.max_spend_amount_atomic`
  - `slippage_bps <= policy.max_slippage_bps`
  - `unlimited_approval == false`
  - `token_out in policy.token_allowlist`
- **不适合**：表达需要外部 IO、非决定性或可被 prompt-injection 影响的逻辑（比如“去搜一下这个协议是不是安全”）。

### 3.3 结论：用 CEL 写动态约束是加分项，但要标准化成“可审计 Policy DSL”

推荐升级路径：

- Pack/Protocol 都允许声明 CEL 约束，但必须：
  - 只允许读取固定的上下文根（`inputs/params/ctx/policy/contracts/nodes.*.outputs/calculated`）
  - 禁止网络、禁止时间随机、禁止不稳定 builtins
  - 所有约束必须输出结构化 “violations” 列表，供 need_user_confirm/hard_block 解释

这样“动态约束”才会增强 correctness，而不是把风险转移给 LLM。

---

## 4. 协议不一定是链上合约：通过“注册 Protocol Handler/Executor 插件”纳入统一边界

你的 Aave APY 例子本质是一个 **offchain query**。这应该被 AIS 一等支持，但正确的落地方式不是把它当成“另一类内建执行类型”，而是把它做成一个 **注册的 protocol handler / executor 插件**（与接入 BTC/Sui/Aptos 等新链是同一类机制）。

### 4.1 统一抽象：Execution Spec 的 `type` 只是一把“路由键”，需要由已注册 handler 解释

建议把执行能力分成两层，并全部纳入 Pack 的 allowlist 与运行时 enforce：

- **Core executors（内建）**：EVM / Solana 是默认支持的（例如 `evm_read`、`evm_call`、`solana_instruction` 等）。
- **Plugin executors（注册）**：除 core 之外的一切执行类型都必须通过“注册 handler”提供（包括 offchain API、以及 BTC/Sui/Aptos 等新链）。

关键在于：无论链上还是 offchain，都必须：

- 有明确的 `execution.type`
- 有明确的参数/返回形状（schema/returns）
- 可被 policy gate 提取关键字段
- 可被 Pack allowlist 控制（包括 execution type allowlist；以及 handler 自身的配置 allowlist，例如域名/endpoint/chain-scope）

### 4.2 Offchain 数据的安全底线（demo 也建议遵守）

- Offchain handler（插件）必须是“显式注册”的执行类型；Pack 要能限制：
  - 是否允许该 execution type（allowlist）
  - 允许的域名/endpoint（handler 自己的 allowlist 配置项；并应被 pack 覆盖/收紧）
  - 超时/重试
  - 是否允许把 offchain 结果用于触发资金动作（例如 APY gate）
- Engine/runner 应把 offchain query 结果写入 `nodes.<id>.outputs`，并在 trace 中标注 `source=offchain`。

---

## 5. Agent 最佳交互契约：Plan-Skeleton / Fragments 优先，而不是直接写 Workflow

AIS 作为 agent tool 的核心建议是：

- LLM 输出：`ais-plan-skeleton/0.0.1`（或 fragments 组合结果）
- SDK 编译：skeleton → `ais-plan/0.0.3`
- Engine 执行：plan → events
- LLM 多轮介入：仅在事件边界生成结构化命令或修改 plan

这与仓库已有方向一致（`docs/ais-plan-skeleton.md`、`docs/ais-fragments.md`），建议把它升级为“默认主路径”。

---

## 6. Runner + LLM 的“外层 agent loop”设计（对标 ironclaw 的 worker loop）

### 6.1 新增一个 demo 入口：`ais-runner agent`

建议在 `ais-runner` 增加一个子命令（名字仅示例）：

- `ais-runner agent --workspace <dir> --pack <name@version> --config <runner.yaml> [--mode safe|yolo]`

职责：

1) 构建候选集合（Catalog → Pack 过滤 → Engine capabilities 过滤）  
2) 让 LLM 选择并产出 plan-skeleton/fragments  
3) 编译/校验（失败 → 把 issues 结构化喂给 LLM 要求产出 patch）  
4) 执行 plan（事件驱动）  
5) 暂停/失败时，进入多轮修复/确认闭环  

### 6.2 LLM 看到的 tool-calling API（建议最小集合）

把 LLM 的工具面做小，避免它“自由写大 JSON”。推荐工具（概念名）：

- `ais.catalog.search(query, pack, mode, chain_scope?) -> Action/Query cards (index)`
- `ais.catalog.get_detail(refs[]) -> Detail cards`
- `ais.plan_skeleton.compile(skeleton_json) -> { ok, plan_json | issues[] }`
- `ais.plan.run(plan_json, runtime_json?, checkpoint?) -> event_stream_handle`
- `ais.engine.next_event(handle, max_events, summarize=true) -> { events[], summary }`
- `ais.engine.send_command(handle, command_jsonl_line) -> ack`
- `ais.plan.diff(before_plan, after_plan) -> diff`

关键：`next_event(... summarize=true)` 必须做“事件摘要/压缩”，避免把 JSONL 洪水喂给模型。

### 6.3 事件驱动的多轮闭环（核心）

外层 loop 的伪流程：

1) 读取事件摘要（包含：当前暂停原因、节点、缺失字段、可选候选、confirmation_hash 等）
2) LLM 决策输出以下之一：
   - `user_confirm`（approve/deny）
   - `apply_patches`（补 inputs/ctx/contracts 等）
   - `select_provider`（detect/provider）
   - `replace_plan`（见下节 Plan Mutation）
   - `cancel`
3) runner 把命令送回 engine，继续执行

这就是 ironclaw 的“工具调用 + 状态机 + 审批闸门”在 AIS 上的对应落地。

---

## 7. need_user_confirm：按 risk_level 配置 + 支持 yolo，但必须可审计

### 7.1 建议的确认策略配置（Pack/Runner）

最小三档：

- `safe`：risk_level ≥ N 必须人类确认；缺关键信息（missing/unknown）默认拒绝或强确认
- `assist`：risk_level ≥ N 需要确认，但允许 LLM 代确认（仍要记录 confirmation_hash + 决策理由）
- `yolo`：自动批准（仍记录 policy gate input/output 与 hash）

### 7.2 confirmation_hash 必须绑定“语义摘要”，并且可复用

确认不是“点一下继续”，而是要绑定：

- action_ref（协议身份）
- chain/execution_type
- 风险字段与阈值命中原因
- 关键金额/授权/滑点字段（或其缺失/未知）

这样才能避免：

- 用户/LLM 批准 A，实际执行 B（TOCTOU）
- 多轮重试后确认状态漂移

---

## 8. Plan Mutation：允许修改，但要可恢复、可对账、可限制

你希望 agent 在执行中可以修改 plan。建议把这件事“协议化”，否则会破坏 checkpoint/replay 的稳定性。

### 8.1 推荐语义：Plan Epoch（计划分代）

引入概念（不一定进入 spec，但 runner/engine 必须实现）：

- 每个运行绑定一个 `plan_hash`（已有）
- 如果 plan 发生结构性变更：
  - 生成新 plan（新 hash）
  - 记录 `plan_parent_hash`（在 extensions 里）
  - 运行切换到新 epoch

### 8.2 允许的变更（建议默认策略）

为了 demo 能跑、又不至于把系统弄成不可控，建议默认允许：

- 增加新节点（补前置查询、补 approve-if-needed、补 wait-until）
- 调整未执行节点的 args/condition/until/retry/timeout
- 替换 provider 选择（如果 detect/provider 属于 plan 内容）

默认禁止或强确认：

- 删除已执行节点（破坏审计链）
- 修改已执行节点的 execution（相当于篡改历史）
- 引入 Pack 未 allowlist 的协议/执行类型/域名

### 8.3 操作方式：不要让 LLM 直接“编辑整份 plan”

建议提供工具：

- `ais.plan.propose_patch(plan, intent, current_state_summary) -> { patched_plan, diff, requires_confirm }`
- 或让 LLM 输出受限的 “plan patch” DSL（类似 runtime patch，但作用于 plan）

runner 必须：

- 对新 plan 重新做 pack/enforce + capability 校验
- 输出 plan diff 给用户/LLM
- 对高风险 diff 触发 need_user_confirm（可以进入 yolo/assist/safe 策略）

---

## 9. Token 策略：把“协议知识”从 prompt 搬到“可缓存检索”

### 9.1 两级 cards 是省 token 的核心

强制 LLM 只看：

- index cards（可搜索、可排序、很短）
- 选择后再拉 detail cards（仅少量）

避免：

- 直接把 protocol.yaml 全文塞进上下文
- 或者让 LLM 自己“去网上查、自己总结”再写 plan（不可控且不可审计）

### 9.2 事件摘要（必须做）

事件流很长时，LLM 应该只看到：

- 当前暂停原因（blocked/confirm/error/no_progress）
- 相关 node_id/action_ref/chain
- 缺失字段列表 + 候选值（如果有）
- policy gate 命中摘要 + confirmation_hash
- executor_error 的结构化 reason（不要整段日志）

---

## 10. EVM “逻辑一样” vs “执行配置不同”

你说 EVM 链逻辑层面一样，这对 **execution type** 是对的（`evm_read/evm_call`），但 runner 仍必须按链配置：

- RPC endpoint、timeout、重试
- signer（demo 私钥）
- nonce 并发策略（同地址同链写交易默认串行）
- gas/fee 策略（demo 可简化，但至少要可配置上限，防止意外燃烧）

建议把这些都视为 Pack/Runner 的“环境安全边界”，而不是 LLM 的自由发挥空间。

---

## 11. 动态安装/生成 Protocol：配置化，但必须分级治理

建议分三档（与 mode 对齐）：

- `safe`：禁止动态安装/生成；只允许 workspace/registry 中已审核协议
- `assist`：允许安装，但必须：
  - 来源受信（registry + integrity）
  - 新协议必须经过 compile + dry-run + simulate（若支持）+ need_user_confirm
- `yolo`：允许任意来源/LLM 自生成，但仍要：
  - 记录来源（uri/hash）
  - 记录所有执行与确认摘要

最重要的是：即使 yolo，也不要让“动态协议”绕过 Pack allowlist。yolo 只是降低确认门槛，不是取消边界。

---

## 12. 对 `rust/ais-rs` 的具体改造建议（最小闭环里程碑）

下面按“先能 demo → 再变稳”的顺序切分。

### M1：把 Pack 真正接入执行（ correctness 第一）

- runner 解析 active pack（来自 workflow.requires_pack 或 CLI 参数）
- 将 pack 映射为 engine 的 `PolicyEnforcementOptions`（chains/execution/action refs/thresholds/yolo 策略）
- policy gate 输入必须携带 action_ref/risk 信息（见 M2）

### M2：贯通 action_ref + risk_level/risk_tags 到 plan 节点与 policy gate

- planner 在编译 workflow/plan-skeleton 时，把 action/query 的身份与风险字段写入 plan node（或写入 node.source + node.extensions 的标准槽位）
- policy gate 从 plan node 读取这些字段，而不是靠 method 字符串猜

### M3：补齐 Catalog Cards（省 token 的必要条件）

- `build_catalog` 输出至少对齐 spec 建议字段（risk、params/returns、description）
- 提供 “detail card” 获取接口（或在 catalog 中按需加载）

### M4：新增 `ais-runner agent` 外层循环（多轮 tool-calling）

- 接入一个 LLM provider（tool-calling）
- 实现事件摘要/压缩
- 在 paused/error 时生成命令并继续执行
- CLI 在 need_user_confirm 时阻塞等待（用户/LLM 根据 mode 决定）

### M5：Plan Mutation（plan epoch + diff + 再确认）

- 引入 `replace_plan`（runner 层实现即可，engine 不一定要理解）
- plan diff 输出 + risk-based confirm
- checkpoint/runtime 迁移策略（最简单：新 plan 从当前 runtime 继续跑，但记录 epoch 链）

---

## 13. 用你的例子走一遍（推荐的执行形态）

Intent：

> 如果 Aave USDC supply APY > 3%，则 Uniswap swap 10 ETH→USDC 并存入 Aave；中间需要确认就停。

推荐 plan-skeleton（概念）：

1) `aave_apy`：offchain query（通过已注册的执行插件，例如 `execution.type = "aave_apy_query"`）→ outputs.apy  
2) `swap`：`condition: aave_apy.outputs.apy > 0.03` + quote-then-swap fragment（可能含 approve-if-needed）  
3) `deposit`：依赖 swap 输出 amount + approve-if-needed → aave deposit  

执行中：

- policy gate 命中（滑点/无限授权/金额超阈值）→ need_user_confirm（按 mode 自动/人工/LLM）
- RPC 失败/回执超时 → error → LLM 选择 retry/调整 timeout/取消
- 发现缺 inputs（token 地址/decimals/router）→ solver 提供候选 → LLM 产出 apply_patches 或选择 provider

整个过程中 LLM 不需要“重写 workflow”，只需要在边界点给出结构化命令或 plan mutation。

---

## 14. 总结：Pack 是否“好”？

Pack 的核心定位非常好：它应该是 AIS 作为 agent tool 时的 **第一安全边界**。

但要让它真正“好用且适合 agent”，必须做到三点：

1) **运行时硬 enforce**（不是只做静态校验）
2) **可解释且可哈希的确认摘要**（confirmation_hash 绑定语义）
3) **可与 CEL 约束融合**（成为审计友好的 Policy DSL，而不是脚本口子）

做到这三点后，AIS 就会从“可执行文档规范”升级为“可组合、可修复、可治理的 agent tool 体系”。
