# AIS-RS Agent 设计实践与细节（vNext）

状态：实践文档（面向当前 `rust/ais-rs` 实现）  
目标：说明现在 AIS-RS 的 agent 设计理念、关键机制、工程取舍与落地细节。

---

## 1. 设计目标与边界

AIS-RS 的 agent 目标不是“自由聊天机器人”，而是**可执行、可审计、可恢复**的 intent 执行系统：

- 可执行：LLM 输出必须落到严格 schema（`plan-sketch -> plan -> engine`）。
- 可审计：每个关键动作有稳定事件、reason_code、confirmation hash。
- 可恢复：崩溃重启后，靠 checkpoint + side-effect ledger 保证幂等。
- 可控风险：policy gate + need_user_confirm + pack allowlist 为硬边界。

核心边界：

- LLM 负责“规划建议”，不直接触达链执行器。
- runner/engine 负责“验证、约束、执行、审计”，永远是最终裁决层。

---

## 2. 分层架构（当前实现）

1. **LLM Planner（`ais-runner`）**
   - 工具调用协议：`plan.begin` / `plan.propose_segment` / `plan.revise_segment`
   - 发现工具：`list_candidates` / `catalog.search` / `get_candidate_detail`
2. **Compiler（`ais-sdk`）**
   - `compile_plan_sketch` 将 segment 编译成 `ais-plan/0.0.3`
   - 产出结构化 issues（如 `candidate_not_found`、`input_type_mismatch`）
3. **Execution Engine（`ais-engine`）**
   - `readiness -> solver -> policy gate -> executor`
   - 统一事件协议、命令协议、checkpoint 合约
4. **Executors / Plugins**
   - 按 `chain + execution.type` 路由
   - core 与 plugin 统一注册语义

---

## 3. 关键理念（实践准则）

### 3.1 Segment-first，不做 one-shot 全量大 plan

- 默认通过 segment 逐段推进，而不是一次性产出完整大图。
- 每段执行后把事实状态（`state_summary` / `previous_error`）回馈给下一段规划。

### 3.2 Host-enforced FSM（工具调用状态机）

- begin 阶段只允许 `plan.begin`。
- propose/revise 阶段只允许 discovery 工具 + 对应 finalize 工具。
- finalize 工具必须最后且每轮最多一次。

### 3.3 Candidate-first（先发现再细化）

- LLM 先看 name-only index（省 token），再按 ref 拉 detail（保精度）。
- 禁止“臆造协议/动作”。

### 3.4 Schema-first（结构优先于文本）

- LLM 输出只是候选，必须经过 JSON/schema/语义校验与编译。
- 不满足契约立即进入修复闭环，不直接执行。

### 3.5 Safety-by-default（策略为硬边界）

- `safe|assist|yolo` 只影响确认流程，不绕过 hard block 与 allowlist。
- policy gate reason_code 稳定化，便于自动化与追踪。

### 3.6 Event-driven idempotency（事件驱动幂等）

- side-effect 由 executor 显式上报，runner 按 ledger 重放/去重。
- 去掉运行时扫描 fallback，减少隐式推断错误。

---

## 4. 现在的分段机制：怎么“拆”意图？

### 4.1 当前机制

- 是否拆分由 LLM 在 segment 草案里给出：`segment.steps + done + cursor_*`。
- runner 按 `done` 和 `max_segments/max_planner_rounds` 驱动循环。
- prompt 里有“尽量小段、先读后写”的约束，但这是软约束。

### 4.2 现状结论

- **已支持**：小流程一段完成；大流程多段闭环执行。
- **未完全主机强制**：复杂度判定还不是硬策略（更多依赖 LLM）。

---

## 5. 上下文与省 token策略（当前实现）

### 5.1 三层工具信息面

- `list_candidates`：name-only 发现视图（低 token）
- `catalog.search`：按关键词/风险/链过滤（中 token）
- `get_candidate_detail`：按 ref 拉细节（高 token）

### 5.2 轻量 `PlanningMemory`（已落地）

- 缓存范围：`list_candidates` / `catalog.search` / `get_candidate_detail`
- 缓存 key：`(session_id, snapshot_hash, tool, args_hash)`
- 命中后直接返回缓存结果，避免重复 tool 查询和重复 token 消耗。

### 5.3 JSON 预算压缩

- 对 tool result 做 compact（深度/数组/字符串预算）；
- detail 查询 refs 有上限窗口，避免大 catalog 爆上下文。

---

## 6. 鲁棒性：针对真实 LLM 的容错闭环

当前实现已覆盖常见 provider 输出偏差：

1. `segment` 返回 JSON 字符串（而非对象）→ 自动解析。
2. `cursor_next` 返回数字 → 自动转字符串。
3. `cursor_next` 缺失 → fallback `segment.cursor_out`。
4. 工具参数坏 JSON / shape 错误 → 不首错即停，进入 bounded repair（`previous_error -> revise_segment`）。

这部分是“生产必须项”，不是可选优化。

---

## 7. 安全治理链路（执行前后）

1. Planner 产物先编译（`compile_plan_sketch`）  
2. `replace_plan` 受 guard（已完成节点不可随意改写）  
3. Engine policy gate 判定：`ok / need_user_confirm / hard_block`  
4. confirm 走 `safe|assist|yolo` 但不越过 hard block  
5. side effect 事件化记录，checkpoint 可恢复与对账  

---

## 8. 多链与插件原则

- EVM/Solana 是默认核心路径；其它链/链下能力走 plugin 注册。
- `execution.type` 是路由 key，不是“自动可执行承诺”。
- pack allowlist + handler registration 必须同时满足。

---

## 9. 可观测性与调试实践

推荐默认开启：

- `--verbose`：事件流、policy gate I/O、checkpoint 细节
- `--verbose-llm`：system/user prompt、tool defs、tool calls、tool results（含 `cached=true/false`）

这让“LLM 为什么这么做/为什么失败”可以被精确定位。

---

## 10. 当前仍建议继续增强的点

1. **硬策略分段守卫**（host-side）
   - 例如 `max_steps_per_segment`、读写强制拆段、`done=true` 结构上限校验
2. **planner memory 持久化**
   - 与 checkpoint 打通，进程重启后保留 tool 上下文缓存
3. **provider 质量分级与路由策略**
   - 按结构化输出稳定性自动降级/切换模型
4. **自动恢复策略可配置**
   - 不同错误类型的重试次数、是否强制人工介入

---

## 11. 实践总结（简版）

AIS-RS agent 的核心不是“让 LLM 更聪明”，而是：

- 用严格协议把 LLM 收敛成可控规划器；
- 用编译、策略、事件、checkpoint 把执行系统做成可验证状态机；
- 在工程上为真实模型噪声提供容错与恢复路径。

这套设计的价值在于：**即使模型偶发不稳定，系统整体仍可持续推进且不越过安全边界**。
