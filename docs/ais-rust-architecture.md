# AIS Rust Architecture (`ais-rs`)

本文描述 Rust 重写版本的模块边界、调用链路和数据契约，作为 `AISRS-DOC-001` 交付文档。

## 1. Workspace 与 crate 划分

`rust/ais-rs` 当前按“核心能力 + 执行引擎 + 执行器 + CLI”拆分：

- `ais-core`
  - `FieldPath` / `StructuredIssue`
  - `stable_json` / `stable_hash`
  - runtime patch + patch guard + audit
- `ais-schema`
  - schema id 常量
  - 内嵌 JSON Schema registry
  - schema 校验适配
- `ais-cel`
  - CEL lexer/parser/evaluator
  - numeric 模型 + builtins
- `ais-sdk`
  - 文档模型（protocol/pack/workflow/plan/...）
  - parse + validate（含 workspace/workflow 语义）
  - resolver/value-ref
  - planner（compile_workflow、dry-run、readiness）
- `ais-engine`
  - plan-first 执行循环
  - events/commands JSONL
  - policy gate、checkpoint、trace、replay
- `ais-evm-executor`
  - EVM 执行器（alloy + RPC 约束）
  - chain 级 long-lived client 复用（http/ws）
  - `timeout_ms` 作用于 connect/read/call/send/receipt 等 await
- `ais-solana-executor`
  - Solana 执行器（RPC + 交易编码发送）
- `ais-runner`
  - CLI（run plan / run workflow / plan diff / replay）
  - workspace 装载、executor 装配、输出渲染

## 2. 模块交互总览

```text
workflow/protocol/pack files
        │
        ▼
  ais-runner (IO + CLI)
        │ parse/validate/compile
        ▼
      ais-sdk  ─────────────┐
        │ plan              │ schema constants/validation
        ▼                   ▼
     ais-engine  ◀────── ais-schema
        │ execute
        ├── RouterExecutor ──► ais-evm-executor / ais-solana-executor
        ├── issues/hash/patch ─► ais-core
        └── value-ref/cel eval ─► ais-sdk / ais-cel
```

约束：

- `ais-sdk` 尽量保持纯逻辑（少 IO）。
- `ais-engine` 不直接绑定具体链 SDK，通过 executor trait 解耦。
- `ais-runner` 负责外部输入输出、配置、文件系统、命令行协议。

## 3. 关键执行链路

### 3.1 `run workflow`（0.0.3）

1. `ais-runner` 读取 workflow + workspace 文档。
2. `ais-sdk` 进行：
   - schema 校验（`ais-schema`）
   - workspace 引用闭环校验
   - workflow 语义校验（imports/assert/calculated_overrides 等）
3. `ais-sdk::compile_workflow` 输出 `ais-plan/0.0.3`。
4. 根据 CLI：
   - `--dry-run`：走 `dry_run_text/json`
   - 否则：交给 `ais-engine::run_plan_once` 执行
5. engine 产出 event stream / trace / checkpoint（按参数写出）。
6. 若指定 `--outputs`，runner 基于最终 runtime 评估 workflow 顶层 outputs 并单独写 JSON 文件。

### 3.2 `run plan`

1. 直接解析 plan 文档。
2. `--dry-run` 使用 `ais-sdk` readiness/dry-run。
3. execute 模式使用 `ais-engine` + router executor（由 `ais-runner` 按 config 装配）。

## 4. 事件与命令边界

- 事件 schema：`ais-engine-event/0.0.3`
- 命令 schema：`ais-engine-command/0.0.1`
- checkpoint schema：`ais-checkpoint/0.0.1`
- workflow outputs export schema（runner）：`ais-runner-workflow-outputs/0.0.1`

引擎状态机核心结果：

- `completed`
- `paused`
- `stopped`

典型 pause/stop 来源：

- `need_user_confirm`
- policy hard block
- executor error
- assert failed（可配置 pause/stop 策略）

## 5. Workflow 0.0.3 语义落点（当前实现）

- `imports.protocols[]`：在 SDK 校验与 workspace 校验闭环。
- `assert/assert_message`：
  - compile 阶段语义校验与 plan 透传
  - engine 执行后求值，失败触发 error + pause/stop
- `calculated_overrides`：
  - SDK 计算依赖拓扑序
  - 报告 missing dependency / cycle
- `preflight.simulate`：
  - compile 透传到 plan meta
  - engine 按节点 simulate 路径执行（跳过真实 executor）

## 6. 设计原则

- **Plan-first**：执行只依赖 plan，不直接依赖 workflow 文档。
- **Deterministic**：排序、hash、issues 输出稳定。
- **Safe defaults**：
  - patch guard 默认禁止 `nodes.*`
  - trace 默认 redaction
  - policy gate 对信息缺失保守处理
- **Pluggable executors**：链能力扩展不影响 engine 核心循环。

## 7. 后续文档对应

- `docs/protocols.md`（`AISRS-DOC-002`）：补事件/命令/plan 类型与版本策略细节。
- `docs/cli.md`（`AISRS-DOC-003`）：补 runner 命令矩阵、输出样例与 JSONL 约定。
- `docs/workflow-0.0.3-conformance.md`（`AISRS-DOC-004`）：补字段级语义差异矩阵与测试映射。
