# Prompt: 用 Rust 重写 AIS TS-SDK 与 Runner（重设计 + 优雅实现）

你是一个资深 Rust 工程师与架构师。请在一个全新的 Rust workspace 中从零实现 AIS 的 **SDK（schema/解析/校验/规划/引擎）** 与 **Runner CLI（执行/事件/命令/trace/checkpoint/dry-run）**。你不需要兼容旧代码或旧 CLI 参数，但必须保持功能覆盖与更优雅的结构。

## 0) 背景与目标

AIS（Agent Interaction Specification）是 agent 与“可执行组件”之间的契约体系。核心思路是：

- 工作空间包含：Protocol Spec、Pack、Workflow（可选）、ExecutionPlan（主执行契约）
- Agent 主要输出：ExecutionPlan（或 PlanSkeleton -> compile -> ExecutionPlan）与 patches/confirm commands
- Runner 执行 plan：readiness -> solver -> executor -> policy gate -> events -> (pause -> commands -> continue)
- 所有交互面向 agent：结构化错误、稳定排序、可 hash、JSONL 事件协议、可回放 trace/checkpoint

请在 Rust 实现中做到：**优雅、简洁、人类可读、模块化、解耦**。单文件过长要拆，单函数过长要拆，多用 trait/类型边界表达协议。保持 API 清晰、测试完备。

## 1) 产出物（必须交付）

1. 一个 Rust workspace（`Cargo.toml` workspace）至少包含这些 crates：
- `ais-core`：schema 类型 + 通用工具（json codec、stable hashing、redaction、field-path）
- `ais-schema`：JSON Schema/版本常量（可选：生成器/嵌入）
- `ais-sdk`：加载/解析/校验/规划/编译执行预览/pack 能力边界等纯逻辑
- `ais-engine`：执行引擎（plan-first），事件、命令、patch、solver/executor trait
- `ais-runner`：CLI（二进制 crate）：run plan/workflow/action/query、dry-run text/json、events jsonl、commands stdin jsonl、replay、plan diff
- `ais-fixtures`（可选）：测试 fixtures 管理

2. 完整测试：
- 单元测试覆盖关键模块（解析、校验、planner、policy gate、patch guard、redaction、jsonl codec）
- 集成测试覆盖 runner 的关键回归（dry-run json、events jsonl、commands stdin、plan diff、replay）

3. 文档（Markdown）：
- `docs/architecture.md`：Rust 版模块与交互架构（面向 reviewer）
- `docs/protocols.md`：事件/命令/plan 的 Rust 类型与版本管理策略
- `docs/cli.md`：runner 命令示例与输出格式（含 JSON 输出）

## 2) 功能范围（必须覆盖）

按“能力”组织模块，而不是照搬 TS 文件结构。你需要覆盖这些能力（与之前实现等价或更好）：

### 2.1 Schema 与解析

- 支持 AIS 文档类型：Protocol、Pack、Workflow、Plan、Catalog、PlanSkeleton（如存在）
- 解析：YAML/JSON -> typed struct
- 版本字段：显式 schema 版本字符串（如 `ais-plan/0.0.3`），严格校验
- 严格字段策略：默认拒绝未知字段（除 extensions map），可选宽松模式（但默认严格）

### 2.2 Structured Issues（统一错误）

统一结构：

```json
{ "kind": "...", "severity": "error|warning|info", "node_id": "...?", "field_path": "...", "message": "...", "reference": "...?", "related": {...}? }
```

- 解析错误、schema 校验错误、workspace validator、workflow validator、planner 错误、policy gate 错误都要能转换成这个形状
- `field_path` 统一表示（root、数组索引、对象字段）

### 2.3 Catalog & Candidates（检索与可执行候选集）

- 从 workspace 构建 `Catalog`（稳定排序、可 hash、包含 cards）
- `CatalogIndex`：快速索引（by_ref/by_protocol_version 等）
- `filterByPack(index, pack)`：pack includes + chain_scope 收敛
- `filterByEngineCapabilities(index, capabilities)`：execution_types/detect_kinds/capabilities 收敛
- `getExecutableCandidates(...)`：一次性输出候选 actions/queries/providers/plugins（稳定排序、可 hash）

### 2.4 Planner / Plan

- Workflow -> ExecutionPlan（plan-first）
- readiness 检查：missing refs、needs detect、errors
- solver：最小默认 solver（自动填 contracts；缺 inputs 触发 need_user_confirm）
- dry-run：text + json 输出（json 用于 agent）

### 2.5 Engine（事件驱动、可暂停、命令驱动继续）

- 事件协议（JSONL envelope，seq/run_id/ts）
- 命令协议（JSONL stdin）：apply_patches/user_confirm/select_provider/cancel，幂等（command id 去重）
- patch guard：允许写 roots 的白名单；支持 allow_path_patterns/allow_nodes_paths
- checkpoint：可恢复，包含已完成 node ids、paused state、poll state、(可选) events
- trace：jsonl，可 redact
- 需要确认点：need_user_confirm 事件必须携带解释性摘要（title/summary/hash）

### 2.6 Policy Gate / Pack Allowlist

- 执行期强制 pack allowlist（detect provider / plugin execution）
- policy gate：输入结构、规则执行、输出 `ok|need_user_confirm|hard_block`
- `need_user_confirm.details` 中要带 “confirmation_summary + confirmation_hash”（稳定可对账）

### 2.7 Runner CLI（ais-runner）

提供命令：

- `run plan --file --workspace ...`（dry-run/execute）
- `run workflow --file --workspace ...`（可选）
- `--events-jsonl path|-` 输出事件 JSONL
- `--commands-stdin-jsonl` 从 stdin 读命令
- `--trace path` + `--trace-redact default|audit|off`
- `plan diff --a --b --format text|json`
- `replay --checkpoint <file> [--until-node]` 或 `replay --trace <jsonl> [--until-node]`

所有 CLI 输出必须：

- human text 默认可读
- `--format json` 或相应开关可输出机器可读 JSON
- 错误时 exit code 非 0，并输出结构化 issues（或结构化 error payload）

## 3) Rust 实现约束（强制）

### 3.1 代码风格与结构

- 每个 crate 按职责拆模块：`mod.rs` + 子模块文件，不要把所有内容塞一个文件
- 单函数 > 60 行必须拆分
- 避免 “god structs”，用小 struct + trait 组合
- 可测试性优先：纯函数与 I/O 分层
- 不要过度泛型；但 trait 边界要清晰（Executor/Solver/Codec/Redactor/Hasher）

### 3.2 依赖建议（可调整，但必须说明原因）

- `serde`, `serde_json`, `serde_yaml`
- `thiserror`, `anyhow`（边界层）
- `clap`（runner CLI）
- `regex`
- `sha2` 或 `blake3`（hash；若用 blake3 需说明）
- `tokio`（如需要 async 执行器；否则用 sync 也行，但要一致）
- `insta`（可选：快照测试）
- 不允许引入重量级框架导致工程复杂化

### 3.3 Redaction（隐私与最小泄露）

- 提供 redact 模式：`default|audit|off`
- 默认 mode 必须最保守：不输出私钥、seed、签名材料、完整 rpc payload 等
- redaction 不能破坏二进制类型 roundtrip（如 bytes）
- 支持 allowlist path patterns 以便审计场景（慎用）

### 3.4 稳定性与 hash

- 所有需要 hash 的结构必须：
  - stable order（排序规则明确）
  - hash 计算忽略 `created_at/ts` 等非决定字段
- 提供统一的 `stable_json` 与 `stable_hash` 工具

## 4) 交互与输出要求（你需要怎么做）

请按以下步骤输出你的工作：

1. 给出 workspace 目录结构（文件树），说明每个 crate 的职责
2. 列出核心类型（Rust structs/enums）与版本字符串常量
3. 设计事件/命令/issue 的 Rust 类型与 JSON 编解码策略（含 schema version）
4. 实现代码（分文件），并写测试（分文件）
5. 提供 `cargo test` 可通过的说明
6. 提供 runner CLI 示例（命令行与输出示例）

注意：不要输出任何“旧 TS 代码引用”。这是全新 Rust 实现。也不要做历史兼容层。

## 5) 验收标准（必须满足）

- `cargo test` 全绿
- runner CLI 可运行：
  - `plan diff` / `replay` 有测试覆盖
  - `run plan` dry-run json 输出包含 structured issues + per-node report
  - `need_user_confirm` details 包含 `confirmation_summary` 与 `confirmation_hash`
  - `--trace-redact default` 会强 redaction，`audit` 保留更多结构但仍 redaction secrets
- 所有模块保持清晰边界，代码可读、可 review

---

如果你需要我提供额外信息（例如现有 spec 文件具体字段、某些 schema 的精确结构），请用最少问题数询问；否则直接开始设计与实现。

