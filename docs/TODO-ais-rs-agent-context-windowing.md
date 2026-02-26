# TODO: AIS-RS Agent Context Windowing & Memory Accumulation

状态: Draft  
日期: 2026-02-26  
范围: `rust/ais-rs/crates/ais-runner`（主），联动 `ais-sdk`（少量 compile/ref lint）  
目标: 保留 `list_candidates`，并把 tool/fact/context 改为“持续追加 + 接近窗口上限再按策略压缩”，降低 discovery 循环和遗忘。

---

## 1. 决策项（按你当前反馈，默认采纳）

### AISRS-CTX-D01: 不禁用 `list_candidates`

- 决策: 保留 `list_candidates`，但改为受控工具（默认不优先、不可重复滥用）。
- 规则:
  - Ground/Todo phase 可用。
  - Propose/Revise phase 可用，但只有在 `tool_memory_projection` 缺“全局协议清单”时才建议调用。
  - 同一 `snapshot_hash` 下重复 `list_candidates {}` 不应再次发起真实发现。

### AISRS-CTX-D02: 上下文采用“累积日志 + 近窗压缩”，不是“每轮重建+大幅裁剪”

- 决策: `state_summary` 保留关键累计内容（tool/fact/refs/errors/todo progress），仅在逼近窗口阈值时触发分级压缩。
- 目标: 避免模型“刚查完就忘”，减少重复 tool call。

### AISRS-CTX-D03: `tool_memory_projection` 改为长期滑窗（可覆盖去重）

- 决策: 每轮新 tool 结果增量并入 memory，full 结果覆盖 digest/summary，同 ref 只保留最新高价值版本。
- 目标: 稳定提供“最近可用能力、签名、schema/topic 合同”。

### AISRS-CTX-D04: 上下文预算以“当前轮窗口使用率”驱动，不用累计 token 驱动

- 决策: 压缩与否基于当前请求 `context_window_input_tokens / context_soft_limit_tokens`。
- 目标: 有余量时不急着压，压力高时再收敛。

---

## 2. 推荐改法（结构）

1. 把 `state_summary` 从“delta 风格（context_unchanged 时只给极简）”改成“始终给完整投影（预算内）”。  
2. `tool_memory_projection` 从“小容量快照”升级为“按价值分层的滑窗记忆”:
   - L1: guide schema/topic 摘要（高优先级，少量稳定）
   - L2: candidate_detail 精简签名（中优先级）
   - L3: catalog.search 命中 refs（低优先级，按 recency 淘汰）
3. `list_candidates` 改成“可用但限流”:
   - 同 snapshot 下首次可调用；
   - 如果 projection 已覆盖协议组清单，prompt 明确先复用，不重复调。
4. 压缩策略从“固定预算”改为“阈值分段”:
   - 使用率 < 70%: 不压缩，保留完整累计；
   - 70%~85%: 轻压缩（字符串裁剪、去低价值 catalog 项）；
   - 85%~92%: 中压缩（降 detail 深度）；
   - >92%: 强压缩（仅保留高优先级索引和最近失败修复关键信息）。
5. 对 planner 引入“重复调用抑制”:
   - 若连续两轮 `catalog.search` query 语义近似且命中为空，强制转向 `list_candidates` 或 `get_candidate_detail`。

---

## 3. 可追踪 TODO（按收益/工作量排序）

### AISRS-CTX-001: 去掉 `context_unchanged` 极简分支，改为完整可预算 summary

- 优先级: P0
- 工作量: M
- 状态: todo
- Deps: 无
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/context_view.rs`
- 交付:
  - `next_summary` 在 `context_unchanged=true` 时仍输出完整（预算后的）上下文，不再只给极简字段。
  - 保留 `context_hash/context_version/context_unchanged` 作为元信息，不作为裁剪开关。
- 验收:
  - 跨 phase（ground/todos/propose/revise）输入中，`input_registry/canonical_context/tool_memory_projection/todo_state` 均持续可见。
- 测试:
  - 新增单测: 连续两轮 hash 相同，summary 字段完备性不下降。

### AISRS-CTX-002: 实现“近窗触发”上下文压缩策略（基于使用率分段）

- 优先级: P0
- 工作量: M
- 状态: todo
- Deps: `AISRS-CTX-001`（硬依赖）
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/context_view.rs`
  - `rust/ais-rs/crates/ais-runner/src/agent/intent_segmented.rs`
- 交付:
  - 新增 `context_pressure_level` 计算（由 `context_window_input_tokens/context_soft_limit_tokens` 推导）。
  - 分段策略落地: `<70%` 不压缩，`70~85%` 轻压，`85~92%` 中压，`>92%` 强压。
  - `--verbose-llm` 打印 pressure level 与触发动作。
- 验收:
  - 大部分轮次（低压）不触发压缩动作。
  - 压力升高时，压缩动作可预测且稳定。
- 测试:
  - 参数化单测覆盖四档 pressure。

### AISRS-CTX-003: `tool_memory_projection` 升级为持久滑窗（full 覆盖 non-full）

- 优先级: P0
- 工作量: M
- 状态: todo
- Deps: `AISRS-CTX-001`（硬依赖）
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/planning_memory.rs`
  - `rust/ais-rs/crates/ais-runner/src/agent/orchestrator.rs`
- 交付:
  - memory 中按 `tool+semantic_key` 合并，同语义条目 full 覆盖 digest/summary。
  - 默认容量上调（保留可配置）:
    - catalog entries: 2 -> 6
    - detail entries: 2 -> 6
    - guide entries: 2 -> 4
  - 仅在 pressure>=中压时开始大幅裁剪 catalog 层。
- 验收:
  - 多轮后 projection 仍能覆盖最近关键 refs + schema/topic。
  - discovery 重复调用下降（见 CTX-008 指标）。
- 测试:
  - 单测: full/non-full 去重覆盖；
  - 单测: 高压下按层级淘汰。

### AISRS-CTX-004: `list_candidates` 受控复用策略（不禁用）

- 优先级: P0
- 工作量: S
- 状态: todo
- Deps: `AISRS-CTX-003`（硬依赖），`AISRS-CTX-001`（软依赖）
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/intent_segmented.rs`
  - `rust/ais-rs/crates/ais-runner/src/agent/planning_memory.rs`
- 交付:
  - prompt 新增规则: 有 projection 时优先读 projection，缺全局协议清单才调用 `list_candidates`。
  - 同 snapshot hash 内，`list_candidates {}` 调用默认命中 memory，不再产生冗余发现。
- 验收:
  - propose/revise phase 中 `list_candidates` 次数显著下降，但仍可在缺信息时调用成功。
- 测试:
  - e2e: 同 snapshot 多轮修复，仅首次出现真实 `list_candidates`。

### AISRS-CTX-005: `catalog.search` 查询归一化与近似去重（减少换词空转）

- 优先级: P1
- 工作量: M
- 状态: todo
- Deps: `AISRS-CTX-003`（软依赖）
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/candidates.rs`
  - `rust/ais-rs/crates/ais-runner/src/agent/intent_segmented.rs`
- 交付:
  - `matches_keyword` 从整串 `contains` 升级为 tokenized matching（支持 `erc20/token`, `native/eth` 等轻量同义归一）。
  - `catalog.search` cache key 对 query 做归一化（lowercase + token sort + 去噪词）。
- 验收:
  - `erc20 balance`/`token balance` 命中一致性提升。
  - 同义查询命中缓存，重复查询下降。
- 测试:
  - 单测覆盖同义词和 token 顺序变化。

### AISRS-CTX-006: planner 重复调用抑制器（phase 内防空转）

- 优先级: P1
- 工作量: M
- 状态: todo
- Deps: `AISRS-CTX-004`、`AISRS-CTX-005`（硬依赖）
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/intent_segmented.rs`
- 交付:
  - 记录近 N 轮工具调用轨迹。
  - 发现“连续近似空结果 search”时，向模型注入结构化提示（下一步建议转 `list_candidates/get_candidate_detail/guide.get`）。
- 验收:
  - “query 换词连续空结果”循环被打断，轮次数下降。
- 测试:
  - 构造失败用例，确认抑制器触发后可转向 finalize。

### AISRS-CTX-007: 观测指标与日志增强（验证优化是否生效）

- 优先级: P1
- 工作量: S
- 状态: todo
- Deps: `AISRS-CTX-002`、`AISRS-CTX-003`（硬依赖），`AISRS-CTX-004~006`（软依赖）
- 涉及:
  - `rust/ais-rs/crates/ais-runner/src/agent/intent_segmented.rs`
  - `rust/ais-rs/crates/ais-runner/src/agent/orchestrator.rs`
- 交付:
  - 新增指标:
    - `duplicate_tool_call_ratio`
    - `empty_search_streak_max`
    - `memory_hit_rate_by_tool`
    - `phase_round_count`
  - `--verbose-llm` 输出每轮摘要（是否压缩、压缩级别、memory 命中情况）。
- 验收:
  - 能从一次运行日志直接判断“是否发生遗忘式循环”。
- 测试:
  - 单测 + 日志快照测试。

### AISRS-CTX-008: 端到端回归基线（以“循环率”作为验收）

- 优先级: P1
- 工作量: M
- 状态: todo
- Deps: `AISRS-CTX-002`、`AISRS-CTX-003`、`AISRS-CTX-004`、`AISRS-CTX-005`、`AISRS-CTX-007`（硬依赖）
- 涉及:
  - `rust/ais-rs/fixtures/runner-local/*`
  - `rust/ais-rs/crates/ais-runner/src/agent/mod_test.rs`
- 交付:
  - 建立基线场景: `native+erc20 balance gate + dual transfer`。
  - 定义验收阈值:
    - 总工具调用 <= 目标上限
    - discovery 类调用占比 <= 目标上限
    - propose/revise 轮次 <= 目标上限
- 验收:
  - 相对当前日志，重复 discovery 次数下降 40%+（目标值，按基线修订）。
- 测试:
  - e2e fixture + checkpoint resume 回归。

---

## 4. 实施顺序（推荐）

1. `AISRS-CTX-001`  
2. `AISRS-CTX-002`  
3. `AISRS-CTX-003`  
4. `AISRS-CTX-004`  
5. `AISRS-CTX-005`  
6. `AISRS-CTX-006`  
7. `AISRS-CTX-007`  
8. `AISRS-CTX-008`

---

## 5. 依赖项规划（执行视图）

### 5.1 关键路径（Critical Path）

`AISRS-CTX-001 -> AISRS-CTX-003 -> AISRS-CTX-004 -> AISRS-CTX-006 -> AISRS-CTX-007 -> AISRS-CTX-008`

说明:
- 这条链决定“是否能稳定减少循环并可量化验证”。
- `AISRS-CTX-005` 不在关键路径上，但完成后可显著降低 query 换词空转。

### 5.2 并行批次（Wave）

- Wave 1（先行）:
  - `AISRS-CTX-001`
- Wave 2（可并行）:
  - `AISRS-CTX-002`（依赖 001）
  - `AISRS-CTX-003`（依赖 001）
- Wave 3（可并行）:
  - `AISRS-CTX-004`（依赖 003）
  - `AISRS-CTX-005`（建议在 003 之后）
- Wave 4:
  - `AISRS-CTX-006`（依赖 004+005）
  - `AISRS-CTX-007`（依赖 002+003）
- Wave 5（收口验收）:
  - `AISRS-CTX-008`（依赖 002+003+004+005+007）

### 5.3 阻塞关系摘要

- 若 `AISRS-CTX-001` 未完成:
  - 后续所有“累积上下文有效性”工作价值会被削弱。
- 若 `AISRS-CTX-003` 未完成:
  - `AISRS-CTX-004/005/006` 的 memory 复用收益难以体现。
- 若 `AISRS-CTX-007` 未完成:
  - `AISRS-CTX-008` 无法形成可比指标闭环。

---

## 6. 备注

- 这份 TODO 明确保留 `list_candidates`，只做“优先级与复用策略”收敛，不做禁用。
- 本方案与现有 `tool_memory_projection` 演进方向兼容，但会显著提升容量与生命周期语义。
