# Workflow 0.0.3 Conformance Matrix

本文对应 `AISRS-DOC-004`，用于追踪 `specs/ais-1-workflow.md`（`ais-flow/0.0.3`）在 Rust 实现中的语义落地。

## 1. 范围与判定标准

- **Spec 基准**：`specs/ais-1-workflow.md:1`
- **实现范围**：`rust/ais-rs` 的 `ais-sdk`（parse/validate/compile）、`ais-engine`（execute）、`ais-runner`（workflow entry）
- **状态定义**
  - `Implemented`：语义已在代码路径生效，且有测试覆盖
  - `Partial`：仅部分实现（如仅编译透传、运行时未执行）
  - `Planned`：尚未落地

## 2. 字段级语义矩阵（Spec -> Rust -> Tests）

| Spec 项 | 要求摘要 | Rust 实现位置 | 测试位置 | 状态 |
|---|---|---|---|---|
| `schema = ais-flow/0.0.3` | schema 严格匹配 | `rust/ais-rs/crates/ais-sdk/src/validate/semantic.rs:142` | `rust/ais-rs/crates/ais-runner/src/io/read_document_test.rs:33` | Implemented |
| strict + `extensions` | unknown fields 拒绝，扩展字段单独承载 | `rust/ais-rs/crates/ais-sdk/src/documents/workflow.rs:5` |（serde 严格解析链路覆盖）`rust/ais-rs/crates/ais-sdk/src/parse/mod_test.rs:1` | Implemented |
| `imports.protocols[]` | `protocol/path/integrity` 结构与节点协议闭环 | `rust/ais-rs/crates/ais-sdk/src/validate/workflow.rs:172` | `rust/ais-rs/crates/ais-sdk/src/validate/workflow_test.rs:137` | Implemented |
| `requires_pack` + pack includes/chain_scope | workflow 与 pack、chain scope 一致 | `rust/ais-rs/crates/ais-sdk/src/validate/workspace.rs:183` | `rust/ais-rs/crates/ais-sdk/src/validate/workspace_test.rs:61` | Implemented |
| Node 基础模型 | `type/action/query/protocol/chain` 校验 | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:250` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:1` | Implemented |
| Chain resolution | `nodes[].chain` -> `workflow.default_chain`，缺失报错 | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:298` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:124` | Implemented |
| DAG（显式 deps） | 依赖拓扑与稳定顺序 | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:391` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:70` | Implemented |
| DAG（隐式 refs） | 从 ValueRef/CEL 引用提取隐式依赖 | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:391` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:292` | Implemented |
| `assert/assert_message` | 编译校验 + 执行后断言 + 失败策略 | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:374` / `rust/ais-rs/crates/ais-engine/src/engine/runner.rs:479` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:239` / `rust/ais-rs/crates/ais-engine/src/engine/runner_test.rs:260` | Implemented |
| `calculated_overrides` | 拓扑排序、缺依赖/环检测、稳定输出 | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:406` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:292` | Implemented |
| `preflight.simulate` | workflow -> plan meta 透传；engine 跳过 executor | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow.rs:225` / `rust/ais-rs/crates/ais-engine/src/engine/runner.rs:225` | `rust/ais-rs/crates/ais-sdk/src/planner/compile_workflow_test.rs:193` / `rust/ais-rs/crates/ais-engine/src/engine/runner_test.rs:277` | Implemented |
| `policy` | policy gate 决策（ok/confirm/block）进入执行循环 | `rust/ais-rs/crates/ais-engine/src/engine/runner.rs:273` | `rust/ais-rs/crates/ais-engine/src/policy/gate_test.rs:7` | Implemented |
| `condition` | 执行前条件判定，false 跳过 | `rust/ais-rs/crates/ais-engine/src/engine/runner.rs:180` | `rust/ais-rs/crates/ais-engine/src/engine/runner_test.rs:384` | Implemented |
| `until/retry/timeout_ms` | 轮询重试与超时生命周期 | `rust/ais-rs/crates/ais-engine/src/engine/runner.rs:683` | `rust/ais-rs/crates/ais-engine/src/engine/runner_test.rs:558` | Implemented |

## 3. 端到端覆盖（Runner）

- `run workflow` dry-run + workspace 装载：`rust/ais-rs/crates/ais-runner/src/run_test.rs:133`
- `run workflow` execute 路径（需要 config）：`rust/ais-rs/crates/ais-runner/src/run_test.rs:254`
- `run plan` execute + events/trace/checkpoint：`rust/ais-rs/crates/ais-runner/src/run_test.rs:518`

结论：`run workflow (0.0.3)` 主链路（parse/validate/compile/execute）已打通。

## 4. 与 TODO 的对应关系

- 已完成：
  - `AISRS-SDK-033`（imports）
  - `AISRS-SDK-034`（assert）
  - `AISRS-SDK-035`（calculated_overrides）
  - `AISRS-ENG-022`（assert/preflight 执行语义）
  - `AISRS-ENG-023`（condition 执行语义）
  - `AISRS-ENG-024`（until/retry 执行语义）
  - `AISRS-ENG-025`（timeout_ms 执行语义）
  - `AISRS-RUN-022`（run workflow 0.0.3 语义模式）

## 5. 当前差异结论（2026-02-14）

- **已符合**：imports/requires_pack/assert/calculated_overrides/preflight/policy 的核心语义。
- **已符合**：imports/requires_pack/assert/calculated_overrides/preflight/policy/condition/until/retry/timeout 的核心语义。
- **建议下一步**：围绕并发调度与跨链混合场景补充更多 replay + checkpoint 回归用例。
