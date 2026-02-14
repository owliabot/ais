# AIS Rust Protocols（事件 / 命令 / Plan 类型与版本策略）

本文对应 `AISRS-DOC-002`，定义 `ais-rs` 当前协议边界、Rust 类型映射与版本演进规则。

## 1. 当前协议 ID（canonical）

统一常量位置：`rust/ais-rs/crates/ais-schema/src/versions.rs:1`

| 协议 | schema id | Rust 类型 |
|---|---|---|
| Protocol 文档 | `ais/0.0.2` | `ais_sdk::ProtocolDocument` |
| Pack 文档 | `ais-pack/0.0.2` | `ais_sdk::PackDocument` |
| Workflow 文档 | `ais-flow/0.0.3` | `ais_sdk::WorkflowDocument` |
| Plan 文档 | `ais-plan/0.0.3` | `ais_sdk::PlanDocument` |
| Catalog 文档 | `ais-catalog/0.0.1` | `ais_sdk::CatalogDocument` |
| Plan Skeleton | `ais-plan-skeleton/0.0.1` | `ais_sdk::PlanSkeletonDocument` |
| Engine 事件流 | `ais-engine-event/0.0.3` | `ais_engine::EngineEventRecord` |
| JSON profile | `ais-json/1` | `ais_core`（后续 codec 对齐） |

> 说明：`engine-command/checkpoint` 目前由 `ais-engine` 内部常量定义，见第 3 节。

## 2. 事件 / 命令 / Plan Rust 类型边界

### 2.1 Plan

- 类型：`rust/ais-rs/crates/ais-sdk/src/documents/plan.rs:6`
- 结构：`schema` + `meta?` + `nodes[]` + `extensions{}`
- 约束：`#[serde(deny_unknown_fields)]`，不接受未知字段。

### 2.2 Engine Event

- 类型：`rust/ais-rs/crates/ais-engine/src/events/types.rs:56`
- record 结构：`schema/run_id/seq/ts/event`
- event 结构：`type/node_id?/data/extensions`
- 序列约束：`seq` 必须从 `0` 单调递增（`ensure_monotonic_sequence`）。
- JSONL：`encode_event_jsonl_line` / `parse_event_jsonl_line`（一行一个 `EngineEventRecord`）。

### 2.3 Engine Command

- 类型：`rust/ais-rs/crates/ais-engine/src/commands/types.rs:29`
- envelope 结构：`schema + command`
- command 结构：`id/type/data`
- 支持命令：`apply_patches` / `user_confirm` / `select_provider` / `cancel`
- JSONL：`encode_command_jsonl_line` / `decode_command_jsonl_line`。

### 2.4 Checkpoint

- 类型：`rust/ais-rs/crates/ais-engine/src/checkpoint/types.rs:27`
- 结构：`schema/run_id/plan_hash/engine_state/runtime_snapshot?`
- 用途：`run plan` 断点恢复 + replay from checkpoint。

### 2.5 Workflow Outputs（runner 导出协议）

- CLI 入口：`ais-runner run workflow --outputs <file>`
- 文件 schema：`ais-runner-workflow-outputs/0.0.1`
- 文件结构：
  - `schema: "ais-runner-workflow-outputs/0.0.1"`
  - `outputs: { ... }`（由 workflow 顶层 `outputs` 的 ValueRef/CEL 在最终 runtime 上求值）
- 说明：此导出是 runner 层协议，不属于 engine event/checkpoint schema。

## 3. 版本策略（当前执行规则）

### 3.1 Schema 选择与分发

- 文档入口统一走 `schema` discriminator（SDK parse 分发）。
- `schema` 不匹配时直接报错，不做隐式降级。
- workspace 级校验要求：`workflow/protocol/pack` 的 schema 与引用链闭环一致。

### 3.2 兼容性级别

- **同 schema id**：必须保持向后兼容（新增字段仅允许可选字段）。
- **schema id 变更**：视为协议升级，走显式迁移（fixtures + conformance + changelog）。
- **运行时输入（JSONL）**：严格按 envelope schema 解析，未知字段默认拒绝（依赖 `deny_unknown_fields`）。

### 3.3 推荐升级流程

1. 在 `ais-schema` 增加新常量与 schema registry 条目。  
2. 在 `ais-sdk` 更新 typed struct + parse/validate。  
3. 在 `ais-engine` 更新 event/command/checkpoint 常量与编码解码。  
4. 在 `ais-runner` 更新 CLI 输出契约与 fixture。  
5. 增加跨 crate 集成测试（parse → compile → run/replay）。

## 4. 当前实现取舍与后续收敛

- `ais-engine-command/0.0.1` 与 `ais-checkpoint/0.0.1` 常量暂未集中到 `ais-schema`。
- `ais-json/1 codec`（`AISRS-CORE-005`）完成后，event/command/trace/checkpoint 的 JSON 兼容策略需要统一复核一次。
- DOC-003 将补充 CLI text/json/jsonl 输出示例与字段稳定性约束。
