# 从 Pi（OpenClaw 内核 Agent）得到的启示：AIS & AIS-SDK 的最小闭环、可扩展与可审计设计

日期：2026-02-06  
输入材料：`ref/pi.md`、`ref/pi-mono/`（Pi Monorepo）  
目标：分析 Pi 的实现与设计哲学，提炼对 AIS spec 与 AIS TS SDK（执行闭环）最有价值的启示，并给出“**不考虑历史兼容**”前提下的收敛/重构建议。

---

## 1. Pi 到底解决了什么问题（抽象层面）

Pi 的核心不是“聪明的 AI”，而是一个 **最小但可靠的 agent harness**：

- **把 LLM 变成可控的状态机**：事件流、工具调用、可重放会话、可 compaction。
- **把能力扩展变成“资源加载 + 插件”**：skills / prompt templates / extensions / themes / packages。
- **把 UI/集成方式拆成多模式**：interactive / print / JSON / RPC / SDK。

这和 AIS 的目标（“agent 构造可执行 spec，代码可靠执行，必要时再让 AI 决策”）在范式上高度一致：**LLM 负责生成结构化计划，执行与状态管理由确定性代码承担**。

参考：
- Pi 的哲学（极小核心 + 扩展系统 + 会话树/压缩）：`ref/pi.md:1`
- coding-agent 的定位（“minimal terminal harness”，多模式 + extensions/skills/prompts）：`ref/pi-mono/packages/coding-agent/README.md:1`

---

## 2. Pi 的实现中最值得抄作业的点（implementation-level patterns）

### 2.1 “事件流”是一等公民（不是日志）

Pi 的 `AgentSession` 把所有模式（interactive/print/rpc）共享的核心逻辑收敛在一个类里，并向外统一发事件：

- agent_start / turn_start / message_update / tool_execution_* / agent_end
- 外挂扩展也通过事件系统挂钩（before_agent_start、turn_start、turn_end、session_switch…）

参考：
- `ref/pi-mono/packages/coding-agent/src/core/agent-session.ts:1`（核心循环、扩展事件注入、队列/中断/重试/压缩）
- `ref/pi-mono/packages/agent/README.md:1`（Agent runtime 的事件序列定义）

**启示（AIS-SDK）**：你现在的 `EngineEvent` 已经是正确方向，但要把它当作 *API 合约*，而不是 debug 输出。后续所有执行器/桥/solana/轮询，都必须以事件方式“可观察”。

### 2.2 “LLM 上下文”与“系统内部状态”严格分层

Pi 在会话文件里区分了两类扩展数据：

- `CustomEntry`：**只用于扩展持久化状态**，不进入 LLM 上下文  
- `CustomMessageEntry`：**会进入 LLM 上下文**（可显示/隐藏），并允许带扩展 metadata（不进 LLM）

这让 Pi 可以在不污染 LLM 的情况下保存大量内部状态、指标、索引、缓存、UI 信息。

参考：
- `ref/pi-mono/packages/coding-agent/src/core/session-manager.ts:1`（CustomEntry vs CustomMessageEntry）
- `ref/pi-mono/packages/agent/README.md:1`（AgentMessage vs LLM message，convertToLlm/transformContext）

**启示（AIS & SDK）**：你要的“执行全自动、出错才丢给 AI”最怕的就是上下文被执行日志淹没。  
因此 AIS-SDK 应该把：
- **全量执行 trace / rpc 请求响应 / receipts / logs** 存在 *engine trace*（外部持久化，供审计/恢复/调试）
- 只在 `need_user_confirm` / `error` 时生成 **精简的、结构化的摘要消息**（给 AI）

这比“把所有执行历史塞进 prompt”稳定太多。

### 2.3 “资源加载器”统一管理扩展、skills、prompts（并支持多来源）

Pi 的 ResourceLoader 支持：

- 全局目录（`~/.pi/agent/...`）
- 项目目录（`.pi/...`）
- 通过 package manager 安装的资源（npm/git）
- 显式路径覆盖（CLI 参数），以及 reload

参考：
- `ref/pi-mono/packages/coding-agent/src/core/resource-loader.ts:1`
- `ref/pi-mono/packages/coding-agent/README.md:1`（Pi packages / install / config / update）
- skills 规范化（frontmatter 校验、可禁用自动注入）：`ref/pi-mono/packages/coding-agent/src/core/skills.ts:1`

**启示（AIS 生态）**：AIS 本质也是“资源体系”（protocol specs / packs / workflow templates / solvers / executors）。  
与其把一切塞进 spec，不如：
- spec 保持最小表达
- 把“协议集合、策略、执行器、桥适配器”等通过 **package/插件机制**交付与更新

这能避免 AIS 越做越像“大而全应用”，同时让生态可扩展。

### 2.4 会话是树（branching），不是线（这是“可调试 agent”的核心）

Pi 的 session 设计是 JSONL + (id,parentId) 的树结构，支持：

- 从任意历史节点分叉继续（/tree）
- compaction 也是一种“结构化历史操作”

参考：
- `ref/pi-mono/packages/coding-agent/src/core/session-manager.ts:1`（tree session + migration）
- `ref/pi.md:1`（branching / compaction 的价值）

**启示（AIS 执行闭环）**：跨链工作流天然会遇到“策略分叉/失败恢复/重新路由”。  
与其只做线性的 checkpoint（覆盖式），更优雅的是做“执行树”：

- 每次需要 AI 决策/用户批复时，创建一个分叉点
- 不同决策路径形成不同分支（可审计/可对比）
- 任意分支都可从某 checkpoint 继续跑

这会把“AI 处理错误”从黑盒变成可追踪的工作流。

---

## 3. 对 AIS Spec 的直接启示（如何增强表达而不变冗余）

### 3.1 小核心 + 强表达：把“循环/等待”做成引擎语义，而不是新增一堆 execution types

Pi 的哲学是：核心只保留必要机制，其余通过扩展/脚本完成。映射到 AIS：

- 不要为每个桥、每种确认方式都新增 execution type
- 把通用控制面抽象出来：`retry / until / timeout`（node-level），由 engine 统一实现

这能覆盖你提出的“跨链到账轮询、后置检查、状态达标再继续”，同时不把 spec 写爆。

### 3.2 多链 workflow 的关键是 per-node `chain`（最小增量获得最大表达）

Pi 的能力是“同一个 session 里可以做不同类型任务”。映射到 AIS workflow：

- 每个节点需要独立 chain（否则 bridge 场景只能拆成多个 workflow 或靠外部胶水）
- planner 应按 node.chain 选择 execution spec

这同样属于“最小字段增量，最大表达收益”的改造。

> 这部分的详细方案已在 `docs/design-multichain-workflow-engine.md:1` 给出。

### 3.3 技能/模板 vs 规范：把“推荐前置/后置检查”做成 lint/模板，不要做成强制 spec

Pi 不强推 plan mode/to-do，而是让你用 prompt templates / skills 去实现自己的流程。映射到 AIS：

- “余额/allowance/到帐检查”非常重要，但不应变成 action 必填强语义（容易导致 spec 臃肿）
- 更优雅：提供 **pack/workflow 模板** 或 **protocol lint 建议**（recommended checks）
- 真正执行层仍然只执行 workflow DAG（确定性）

---

## 4. 对 AIS TS SDK 的直接启示（架构与工程形态）

### 4.1 把 Engine.runPlan 走向“可集成产品化”的三个步骤

你现在已经有了 `runPlan()` + checkpoint（很好）。下一步建议直接抄 Pi 的“多模式输出”：

1) **JSONL 事件输出模式**（给 agent executor/服务集成）
2) **RPC 模式**（进程集成，外部系统可驱动引擎）
3) **SDK 模式**（库调用）

Pi 的 coding-agent 本质就是把一个可流式的核心 loop 做成多模式入口。AIS-SDK 的执行闭环也应如此，而不是只给“函数库”。

### 4.2 Engine Trace：区分“可审计的执行历史”和“LLM 可消费摘要”

建议新增两个并行持久化面：

- `EngineTrace`（JSONL、树结构）：全量事件（含 rpc 输入输出、receipt、logs、attempts、耗时）
- `AiBrief`（可选）：当需要 AI 介入时，把 trace 压缩成结构化摘要（只包含关键信号）

对应 Pi：
- session 的 tree（trace）
- compaction/branch summary（brief）

### 4.3 插件体系：用“资源加载器”管理 solver/executor/protocol packs（像 Pi packages 一样）

Pi 的 extension/skills/prompts 都能从本地目录或 package 安装。对应 AIS：

- protocol specs / packs / workflow templates：属于“内容资源”
- solvers / executors / detect providers：属于“代码资源”

建议把它们统一成 **AIS Package**（manifest + 版本 + 来源），由 CLI 安装/启用/更新；SDK 运行时只消费已加载资源。

这样你就能在不修改 SDK 核心的情况下：
- 添加一个新的桥协议执行器
- 添加一个新的链 executor
- 添加一个新的路由/报价 solver

### 4.4 让“AI 介入错误处理”变成可控接口（不要靠文本猜）

Pi 的 agent-core 强调 message types 与转换；AIS-SDK 也应把错误结构化：

- error code（可归因：RPC 失败 / revert / insufficient funds / nonce / timeout / chain unavailable）
- node/execution/chain/attempt/elapsed
- 可选动作建议（retry/backoff/ask user/abort/branch）

让 agent 在 `error` 事件上做“策略函数”，而不是把日志丢给 LLM 让它自由发挥。

---

## 5. 建议的“收缩”清单（避免 AIS & SDK 失控）

结合 Pi 的哲学（“What’s not in Pi”）对 AIS 的启示：有些东西越晚内置越好。

建议 AIS 核心暂时不要内置/强耦合：

- 大而全的“桥统一协议”抽象（先用 per-node chain + bridge 协议 spec 组合表达）
- UI/审批流（保持 `need_user_confirm` 事件协议，交给上层产品）
- 复杂的工作流 DSL（保持 DAG + ValueRef/CEL；把模板放在 packs/skills）
- 超多工具/能力声明（保持 capabilities 最小集合，扩展走 package）

---

## 6. 一针见血的结论（对 AIS 的“最优雅”落地路径）

从 Pi 的实现可以得到一个非常清晰的路线：

1) **Spec 只做强表达的最小核心**（DAG + per-node chain + retry/until/timeout + ValueRef/CEL）  
2) **SDK 只做确定性执行闭环**（plan/readiness/solver/executor/engine + event stream + trace tree）  
3) **生态通过 packages/extensions 扩展**（协议/桥/链/策略/模板）  
4) **AI 只在决策点介入**（need_user_confirm/error），上下文来自结构化摘要而不是全量日志

这样 AIS 不会越来越臃肿，但表达与可扩展性会非常强。

