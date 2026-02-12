# AIS x Agent 最佳交互模式（不考虑历史兼容，面向可实现的设计）

本文目标：在不考虑历史兼容的前提下，给出 AIS 与 Agent 的最佳交互模式，使系统同时满足：
- 可靠：降低“生成整份 workflow 文档”带来的结构/语义错误面
- 省 token：避免把协议知识当 prompt 文本反复搬运
- 可控：策略边界（Pack）可执行、可解释、可审计
- 可实现：能落到明确的模块与接口，支持渐进迭代

本文默认 AIS 的核心组成仍是：
- Protocol Spec（协议能力与执行配方）
- Pack（策略边界与 allowlist）
- Workflow（编排模板）
- ExecutionPlan（可序列化执行计划 IR）
- Engine（调度器）+ Executor（链 IO）+ Solver（补全/交互）

---

## 1. 核心结论

1. **不要把“生成整份 `workflow.yaml`”当作 Agent 的主要输出**。它可以是“可选导出物”，但不应是每次运行都从零生成的交付面。
2. **把运行时的主输入改为 `ExecutionPlan`（plan-first）**：Agent 输出结构化计划或计划增量，runner 执行并以事件流回传。
3. **把 Agent 的主要职责定位为“决策与补丁（patch）”**：选择协议能力、选择策略 pack、补全缺失上下文、处理用户确认点，而不是拼装长文本。
4. **把 Pack 当作硬边界**：planning 阶段过滤不允许的协议/能力；execution 阶段二次 gate（硬阻断 + 软确认）。
5. **把可变信息推迟到运行时**：报价、路由、fee tier、gas、余额、允许的 provider 选择等用 `detect/solver/provider` 解决，不写死在 workflow/plan 中。

---

## 2. 第一性原理：Agent 链上操作如何做到准确、高效、安全

把“链上执行”视为一个高风险、强约束的决策与控制问题，最稳的系统不是让 LLM 生成更多文本，而是让 LLM 在更小的决策面内做选择，并让确定性的组件承担校验与执行。

### 2.1 准确（Correctness）

准确的本质是：**语义唯一 + 可校验 + 可收敛的修复回路**。

- 语义唯一：动态值必须结构化（引用/计算/检测解析不能混在裸字符串里）。
- 可校验：任何计划必须在执行前通过结构与引用校验（协议存在、参数签名匹配、DAG 无环、链选择明确）。
- 可收敛：出错时用“结构化错误 -> 最小 patch”修复，而不是重写整份文档/计划。

### 2.2 高效（Efficiency）

高效的本质是：**token 不用于重复信息搬运；计算不在 LLM 内完成**。

- 协议知识应检索化：只给 Agent 需要的 action/query “卡片摘要”，而不是全量 spec 原文。
- 可变信息后移：报价/路由/选择等交给 detect providers 或 solver 在运行时处理。
- 计划复用：对相同“意图形态”复用模式库（见下文微模板/片段），而不是每次从零生成长结构。

### 2.3 安全（Safety）

安全的本质是：**最小权限 + 明确边界 + 可解释的人类确认点**。

- 最小权限：Pack 的 allowlist 与 policy gate 作为硬边界，防止“能力逃逸”。
- 明确边界：把“可执行单元”拆成可审计节点（读/写/等待/断言），并以 ExecutionPlan 明确 deps 与写入路径。
- 人类确认点：对高风险动作（写交易、无限授权、超阈值滑点、非 allowlist token/provider）强制 `need_user_confirm`，并给出可解释理由。

---

## 2. 为什么“workflow 文档中心”不是最佳模式

你提出的模式是：
> Agent 读取用户意图 -> 结合所有支持的 protocol specs -> 生成 `workflow.yaml` -> runner 执行

这个模式在概念上成立，但不是最佳实现路径，原因如下。

### 2.1 Token 成本结构性偏高

- 协议知识（protocol specs）本质是“可检索内容库”。把它们塞进 LLM 上下文属于把 KB 当 prompt 文本使用，规模越大成本越高。
- workflow YAML 输出具有大量重复样板（nodes/args/ref/deps），LLM 对“长而精确”的结构化文本输出成本高、且易出现细微错误。

### 2.2 错误面不仅是语法，更是语义

workflow 的常见问题不是 YAML 不合法，而是：
- action/query id 不存在或版本不匹配
- 参数名/类型不匹配
- 引用路径（inputs/nodes/ctx/contracts）不解析
- DAG 依赖缺失或隐式引用造成环
- 绕过/违反 pack allowlist 或 policy gate
- 把运行时才能知道的值写死，导致脆弱

### 2.3 修复成本高

当 workflow 出错时，LLM 往往倾向“重写整份文件”，导致：
- diff 巨大，审计困难
- 修复迭代不收敛
- token 继续膨胀

---

## 3. 最佳总体架构：Plan-First + 事件驱动 + 增量补丁

把系统拆成四条相互咬合的闭环，形成可实现的“树状交互（dendritic）”。

```text
用户意图
  -> Agent 决策（选择能力、选择策略、选择模板/计划）
    -> Planner/Compiler 生成 ExecutionPlan（结构化、可序列化）
      -> Engine 执行（事件流）
        -> (分支1) Executor 链上 IO（读/写/确认）
        -> (分支2) Solver 产出补丁/确认请求
          -> Agent/用户响应（patch/approve/provider choice）
            -> 回到 Engine 继续执行
```

关键点：
- **Agent 不直接“写文件”，而是“写结构化计划/补丁”**。
- **runner 不要求一次性完成所有信息**，通过 readiness + solver 把缺口显式化。
- **workflow.yaml** 变成：
  - 可选：为了分享、复用、审计或发布模板
  - 非必需：运行时不依赖它的文本形态

---

## 4. 三种推荐交互模式（按产品形态选）

### 模式 A：模式库/微模板优先（产品默认，最稳）

你提到“workflow 很难模板化，因为用户意图千变万化”。这个判断对“整份 workflow 模板”成立，但对“**可组合的微模板/片段（fragments）**”不成立。

关键点：模板化的对象不必是“完整 workflow”，而可以是**稳定的交互模式**与**可复用的图片段**。

适合：面向用户的主功能（swap/借贷/跨链/复投/止盈止损等）以及大多数长尾组合的子结构。

微模板示例（概念级）：
- Read-Then-Write：先 query 读取必要状态，再 action 写入
- Approve-If-Needed：授权不足则补授权，否则跳过
- Quote-Then-Swap：先报价/路由，再执行交换（报价用 detect/provider）
- Guardrail Gate：执行前做策略检查（滑点、token allowlist、风险等级）
- Wait-Until：轮询等待条件成立（跨链到账、确认、余额变化）
- Two-Phase Commit：先准备/模拟，再要求用户确认后广播

微模板不是“固定协议/固定参数”，而是固定 **DAG 结构 + 控制语义 + 失败策略**。协议差异通过“卡片摘要 + 类型绑定 + detect/solver”填充。

- 产品维护一组 **已评审的模式片段库**（每个片段都有：输入槽位、输出槽位、风险标签、适用前置条件、默认策略）。
- Agent 只做：
  - 选择片段并组合（或选择系统推荐组合）
  - 填 inputs（用户参数）与必要上下文
  - 选择 pack（按用户级别/地区/风险偏好）
  - 处理执行过程中的确认点与缺口补全

优点：
- 最低 token 与最低错误率
- 运营可控：片段可灰度、可 A/B、可下线；组合空间远大于“整 workflow 模板”

缺点：
- 需要定义“片段的输入/输出契约”与组合规则（但这是一次性工程化投入）

### 模式 B：半自动编排（结构化构造骨架 + 强校验 + 增量修复）

适合：长尾协议组合、但仍希望落地为可复用资产。

策略：
- Agent 输出“骨架决策”而非长文档：
  - 选择哪些 protocol/action/query
  - 节点依赖关系
  - 每个节点 args 的来源策略（ref/cel/detect）
- Planner 生成 plan；校验失败则把错误转成结构化反馈，让 agent 只产出 patch（增量修复）。

### 模式 C：直接产出 ExecutionPlan（面向高级用户/系统内部）

适合：系统内编排（agent->runner）或自动化任务。

- Agent 直接输出 ExecutionPlan（或对 plan 的 patch），runner 直接执行。
- workflow 文档仅作为 export/审计产物（可选）。

优点：
- 最省 token（不输出样板文本）
- 最容易做“增量修改”

缺点：
- 对外生态（人类阅读/分享）需要额外 export 工具

---

## 5. Agent 输入应该是什么（最小可用上下文，而非“全量 specs”）

最佳实践：LLM 输入不应包含所有 specs 原文，而应包含 **检索后的最小摘要**。

### 5.1 协议能力摘要（Action/Query 卡片）

每个候选 action/query 只需要：
- 标识：`protocol@version` + `action_id`/`query_id`
- 参数签名：参数名、类型、required、asset_ref 关系、默认值、约束摘要
- 风险摘要：risk_level、risk_tags、是否写操作
- 能力摘要：capabilities_required、是否需要 detect
- 执行摘要：执行类型类别（EVM read/call、Solana instruction、composite、plugin type），以及是否跨链

目标：让 Agent 能做“选择与编排”决策，而不是复制协议原文。

### 5.2 策略摘要（Pack 卡片）

向 Agent 提供：
- 允许的协议集合（includes）
- 允许的链范围（chain_scope）
- allowlist（detect providers、execution plugins）
- policy gate（滑点、无限授权、token allowlist、风险审批阈值）
- “硬阻断 vs 需要确认”的规则（产品决议）

### 5.3 引擎能力摘要（Capabilities）

让 runner 把“我支持什么能力”作为输入给 Agent：
- 支持的执行类型（core + plugins）
- 支持的 detect kinds/providers
- 支持的链（RPC/签名能力）

这样 Agent 的计划不会超出引擎能力边界。

---

## 6. 计划的形态：ExecutionPlan 作为系统契约

将 ExecutionPlan 定位为：
- runner 的主要输入
- 可序列化、可 checkpoint、可 trace
- 节点带 deps、链信息、执行 spec、writes、until/retry/assert 等控制语义

为什么这是最适合 agent 的契约：
- 结构化：LLM 更容易通过工具调用/JSON 输出构造
- 易校验：schema + readiness 可以本地快速反馈
- 易修复：对 plan 做 patch 即可，不需要重写整份 workflow 文本

---

## 7. Runner 与 Agent 的交互协议（建议标准化）

建议把 runner 与 agent 的交互设计成“事件流 + 指令响应”的协议。

### 7.1 Runner -> Agent：事件（Events）

事件要满足：
- 可解释：能给出缺什么、为什么停
- 可审计：包含版本/来源/节点 id
- 可恢复：对应 checkpoint 状态

建议事件类别（概念）：
- `plan_ready`：计划生成
- `node_ready`：节点可执行
- `node_blocked`：缺输入/需要 detect/引用缺失
- `need_user_confirm`：触发策略 gate 或缺少授权/签名
- `query_result`：读结果
- `tx_prepared` / `tx_sent` / `tx_confirmed`：写生命周期
- `node_waiting`：until/retry 轮询
- `error`：错误（建议区分 retryable 与 fatal）
- `checkpoint_saved`：断点保存
- `engine_paused`：无可进展，需要外部输入

### 7.2 Agent -> Runner：响应（Responses）

Agent 的响应不应该是“新 plan 文本”，而应该是：
- `apply_patches`：对运行时上下文的增量补全（inputs/ctx/contracts/policy 等）
- `user_confirm`：用户确认结果（approve/deny + 理由）
- `select_provider`：在 allowlist 内选择 detect provider 或 execution plugin（如需要）
- `cancel`：取消执行（可审计）

### 7.3 Patch 的设计原则

- 增量、可逆（便于回滚/审计）
- 明确作用域：只允许写入特定命名空间（inputs/ctx/contracts/policy）
- 可校验：对 patch 本身做 schema 校验

---

## 8. 策略边界如何落地（Pack 的双阶段 gate）

不考虑历史兼容时，建议把策略 enforcement 做成“双阶段”：

### 8.1 Planning Gate（生成计划前）

目标：尽早过滤不允许的路线，减少无意义生成与后续交互成本。

- 协议集合：只允许 includes 内的 `protocol@version`
- 链范围：节点链必须落在 chain_scope
- allowlist：detect kinds/providers、plugin execution types 必须可用且在 allowlist 内
- 能力边界：capabilities 必须由引擎支持

### 8.2 Execution Gate（执行前/广播前）

目标：防止运行时信息导致越界（例如 detect 选择了不允许的 provider、滑点变大、需要无限授权等）。

- 对每个写节点（或敏感节点）做 policy gate：
  - 硬约束：直接阻断
  - 软约束：`need_user_confirm`，并给出可解释理由
- 所有外部 IO（detect provider、rpc escape hatch）都必须在 allowlist 与审计范围内

---

## 9. 当用户意图“千变万化”时，如何仍然做到可计划与可控

很多意图的多样性来自“目标与约束不同”，而不是来自“执行结构完全不同”。从第一性原理出发，建议把“意图 -> 执行”拆成三层，让变化集中在最适合变化的层里。

### 9.1 意图层（Agent 擅长）

意图层输出一个结构化目标（Goal），例如：
- 目标资产与数量（买到/换成/还款/补仓）
- 可接受的成本范围（滑点上限、费用上限、时间上限）
- 风险偏好（保守/激进、是否允许插件能力、是否允许跨链）
- 合规/来源偏好（只允许某些 pack 或某些已验证来源）

这一层不涉及具体协议与链上调用细节。

### 9.2 规划层（确定性系统擅长）

规划层把 Goal 映射为 ExecutionPlan 的骨架，方法可以是：
- 规则与启发式（对多数产品场景足够）
- 约束驱动搜索（当组合空间变大时）

关键在于：规划层只能使用 pack allowlist 内的能力与协议集合，并把不确定信息通过 detect/solver 延迟。

### 9.3 执行层（引擎擅长）

执行层通过 readiness + 事件流驱动交互，把不确定性显式暴露：
- 缺 inputs/ctx/contracts：请求补全
- 需要 detect：在 allowlist 内选择 provider 或请求用户确认
- 命中策略 gate：硬阻断或软确认

这样意图再多样，也不会导致“让 LLM 一次性生成整份 workflow 文本”。

## 9. token 优化与可靠性工程策略（落地建议）

### 9.1 “检索-选择-构造”替代“全量上下文-全文生成”

把决策拆成多步，每步只输入必要信息：
1. 识别意图与槽位（要做什么、哪些输入缺失）
2. 检索候选 actions/queries（小集合）
3. 选择最小可行流程（少节点）
4. 构造计划（结构化输出）
5. 本地校验与增量修复（最多 1-3 轮）

### 9.2 增量修复（patch）替代重写

将 validator/engine 的错误反馈结构化（节点 id、字段路径、原因、建议修复），让 Agent 只返回 patch。

### 9.3 延迟求值（把易变信息放到 detect/solver）

避免在计划中写死：
- 报价/路由/fee tier
- 链上动态状态（余额、allowance、nonce）
- gas/优先费等

改用：
- `detect`（运行时 provider 解析）
- solver（补全合约地址、填 runtime.contracts、触发用户确认）

### 9.4 缓存与复用

- 缓存 protocol action/query 卡片摘要（而不是原文）
- 缓存 plan 模板（相同意图结构 + 不同 inputs）
- 缓存 conformance/validator 结果（对版本化内容）

---

## 10. 建议的产品化落地路线（最小可用到完整闭环）

### Phase 1：模板优先闭环（最快落地）

- 维护少量高频 workflow 模板
- 每个模板绑定一个 pack（或可选择 pack 版本）
- runner 提供事件流与确认点
- agent 负责填 inputs 与确认交互

### Phase 2：半自动编排（覆盖长尾）

- 引入 action/query 卡片检索与排序（按 pack allowlist + 风险偏好）
- agent 输出“骨架决策”，planner 出 plan
- validator 驱动的 patch 修复闭环

### Phase 3：plan-first 生态化

- plan 成为主要可交换资产（内部）
- workflow 文档作为 export/发布模板
- registry/签名/authority 接入，形成可信来源

---

## 11. 开放问题（需要产品/安全/工程共同决议）

- 哪些策略是硬阻断，哪些是软确认？软确认是否需要不同用户等级差异化？
- detect provider 的选择责任归属：agent 选、runner 选、还是 pack 固定？
- overrides 的语义与优先级：对 action 粒度覆盖如何解释给用户？
- 风险标签体系是否需要标准化（最小集合）以支持 UI 与策略引擎？
- “运行时上下文”允许写入哪些字段，如何防止 solver/agent 越权写入？
