# AIS Rust 重写方案：`ais-*` SDK + `ais-runner`（workspace from scratch）

本文是对 `docs/PROMPT-rust-rewrite-ais-sdk-runner.md` 的落地方案文档：在一个全新的 Rust workspace 中，从零实现 AIS 的 **SDK（schema/解析/校验/规划/编译）** 与 **Runner CLI（执行/事件/命令/trace/checkpoint/dry-run）**。不兼容旧 CLI 参数，但覆盖同等能力，并在结构上更模块化、更可审计、更易测试。

---

## 1. 目标与非目标

### 1.1 目标

- **严格解析与校验**：YAML/JSON -> typed structs；默认拒绝未知字段（仅允许 `extensions`）；schema 版本字符串严格匹配。
- **统一结构化问题（Issues）**：解析/校验/规划/执行/策略都能输出同一形状的 issues（稳定可 hash）。
- **Plan-first 引擎**：以 `ExecutionPlan` 为主执行契约；readiness -> solver -> executor -> policy gate -> events -> pause/commands/continue。
- **事件/命令 JSONL 契约**：事件可回放；命令幂等（command id 去重）；trace/checkpoint 可恢复。
- **稳定性**：稳定排序、稳定 JSON、稳定 hash；忽略 `created_at/ts` 等非决定字段。
- **默认安全**：trace 默认强 redaction；RuntimePatch 默认禁止写 `nodes.*`；policy gate 默认保守（信息不足 -> 需要确认或阻断）。
- **测试完备**：关键模块单测 + runner 集成测试（dry-run json、events jsonl、commands stdin、plan diff、replay）。
- **可执行闭环**：提供真实的 EVM/Solana executors（EVM 使用 `alloy` 对接 JSON-RPC + 签名/发交易；Solana 使用官方 Rust SDK 对接 RPC + 交易签名/发送/确认）。

### 1.2 非目标（第一版刻意不做）

- 不做旧 TS SDK 的兼容层/迁移层；不承诺旧文件后缀/旧 CLI 参数兼容。
- 不在 core crates 中绑定具体链钱包/网络 SDK（网络 IO 放在可插拔 executor crates；engine 仅依赖 trait；测试优先用 mock executor）。
- 不引入重量级框架（保持依赖轻、可审计、易编译）。

---

## 2. Workspace 结构（crate 划分与职责）

建议 workspace 名称：`ais-rs`（或放在本仓库的 `rust/` 子目录中）。

文件树（示意）：

```text
ais-rs/
  Cargo.toml
  crates/
    ais-core/
      Cargo.toml
      src/
        lib.rs
        field_path.rs
        issues.rs
        stable_json.rs
        stable_hash.rs
        json_codec/
          mod.rs
          types.rs
          serde_impl.rs
        redaction/
          mod.rs
          rules.rs
        runtime_patch/
          mod.rs
          guard.rs
          apply.rs
    ais-schema/
      Cargo.toml
      src/
        lib.rs
        versions.rs
        embedded.rs
        registry.rs
    ais-sdk/
      Cargo.toml
      src/
        lib.rs
        documents/
          mod.rs
          protocol.rs
          pack.rs
          workflow.rs
          plan.rs
          plan_skeleton.rs
          catalog.rs
        parse/
          mod.rs
          yaml.rs
          json.rs
          detect_duplicate_keys.rs
        validate/
          mod.rs
          semantic.rs
          workspace.rs
          workflow.rs
        catalog/
          mod.rs
          build.rs
          index.rs
          filter.rs
        planner/
          mod.rs
          compile_plan_skeleton.rs
          compile_workflow.rs
          readiness.rs
          preview.rs
        capabilities/
          mod.rs
          engine_caps.rs
        resolver/
          mod.rs
          context.rs
          value_ref.rs
          reference.rs
          detect.rs
    ais-cel/
      Cargo.toml
      src/
        lib.rs
        lexer.rs
        parser.rs
        ast.rs
        numeric.rs
        evaluator.rs
    ais-engine/
      Cargo.toml
      src/
        lib.rs
        engine/
          mod.rs
          runner.rs
          scheduler.rs
          state.rs
        events/
          mod.rs
          envelope.rs
          redact.rs
        commands/
          mod.rs
          envelope.rs
          dedupe.rs
        solver/
          mod.rs
          default_solver.rs
        executor/
          mod.rs
          mock.rs
          router.rs
        policy/
          mod.rs
          schema.rs
          extract.rs
          enforce.rs
          confirm_hash.rs
        checkpoint/
          mod.rs
          format.rs
          store.rs
        trace/
          mod.rs
          jsonl.rs
          redact.rs
        plan_diff/
          mod.rs
          diff.rs
    ais-exec-evm-alloy/
      Cargo.toml
      src/
        lib.rs
        executor.rs
        signer.rs
        provider.rs
        redact.rs
        types.rs
    ais-exec-solana/
      Cargo.toml
      src/
        lib.rs
        executor.rs
        signer.rs
        redact.rs
        types.rs
    ais-runner/
      Cargo.toml
      src/
        main.rs
        cli.rs
        config.rs
        io/
          mod.rs
          read_document.rs
          write_jsonl.rs
        commands/
          mod.rs
          run_plan.rs
          run_workflow.rs
          plan_diff.rs
          replay.rs
  fixtures/ (可选)
    plans/
    workflows/
    expected/
```

crate 责任（要点）：

- `ais-core`：跨 crate 的“协议底座”
  - `FieldPath`、`StructuredIssue`
  - `stable_json` + `stable_hash`
  - `ais-json/1` codec（BigInt/bytes/error 的唯一表示）
  - redaction（`default|audit|off`）
  - `RuntimePatch` + patch guard + apply + 审计摘要

- `ais-schema`：schema 版本与 JSON Schema 内嵌
  - 版本字符串常量（例如 `ais-plan/0.0.3`）
  - 内嵌 `schemas/` 下的 JSON Schema 文本（`include_str!`）
  - schema registry（按 schema id 返回对应 JSON Schema）

- `ais-sdk`：纯逻辑 SDK（尽量无 IO）
  - 文档 typed structs（Protocol/Pack/Workflow/Plan/Catalog/PlanSkeleton）
  - 解析（YAML/JSON，含 YAML 重复 key 拒绝）
  - 单文档校验 + 跨文档 workspace 校验
  - Catalog 构建与过滤（stable sort + hash）
  - Planner：Workflow/PlanSkeleton -> ExecutionPlan，readiness、dry-run preview

- `ais-engine`：执行引擎（plan-first）
  - solver/executor traits + 默认实现（最小 solver、mock executor）
  - 事件协议（JSONL envelope），命令协议（stdin JSONL）
  - policy gate（输入提取 + 决策 + confirmation_summary/hash）
  - checkpoint/trace（可 redact，可恢复）
  - plan diff / replay（runner 复用）

- `ais-cel`：CEL 表达式（lexer/parser/evaluator + 数值模型）
  - 支持 calculated_fields / condition / until / assert 等表达式求值
  - 数值模型：执行关键路径以 `bigint`/精确 decimal 为主，拒绝不安全的浮点路径

- `ais-exec-evm-alloy`：EVM executor（真实对接）
  - 使用 `alloy` 作为 EVM 栈：RPC provider、ABI 编解码、交易构造与签名、发交易与 receipt 轮询
  - 与 `ais-engine` 通过 `Executor` trait 解耦

- `ais-exec-solana`：Solana executor（真实对接）
  - 使用 Solana 官方 Rust crates：RPC client、交易构造/签名/发送/确认
  - 与 `ais-engine` 通过 `Executor` trait 解耦

- `ais-runner`：CLI（二进制）
  - `run plan` / `run workflow`
  - `--events-jsonl` 输出事件 JSONL
  - `--commands-stdin-jsonl` 读取命令 JSONL 并驱动继续
  - `--trace` + `--trace-redact default|audit|off`
  - `plan diff`、`replay`
  - 统一错误输出：人类可读 + 可选 JSON payload（issues）

---

## 3. 版本与文档类型（schema id 常量）

Rust 侧以 `schema: "<id>"` 作为 discriminator，并在解析阶段严格校验。

建议在 `ais-schema/src/versions.rs` 固定这些常量（第一版按仓库现有 spec/文档）：

- Protocol：`ais/0.0.2`
- Pack：`ais-pack/0.0.2`
- Workflow：`ais-flow/0.0.3`（如需对齐为 `0.0.2`，在 `ais-schema` 中集中改）
- ExecutionPlan：`ais-plan/0.0.3`
- Catalog：`ais-catalog/0.0.1`
- PlanSkeleton：`ais-plan-skeleton/0.0.1`
- EngineEvent JSONL：`ais-engine-event/0.0.3`
- JSON codec profile：`ais-json/1`

> 约束：任何“版本漂移”必须通过 `ais-schema` 的集中改动完成，并配套 conformance/fixtures 更新；避免散落在各 crate。

---

## 4. 核心类型设计（跨 crate 统一）

### 4.1 `FieldPath`（issues/path/audit 的唯一表示）

- 内部表示：`Vec<FieldPathSegment>`（`Key(String)` / `Index(usize)`）
- 字符串格式（建议）：JSONPath-like，便于跨语言一致
  - 根：`$`
  - 对象字段：`$.nodes[0].args.amount`
  - 数组索引：`$.includes[2]`
- 提供：`FieldPath::push_key/push_index`、`Display`、`FromStr`（可选）。

### 4.2 `StructuredIssue`

统一形状：

```json
{
  "kind": "parse_error|schema_error|validation_error|planner_error|engine_error|policy_gate|patch_guard|...",
  "severity": "error|warning|info",
  "node_id": "optional string",
  "field_path": "$.nodes[0].args.amount",
  "message": "human readable",
  "reference": "optional stable code/url",
  "related": { "any": "json" }
}
```

要点：

- **稳定**：同一输入产生的 issues 列表必须稳定排序（按 `severity`、`kind`、`field_path`、`message`、`node_id`）。
- **可定位**：解析/校验时尽量产出精确 `field_path`；serde 错误需要映射到 `FieldPath`（做不到时至少 `"$"`）。
- **可 hash**：issues 可以被 stable_json/hash，用于回放对账与快照测试。

### 4.3 `RuntimePatch` + `RuntimePatchGuardPolicy`

Patch 结构（与 docs/ais-runtime-patch-guard.md 一致）：

```json
{ "op": "set|merge", "path": "inputs.amount", "value": 123, "extensions": {} }
```

Guard 默认策略：

- 默认允许 roots：`inputs.*`、`ctx.*`、`contracts.*`、`policy.*`
- 默认禁止：`nodes.*`
- 可配置：`allow_roots[]`、`allow_path_patterns[]`、`allow_nodes_paths[]`

`apply_runtime_patches(...)` 返回审计摘要：

- `patch_count/applied_count/rejected_count/affected_paths/partial_success/hash`

---

## 5. `ais-json/1` 编解码（JSON/JSONL 互操作）

目的：在 events/commands/trace/checkpoint 中稳定传输 “大整数 / bytes / 错误”。

### 5.1 唯一表示（wire format）

- BigInt：
  - `{"__ais_json_type":"bigint","value":"123"}`
- Bytes：
  - `{"__ais_json_type":"uint8array","encoding":"base64","value":"..."}`
- Error：
  - `{"__ais_json_type":"error","name":"...","message":"...","stack":"...?"}`

### 5.2 Rust API（建议）

- `ais_core::json_codec::AisJsonCodec`
  - `to_value<T: SerializeAisJson>(t) -> serde_json::Value`
  - `from_value<T: DeserializeAisJson>(v) -> T`
  - `stringify(value, pretty, strict_opts) -> String`
  - `parse(str) -> serde_json::Value`
- strict opts：
  - `reject_undefined`（Rust 侧通常无 undefined；但用于跨语言输入防御）
  - `reject_non_finite_number`（默认拒绝 NaN/Infinity）

> 注：plan/workflow 的“规范文档”不强制使用该 wrapper；它主要用于运行期日志/事件/命令边界，避免跨语言歧义。

---

## 6. 稳定 JSON 与稳定 hash（审计/缓存/对账）

### 6.1 `stable_json`

规则（建议）：

- 对象 key：按字典序排序（Rust 用 `BTreeMap` + 自定义 serializer 确保）
- 数组：保留原有顺序；但所有“集合语义”的数组在构造阶段就必须排序（Catalog/actions/queries/includes/nodes 等）
- 规范化时忽略字段：`created_at`、`ts`、`run_id`、`seq`（由调用方按场景配置 ignore 列表）

### 6.2 `stable_hash`

- 算法：`sha2::Sha256`（满足“可审计/跨语言一致”的最低要求）
- 输入：`stable_json_bytes`
- 输出：小写 hex 字符串

用途：

- Catalog hash
- confirmation_hash（need_user_confirm 对账）
- patch audit hash
- plan hash（忽略非决定字段）

---

## 7. 解析与校验（`ais-sdk`）

### 7.1 解析入口

`parse_document(bytes, format_hint, strictness) -> Result<AisDocument, Issues>`

- 支持 JSON/YAML
- YAML 必须 **拒绝重复 key**（安全要求，避免静默覆盖）
  - 实现：先用 YAML AST（保留 mapping entries）扫描重复 key，再交给 `serde_yaml` 反序列化（或直接 AST -> serde_json::Value -> typed）
- discriminator：读取顶层 `schema`，选择对应 typed struct
- 默认 `#[serde(deny_unknown_fields)]`，扩展槽固定为 `extensions: BTreeMap<String, Value>`

### 7.2 校验分层

- 单文档校验：
  - schema id 是否正确
  - 必填字段、引用格式、基础类型（CAIP-2、地址格式、ref 格式等）
- workspace 校验：
  - workflow.requires_pack -> pack.includes -> protocol@version 必须闭环命中
  - chain_scope 收敛：workflow node 的 chain 必须在 pack 允许范围
  - allowlist 收敛：detect/provider/plugin/execution_type 必须在 pack + engine capabilities 内
- workflow 校验：
  - DAG 无环、deps 指向存在节点
  - ValueRef 引用可解析（存在 ref root；字段路径合法）

输出：全部转换为 `StructuredIssue`。

---

## 7.3 Resolver / ValueRef / Detect / CEL（SDK 语义核心）

Rust 版必须补齐 TS SDK 的“语义核心链路”，否则 planner/engine/policy gate 的很多能力无法闭环：

- `ResolverContext`：包含
  - `runtime`（`inputs/params/ctx/contracts/nodes/...`）
  - `protocol registry`（按 `protocol@version`）
  - `source metadata`（可选：用于 issues/reference）
- `ValueRef` 求值（sync + async 版本）：
  - `{ lit | ref | cel | detect | object | array }`（结构化，禁止字符串多义）
  - `ref`：通过 `FieldPath` 在 `runtime` 中读取（缺失 -> readiness.missing_refs）
  - `cel`：调用 `ais-cel` evaluator（错误 -> issues）
  - `detect`：调用 detect provider（必须受 pack allowlist + engine capabilities 约束；必要时走 `select_provider` 命令）
- 数值模型：
  - 执行关键路径以 `bigint` 与精确 decimal 表示；避免 `f64` 参与金额/滑点等敏感计算
  - 提供 `to_atomic/to_human/mul_div` 等内建函数（与 TS CEL 模块对应）

---

## 8. Catalog 与 Candidates（检索与候选集）

### 8.1 `Catalog` 生成

输入：workspace（protocols/packs/workflows 可选）  
输出：`ais-catalog/0.0.1` 文档：

- `actions: Vec<ActionCard>`
- `queries: Vec<QueryCard>`
- `packs: Vec<PackCard>`
- `documents`（可选：来源 hash）
- `hash`（stable_hash，忽略 `created_at`）

稳定性：

- cards 必须稳定排序（`protocol`, `version`, `id` 字典序）
- pack includes 必须稳定排序

### 8.2 `CatalogIndex` 与过滤

- `CatalogIndex`：按 `ref`、`protocol@version`、`execution_type`、`chain` 建索引
- `filter_by_pack(index, pack)`：includes + chain_scope 收敛
- `filter_by_engine_capabilities(index, caps)`：execution_types/detect_kinds/providers/plugins 收敛
- `get_executable_candidates(...)`：一次性输出候选 actions/queries/providers/plugins（稳定排序，可 hash）

---

## 9. Planner：Workflow/PlanSkeleton -> ExecutionPlan

### 9.1 ExecutionPlan（主执行契约）

核心要求：

- DAG 稳定拓扑序
- 每个节点明确：
  - `node_id`
  - `chain`
  - `source`（action_ref/query_ref/step）
  - `execution`（execution_type + 编译所需 spec）
  - `deps`
  - `writes`（运行时写回路径）
  - `until/retry/timeout_ms/assert`（控制语义）
- 可序列化（JSON），用于 checkpoint/trace/replay

### 9.2 readiness（阻塞原因显式化）

`get_node_readiness(node, runtime) -> Ready | Blocked { missing_refs, needs_detect, errors }`

- `missing_refs`：缺少 `inputs.* / ctx.* / contracts.* / nodes.*` 等引用
- `needs_detect`：存在 detect 且未决策
- `errors`：表达式/类型/编译错误（可转 issues）

### 9.3 最小默认 solver 行为

默认 solver 只做“安全且可解释”的补全：

- 自动填 `contracts.*`（若 protocol deployment 在当前 chain 下唯一可确定）
- 对 `inputs.*` 缺失：
  - 产出 `need_user_confirm`（给出缺失字段列表与建议）
- 对 `needs_detect`：
  - 若候选集在 allowlist 内且唯一，则自动选择
  - 否则发出 `need_user_confirm` 或 `select_provider` 请求

### 9.4 dry-run（text + json）

- `dry_run_text`：面向人类（节点列表、链、执行类型、写路径、风险摘要、预计确认点）
- `dry_run_json`：面向 agent（per-node report + issues + hashes）

---

## 10. Engine：事件驱动执行 + 可暂停 + 命令驱动继续

### 10.1 事件 JSONL envelope（`ais-engine-event/0.0.3`）

每行：

```json
{
  "schema": "ais-engine-event/0.0.3",
  "run_id": "uuid",
  "seq": 1,
  "ts": "2026-02-12T00:00:00Z",
  "event": { "type": "plan_ready", "node_id": "optional", "data": {}, "extensions": {} }
}
```

最小事件集合（第一版必须覆盖）：

- `plan_ready`
- `node_ready`
- `node_blocked`
- `need_user_confirm`
- `query_result`
- `tx_prepared`
- `tx_sent`
- `tx_confirmed`
- `node_waiting`
- `checkpoint_saved`
- `engine_paused`
- `error`

可扩展：

- `solver_applied`
- `skipped`
- `command_accepted` / `command_rejected`
- `patch_applied` / `patch_rejected`

### 10.2 命令 JSONL（stdin）

命令 envelope（建议在 Rust 版固定一套 schema id，例如 `ais-engine-command/0.0.1`）：

```json
{
  "schema": "ais-engine-command/0.0.1",
  "command": { "id": "uuid", "type": "apply_patches", "data": {} }
}
```

支持命令：

- `apply_patches`：应用 `RuntimePatch[]`（强制启用 patch guard + 审计事件）
- `user_confirm`：针对某个 confirmation_scope/key 的 `approve|deny`（幂等）
- `select_provider`：选择 detect provider 或 execution plugin（必须在 allowlist 内）
- `cancel`：终止执行（可审计）

幂等：

- 引擎维护 `seen_command_ids`（checkpoint 持久化）
- 重复 command id：返回 `command_accepted`（no-op）或 `command_rejected`（按策略固定），但必须稳定

### 10.3 执行模型（核心循环）

1) 读入 plan + runtime 初值  
2) 计算可运行节点集合（deps satisfied）  
3) readiness：
   - ready -> executor
   - blocked -> solver（或直接 `node_blocked` + pause）
4) 写节点执行前：
   - policy gate allowlist + rule evaluation
   - `ok` -> 继续；`need_user_confirm` -> pause；`hard_block` -> error/终止
5) 产生事件、保存 checkpoint、可选写 trace  
6) 如无可进展：`engine_paused` 并等待命令

---

## 11. Policy Gate（执行期强制边界）

### 11.1 输入/输出

- `PolicyGateInput`：执行前风控输入快照（金额/滑点/授权/风险标签/字段来源/missing/unknown）
- `PolicyGateOutput`：`ok | need_user_confirm | hard_block`

缺失语义：

- `missing_fields`：业务上必须（默认触发 `need_user_confirm` 或 `hard_block`，由 pack 配置决定）
- `unknown_fields`：语义不确定（默认 `need_user_confirm`）

### 11.2 confirmation_summary + confirmation_hash（强制）

当输出为 `need_user_confirm` 时，`need_user_confirm.details` 必须包含：

- `confirmation_summary`：稳定、可展示的摘要（标题/关键信息/命中规则/阈值）
- `confirmation_hash`：对 `confirmation_summary` 做 stable_hash（忽略时间戳等）

用途：

- runner/UI/agent 对账：确认的是“同一份风险摘要”，而不是“变化中的 payload”

---

## 12. Trace / Checkpoint / Replay

### 12.1 Trace（JSONL）

- trace 记录“事件与关键数据快照”，用于审计与回放
- trace 必须支持 redaction：
  - `default`：强脱敏（推荐）
  - `audit`：保留更多结构，但仍脱敏 secrets
  - `off`：不脱敏（危险，仅本地调试）

redaction 规则（最低要求）：

- 私钥/seed/助记词/签名材料：一律替换为 `"[REDACTED]"`
- 完整 RPC payload：默认模式下替换或裁剪为摘要（保留 method、chain、status）
- 允许通过 `allow_path_patterns` 例外放行（审计场景）

### 12.2 Checkpoint（可恢复）

checkpoint 文件（JSON）包含：

- `schema`: `ais-checkpoint/0.0.1`（Rust 版自定义，但固定）
- `run_id`
- `plan_hash`
- `engine_state`（完成的 node ids、当前 paused 原因、seen_command_ids、pending retries）
- `runtime_snapshot`（可选：按模式 redact）

要求：

- 反序列化必须能在 redacted payload 上工作（被替换字段不影响结构）

### 12.3 Replay

runner 提供两条 replay 路径：

- `replay --checkpoint <file> [--until-node ...]`：从 checkpoint 恢复并继续/演练
- `replay --trace <jsonl> [--until-node ...]`：顺序回放事件（不重新执行 executor）

---

## 13. Plan diff

目标：比较两个 plan 的结构差异（稳定、可机器读）。

差分粒度（第一版）：

- 节点增删
- 节点关键字段变化：`deps/chain/execution_type/writes/until/retry/timeout/assert/source ref`
- 输出两种格式：
  - `text`：面向人类
  - `json`：面向 agent/CI

实现建议：

- 对每个 node 计算 `node_fingerprint = stable_hash(node_normalized)`
- diff：
  - added/removed：按 node_id
  - changed：fingerprint 不同则列出字段级变化（可选：只列“关注字段”）

---

## 14. CLI（`ais-runner`）

### 14.1 命令集

- `ais-runner run plan --file <plan.(json|yaml)> --workspace <dir>`
  - `--dry-run`（默认可提供 `--dry-run-format text|json`）
  - `--events-jsonl <path|->`
  - `--commands-stdin-jsonl`
  - `--trace <path>`
  - `--trace-redact default|audit|off`
- `ais-runner run workflow --file <workflow.(json|yaml)> --workspace <dir>`（可选；内部会 compile -> plan）
- `ais-runner plan diff --a <file> --b <file> --format text|json`
- `ais-runner replay --checkpoint <file> [--until-node <id>]`
- `ais-runner replay --trace <file.jsonl> [--until-node <id>]`

补充（为了真实 executor 可用性）：

- runner 必须提供 **按 chain 精确路由 executor** 的机制，避免 “把 `eip155:1` 的请求发到 `eip155:8453` 的 RPC”：
  - 推荐在 `ais-engine` 提供 `RouterExecutor`（`HashMap<chain, Box<dyn Executor>>`），runner 只负责装配
  - runner 配置（`ais-runner/0.0.1`，可独立 schema）：为每条 chain 配置 rpc/signer/并发参数

### 14.2 输出与退出码

- 默认输出：人类可读文本
- `--format json`（或各命令的 `--*-json`）输出机器可读 JSON
- 错误时：
  - exit code != 0
  - stderr 输出摘要
  - stdout（或 stderr）输出结构化 issues（可选：`--errors-format json`）

---

## 15. 测试策略（单测 + 集成）

### 15.1 `ais-core` 单测

- `FieldPath` roundtrip（parse/display）
- stable_json：key 排序与 ignore 字段行为
- stable_hash：固定输入固定输出
- json codec：BigInt/bytes/error roundtrip + strict 拒绝策略
- redaction：`default/audit/off` 的差异与 allowlist patterns
- patch guard：默认拒绝 `nodes.*`，配置放行规则生效；审计 hash 稳定

### 15.2 `ais-sdk` 单测

- YAML 重复 key 拒绝（包含深层 mapping）
- 文档解析：protocol/pack/workflow/plan/catalog/plan_skeleton
- 校验：schema id、unknown fields、workspace 引用闭环、chain_scope/allowlist 收敛
- Catalog：稳定排序与 hash
- Planner：plan_skeleton -> plan、workflow -> plan、readiness 输出稳定

### 15.3 `ais-engine` 单测

- 事件 envelope：seq 单调递增、schema 正确、redaction 生效
- 命令去重：重复 command id 行为稳定
- policy gate：missing/unknown 触发 need_user_confirm；confirmation_hash 稳定
- checkpoint serialize/deserialize（含 redacted payload）
- plan diff：added/removed/changed 的输出稳定

### 15.4 `ais-runner` 集成测试

- `run plan --dry-run --format json`：包含 issues + per-node report
- `--events-jsonl -`：输出合法 JSONL、schema 正确、seq 单调递增
- `--commands-stdin-jsonl`：对 `apply_patches/user_confirm/cancel` 的最小闭环（用 mock executor + 人工构造 blocked/confirm）
- `plan diff`：text/json 两种输出
- `replay --trace` 与 `replay --checkpoint`：直到指定 node 或 EOF

建议使用：

- `insta`：对 JSON 输出做快照（配合 stable_json）
- fixtures：固定的小 plan/workflow/pack/protocol 样例，覆盖 allowlist、policy、patch guard

---

## 16. 交付里程碑（建议拆分）

1) `ais-core`：FieldPath/Issue/stable_json/hash/json_codec/redaction/patch guard
2) `ais-schema`：版本常量 + 内嵌 JSON Schema registry
3) `ais-sdk`：解析（含 YAML dup keys）+ 单文档校验 + workspace 校验 + Catalog
4) `ais-sdk`：Planner（plan_skeleton/workflow -> plan）+ readiness + dry-run
5) `ais-engine`：事件/命令/solver/executor traits + mock executor + checkpoint/trace
6) `ais-runner`：CLI + events/commands IO + plan diff + replay
7) 全量测试与 fixtures + 文档（architecture/protocols/cli）

---

## 17. 需要确认的最少问题（如要一次性定稿）

如果你希望 Rust 版与仓库规范完全对齐，请确认两点（不确认也可先按本文默认值推进）：

1) Workflow schema id 统一为 `ais-flow/0.0.3`。
2) 是否要求第一版就内置真实链 executor（EVM/Solana JSON-RPC），还是允许 runner 默认仅提供 mock executor，真实 executor 作为后续可插拔 crate。
