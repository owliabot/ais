# AIS 产品向架构与模块交互（去代码引用版）

面向读者：产品经理、项目 owner、需要评审 AIS 的逻辑架构与落地路径的人。  
本文目标：让非开发读者在 10-15 分钟内获得对 AIS 的“系统图 + 交互树 + 安全边界 + 现状与决策点”的可评审理解。  
不包含：SDK 的函数级对接细节、文件路径/源码引用、如何写代码。

---

## 1. 一页速览（AIS 是什么）

AIS（Agent Interaction Spec）是一套用**结构化文档**描述“智能体如何安全、可验证地与链上协议交互”的标准与实现框架。它把协议交互从“硬编码”变成“可版本化的内容资产”，并提供可执行的运行时模型（计划 + 引擎）。

AIS 的三类核心文档：
- **Protocol Spec（协议说明书）**：协议动作/查询的接口、风险信息、以及链上执行配方。
- **Pack（策略包/上线包）**：选择允许的协议版本与链范围，并定义策略边界（约束、审批、allowlist）。
- **Workflow（工作流）**：跨协议编排 DAG，定义数据流、依赖与运行时控制语义（等待、重试、断言等）。

AIS 的运行时核心：
- **ExecutionPlan（执行计划 IR）**：把 workflow 转成可序列化 DAG 计划，便于调度、暂停/恢复与审计。
- **Engine（执行引擎）**：按计划调度执行，产出事件流；与链 IO 解耦。
- **Executor（链执行适配器）**：负责 RPC/签名/广播/确认等链 IO。
- **Solver（补全与交互层）**：当缺输入/需要用户确认/需要 detect 解析时，提出补丁或暂停。

---

## 2. 设计目标与非目标

设计目标（产品化角度的“为什么这样设计”）：
- **可验证**：执行前就能校验结构、引用、能力与策略边界。
- **可约束**：Pack 是策略边界，能把“允许做什么”落实成强约束与审批门槛。
- **可组合**：Workflow 支持跨协议/跨链组合，且依赖/数据流显式。
- **可恢复与可审计**：计划与事件流可落 checkpoint/trace，支持断点续跑与事故复盘。
- **可扩展**：通过 providers/plugins 扩展 detect 与非核心执行类型，避免无限膨胀核心规范。

非目标（避免范围膨胀）：
- 不把 AIS 当作“协议安全审计替代品”。
- 不把 AIS 当作“钱包托管产品”。
- 不把所有业务策略塞进表达式语言（表达式用于可验证计算与条件，而非通用业务决策引擎）。

---

## 3. 模块树（dendritic 系统图）

下面用“树状结构”展示 AIS 的模块与交互边界（概念模块，不涉及代码）。

```text
AIS 系统
  A. 内容层（可版本化资产）
    1) Protocol Spec
      - 动作 actions / 查询 queries
      - 风险信息（risk_level / risk_tags / 风险说明）
      - 能力要求（capabilities）
      - 执行配方（按链的执行描述）
    2) Pack（策略边界）
      - includes：允许的协议版本集合
      - chain_scope：允许的链范围
      - policy：滑点/授权/审批阈值等策略
      - token_policy：token allowlist 与解析策略
      - providers/plugins allowlist：允许的 detect providers 与执行插件
      - overrides：对特定动作的覆盖（若启用）
    3) Workflow（编排层）
      - DAG 节点：引用协议 action/query
      - 参数与数据流：动态值都用结构化表达
      - 依赖：显式 deps + 隐式引用依赖
      - 控制语义：until/retry/timeout/assert

  B. 工具层（内容的质量与一致性保障）
    - Schema 校验（strict + 扩展槽）
    - Lint（最佳实践）
    - Workspace 校验（workflow -> pack -> protocol 闭环）
    - Conformance 向量（可移植的语义回归测试）

  C. 运行时层（把内容变成可执行）
    - Resolver（引用解析 + 表达式求值 + detect）
    - Planner（workflow -> ExecutionPlan）
    - Engine（调度与状态机）
      - Readiness：节点是否可执行（缺输入/需要 detect）
      - Solver：补全/交互（用户确认、策略 gate、自动填充）
      - Executor：链 IO（读/写/确认）
    - Observability（事件流/trace/checkpoint）

  D. 分发与可信层（生态化方向）
    - 规范化序列化（相同内容得到相同 hash）
    - Registry/Discovery（来源、签名、authority 绑定）
```

产品意义：
- 把“内容资产”与“运行时执行”解耦，才能实现：快速上线新协议内容，同时保持执行安全与一致性。
- 把“策略边界”与“执行适配”解耦，才能实现：同一套 workflow 在不同 pack 下表现不同（例如更保守/更激进）。

---

## 4. 三类文档的交互与边界（谁约束谁）

### 4.1 Protocol Spec 负责“描述”

Protocol Spec 提供：
- 动作与查询的接口（输入/输出形状）
- 风险标签与风险等级
- 执行配方（“怎么调用链上”）
- 能力需求（“引擎必须支持什么能力才能执行”）

它不应该负责：
- 用户审批策略（属于 Pack）
- 环境允许/禁止哪些 providers/plugins（属于 Pack）

### 4.2 Pack 负责“选择 + 收紧”

Pack 是策略边界：
- 选择允许的协议版本集合（includes）
- 收紧链范围（chain_scope）
- 收紧 providers/plugins allowlist（能力边界）
- 定义风控 gate（硬约束 + 审批门槛）

Pack 的本质是：把“系统理论上能做的事情”，缩小为“这次上线/这个用户群允许做的事情”。

### 4.3 Workflow 负责“组合 + 显式依赖”

Workflow 的职责：
- 把动作/查询按 DAG 串起来（可并发、可串行）
- 把数据依赖显式化（通过引用而不是隐式猜测）
- 把等待/重试/断言等运行时控制语义显式化

Workflow 不应该绕过 Pack：
- 一旦 workflow 声明 requires_pack，运行时必须在 pack 的 includes/allowlist/策略边界内执行。

---

## 5. 从“目录内容”到“链上执行”的两条主链路

### 5.1 内容生产与上线（发布链路）

```text
作者/团队产出 Protocol Spec
  -> QA/工具校验（结构、lint、conformance）
  -> Pack Owner 选择 includes + 策略 gate + allowlists
  -> Workflow Designer 组合流程（可选 requires_pack）
  -> Workspace 校验（workflow -> pack -> protocol）
  -> 形成可发布的内容包（版本化）
```

在产品里对应的“上线动作”通常是：
- 上线一个 pack（带策略）或一个 workflow 模板（绑定 pack）
- 给特定用户/场景开启某个 pack 版本（灰度/AB/地域合规）

### 5.2 运行时执行（执行链路）

```text
加载 workflow + (可选) pack + protocols
  -> 建立运行时上下文（inputs/ctx/contracts/nodes/...）
  -> 规划为 ExecutionPlan（DAG 计划）
  -> Engine 调度：
      - readiness 检查
      - solver 补全/请求用户确认/触发 detect
      - executor 做链 IO
      - 输出事件流 + checkpoint/trace
  -> 结果写回上下文（供后续节点引用）
```

产品意义：
- 执行不是“一次函数调用”，而是一个可观测的状态机过程。
- 用户确认点、失败点、重试点都需要产品化（UI/权限/审计/客服）。

---

## 6. 运行时状态与数据边界（为什么能解释与审计）

AIS 倾向使用一个结构化的运行时根对象（概念）：
- `inputs`：用户输入（workflow inputs）
- `params`：当前节点参数（由 bindings/args 求值后形成）
- `ctx`：环境上下文（钱包地址、时间、链连接信息、权限、策略等）
- `contracts`：合约地址映射（按链选择的部署信息）
- `nodes`：每个节点的 outputs/calculated 等运行态
- `calculated`：计算字段或运行时派生值（由引擎/solver 写入）
- `policy`：pack/workflow 的策略视图（供 gate 与表达式使用）

产品意义：
- 任何一个“为什么这样执行”的问题，都能落到“它引用了哪些路径、依赖了哪些 outputs、缺了哪些 inputs”。
- readiness 能给出“缺什么”而不是“出错了”。

---

## 7. 安全边界（从“配置”到“可阻断”）

把安全拆成多层，是为了让产品能设计明确的失败策略（fail fast / require confirm / block）。

### 7.1 结构边界：strict + extensions

未知字段拒绝，扩展只能进 `extensions`，防止配置绕过校验。

### 7.2 语义边界：结构化动态值（ValueRef）

动态值必须显式标注来源：
- 引用（ref）从哪里取
- 计算（cel）怎么算
- 选择/解析（detect）依赖哪些 provider 与能力

### 7.3 能力边界：capabilities + allowlist

协议声明需要的能力，引擎声明可提供的能力；Pack 把 providers/plugins 收紧为 allowlist。

### 7.4 策略边界：policy gate

Pack 的策略 gate 是产品最重要的“控制阀”：
- 硬约束：直接阻断
- 软约束：触发人工确认（need_user_confirm）

建议产品上把 gate 的结果做成可解释的 UI：
- 哪条约束命中
- 需要用户确认的原因
- 风险标签与风险等级

---

## 8. 引擎事件流（产品如何接入“可解释执行”）

对于 PM 来说，引擎是一个“事件驱动流程”，可用来设计：
- 进度展示
- 用户确认对话框
- 失败重试/恢复
- 审计与可追溯

典型事件类别（概念）：
- 计划生成与节点调度：plan_ready、node_ready、skipped
- 阻断与交互：node_blocked（缺输入/需要 detect）、need_user_confirm
- 链上生命周期：tx_prepared、tx_sent、tx_confirmed、query_result
- 等待与恢复：node_waiting、checkpoint_saved、engine_paused

产品意义：
- 把“执行体验”从黑盒变成可解释状态机，减少客服与事故成本。

---

## 9. 当前实现现状（仅描述能力，不引用代码）

### 9.1 已具备（可用于评审与 demo）

- 三类文档的严格结构校验与 YAML 解析
- 目录/工作区扫描与 cross-file 一致性校验（workflow->pack->protocol）
- 基础的策略 gate（token allowlist、滑点、无限授权、风险审批阈值）
- 统一的动态值求值模型（含表达式与 detect，可异步）
- ExecutionPlan（可序列化计划）+ 引擎参考实现（事件流、并发、checkpoint、trace）
- EVM/Solana 的参考执行适配（用于端到端演示）

### 9.2 产品化必须明确的决策点（影响“策略是否真生效”）

这些点建议形成明确的 PRD/设计决议：
- Pack 的哪些字段属于“强约束”必须 enforce，哪些是“提示/建议”
- allowlist 的维度：仅 kind/provider 还是还要按 chain/priority 严格落地
- overrides 的语义：何时合并、冲突时以谁为准、是否可在 UI 中解释
- 金额与 `"max"` 等 sentinel 的产品语义：允许范围、失败策略、与链上编码的对应关系
- workflow schema/版本的统一策略（生态互通与用户心智）

---

## 10. 产品评审清单（用来快速评审一个 pack/workflow 上线）

对一个“要上线的 pack/workflow”，建议至少回答：
- 它允许哪些协议版本？是否都在 includes 内？
- 它允许哪些链？chain_scope 是否覆盖到所有 workflow 节点？
- detect/providers/plugins 的 allowlist 是否足够收紧？是否会误放“逃逸能力”？
- 风控 gate 是否覆盖核心风险：滑点、授权、token allowlist、风险等级审批？
- 执行过程的用户确认点在哪里？文案/原因是否可解释？
- 失败策略是什么：直接阻断、可重试、可恢复、可人工介入？
- 审计如何做：是否记录关键事件、是否可追溯到具体版本与来源？

---

## 11. 术语表（跨职能沟通）

- Protocol Spec：协议说明书（接口 + 风险 + 执行配方）
- Pack：策略包/上线包（协议集合 + 风控策略 + allowlists）
- Workflow：工作流（跨协议编排 DAG）
- ExecutionPlan：执行计划 IR（可序列化 DAG）
- 动态值（ValueRef）：字面量/引用/表达式/检测解析的显式表示
- Detect：需要外部解析/选择的动态值（可异步）
- Capability：引擎能力声明（协议要求/引擎提供）
- Allowlist：白名单（Pack 对 provider/plugin 的收紧边界）
- Solver：补全/交互层（缺输入、需要确认、detect 触发）
- Executor：链执行适配器（RPC/签名/广播/确认）
- Checkpoint/Trace：断点与审计日志（可恢复、可观测）
