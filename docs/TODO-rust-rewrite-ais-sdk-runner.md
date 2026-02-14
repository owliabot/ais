# TODO: AIS Rust 重写（SDK + Engine + Runner）可追踪任务清单

本文是 `docs/rust-rewrite-ais-sdk-runner-plan.md` 的执行 TODO（细粒度、可追踪、可验收）。  
约定：每个任务有唯一 ID（用于 PR/commit/issue 标题），并写清 **依赖**、**交付物**、**验收标准**、**测试**。

---

## 0. 追踪规范

### 0.4 当前推进快照（2026-02-12）

- 已完成：`AISRS-CORE-001..004`、`AISRS-SCHEMA-001..002`
- 阻塞：当前环境无法访问 `index.crates.io`，`cargo test` 依赖下载失败；待网络可用后执行完整测试

### 0.1 ID 规则

- `AISRS-CORE-###`：`ais-core`
- `AISRS-SCHEMA-###`：`ais-schema`
- `AISRS-CEL-###`：`ais-cel`
- `AISRS-SDK-###`：`ais-sdk`
- `AISRS-ENG-###`：`ais-engine`
- `AISRS-EVM-###`：`ais-evm-executor`
- `AISRS-SOL-###`：`ais-solana-executor`
- `AISRS-RUN-###`：`ais-runner`
- `AISRS-DOC-###`：文档
- `AISRS-FIX-###`：fixtures/测试数据

### 0.2 状态字段（在任务标题后手工维护）

- `[ ]` 未开始
- `[~] 进行中`
- `[x] 完成

### 0.3 Definition of Done（统一验收线）

- 代码：模块边界清晰、无超长函数（>60 行拆分）、默认安全（strict parsing + redact + patch guard）。
- 测试：单测/集成测覆盖；输出稳定可快照；`cargo test` 通过。
- 可追溯：关键输出可 stable_hash（catalog/plan/confirm/patch audit）。

---

## 1. 里程碑（建议顺序）

- **M0 基础底座**：`ais-core` + `ais-schema`（先把 issues/stable_json/hash/json codec/patch guard/schema registry 做牢）
- **M1 语义核心**：`ais-cel` + `ais-sdk resolver`（ValueRef/CEL/detect 的纯逻辑闭环）
- **M2 文档解析与校验**：`ais-sdk documents/parse/validate`（Protocol/Pack/Workflow/Plan/Skeleton/Catalog）
- **M3 Planner/Catalog**：`ais-sdk planner + catalog`（readiness/dry-run/candidates）
- **M4 Engine**：`ais-engine`（events/commands/policy gate/checkpoint/trace/plan diff/replay）
- **M5 真实 executors**：EVM（alloy）+ Solana（官方 crates）
- **M6 Runner CLI**：`ais-runner`（run/dry-run/events/commands/trace/checkpoint/replay/plan-diff）+ 集成测试

---

## 2. `ais-core`（稳定性、编码、隐私、patch 防护）

- [x] **AISRS-CORE-001 FieldPath**  
  - 依赖：无  
  - 交付物：`crates/ais-core/src/field_path.rs`  
  - 验收：支持 key/index；`Display` 输出稳定（建议 `$` 开头）；可用于 issues/audit/allowlist patterns  
  - 测试：roundtrip（parse/display，如实现 `FromStr`）；segment push/pop

- [x] **AISRS-CORE-002 StructuredIssue**  
  - 交付物：`crates/ais-core/src/issues.rs`  
  - 验收：字段齐全（kind/severity/node_id?/field_path/message/reference?/related?）；提供稳定排序函数  
  - 测试：排序稳定；serde roundtrip

- [x] **AISRS-CORE-003 stable_json**  
  - 交付物：`crates/ais-core/src/stable_json.rs`  
  - 验收：对象 key 稳定排序；可配置忽略字段（如 `created_at/ts/seq/run_id`）；输出 bytes 稳定  
  - 测试：同一输入多次输出一致；忽略字段生效

- [x] **AISRS-CORE-004 stable_hash（sha256）**  
  - 交付物：`crates/ais-core/src/stable_hash.rs`  
  - 验收：`stable_hash(value, ignore_paths)` -> hex；跨平台一致  
  - 测试：固定向量

- [ ] **AISRS-CORE-005 ais-json/1 codec**  
  - 交付物：`crates/ais-core/src/json_codec/*`  
  - 验收：支持 wire format：bigint/uint8array/error；JSONL events/commands/trace 统一使用该 codec  
  - 测试：bigint/bytes/error roundtrip；strict 拒绝 NaN/Infinity

- [ ] **AISRS-CORE-006 Redaction（default|audit|off）**  
  - 交付物：`crates/ais-core/src/redaction/*`  
  - 验收：默认模式强脱敏（私钥/seed/签名材料/完整 RPC payload/PII）；audit 保留更多结构但仍脱敏 secrets；off 不脱敏  
  - 测试：与 fixtures 对比；allow_path_patterns 放行规则正确

- [x] **AISRS-CORE-007 RuntimePatch + 校验**  
  - 交付物：`crates/ais-core/src/runtime_patch/mod.rs`  
  - 验收：`op=set|merge`；`path` 非空；value 任意 JSON  
  - 测试：schema 校验与错误映射为 `StructuredIssue`

- [x] **AISRS-CORE-008 Patch Guard（默认禁止 nodes.*）**  
  - 交付物：`crates/ais-core/src/runtime_patch/guard.rs`  
  - 验收：默认 allow roots：`inputs/ctx/contracts/policy`；拒绝 `nodes.*`；支持 allow_path_patterns/allow_nodes_paths  
  - 测试：拒绝/放行/regex 行为稳定

- [x] **AISRS-CORE-009 apply_runtime_patches + audit 摘要**  
  - 交付物：`crates/ais-core/src/runtime_patch/apply.rs`  
  - 验收：输出 applied/rejected 计数、affected_paths、partial_success、audit hash；能用于 engine 事件  
  - 测试：部分成功时摘要稳定；hash 稳定

---

## 3. `ais-schema`（版本常量 + JSON Schema 内嵌）

- [x] **AISRS-SCHEMA-001 版本常量集中化**  
  - 交付物：`crates/ais-schema/src/versions.rs`  
  - 验收：集中定义 schema ids（protocol/pack/workflow/plan/catalog/plan_skeleton/engine-event/json profile 等）  
  - 测试：无（编译期即可）；可加 `assert_eq!` smoke

- [x] **AISRS-SCHEMA-002 内嵌 `schemas/0.0.2/*.schema.json`**  
  - 依赖：AISRS-SCHEMA-001  
  - 交付物：`crates/ais-schema/src/embedded.rs`  
  - 验收：`include_str!` 内嵌；提供 `get_json_schema(schema_id) -> Option<&'static str>`  
  - 测试：能取到 `engine-event.schema.json` 等关键项

- [x] **AISRS-SCHEMA-003 JSON Schema 校验适配层**  
  - 交付物：`crates/ais-schema/src/registry.rs`（+ 选用的 validator glue）  
  - 验收：`validate(schema_id, serde_json::Value) -> Vec<StructuredIssue>`（把 validator 错误映射到 FieldPath）  
  - 测试：对一个故意缺字段/unknown 字段的文档产出稳定 issues

> 备注：如果选择“不依赖 JSON Schema validator”，则此项改为“仅做 schema_id + serde deny_unknown_fields + 手写校验”；但必须明确取舍与覆盖范围。

---

## 4. `ais-cel`（CEL 解析与求值，补齐 TS 的 `ts-sdk/src/cel`）

- [x] **AISRS-CEL-001 AST + lexer**  
  - 交付物：`crates/ais-cel/src/ast.rs`, `lexer.rs`  
  - 验收：token 带位置；支持数字/字符串/标识符/运算符/括号/列表字面量  
  - 测试：tokenization 向量

- [x] **AISRS-CEL-002 递归下降 parser**  
  - 依赖：AISRS-CEL-001  
  - 交付物：`crates/ais-cel/src/parser.rs`  
  - 验收：支持 `+ - * / %`、比较、`&& || !`、`in`、三元 `?:`、member/index/call  
  - 测试：AST 快照（推荐 `insta`）

- [x] **AISRS-CEL-003 数值模型（bigint + 精确 decimal）**  
  - 交付物：`crates/ais-cel/src/numeric.rs`  
  - 验收：整数运算使用 `bigint`；decimal 为精确表示（int+scale）；拒绝 `f64` 参与敏感路径  
  - 测试：边界（scale、负数、溢出/除零）

- [x] **AISRS-CEL-004 evaluator + 缓存**  
  - 依赖：AISRS-CEL-002/003  
  - 交付物：`crates/ais-cel/src/evaluator.rs`  
  - 验收：`evaluate(expr, ctx)`；可选 evaluator 实例缓存 parse 结果；错误带位置  
  - 测试：表达式用例（与 TS README 的示例对齐：`mul_div`、比较、member/index）

- [x] **AISRS-CEL-005 builtins（AIS 必需集）**  
  - 依赖：AISRS-CEL-004  
  - 交付物：同上（内建函数表）  
  - 验收：至少实现：`size/contains/startsWith/endsWith/matches/lower/upper/trim`、`abs/min/max/ceil/floor/round/mul_div`、`int/uint/double/string/bool/type`、`exists/all`、`to_atomic/to_human`  
  - 测试：每个 builtin 最少 1 个用例

---

## 5. `ais-sdk`（文档类型、解析、校验、resolver、planner、catalog）

### 5.1 文档类型（typed structs + strict）

- [x] **AISRS-SDK-001 Protocol/Pack/Workflow/Plan/Catalog/PlanSkeleton structs**  
  - 交付物：`crates/ais-sdk/src/documents/*`  
  - 验收：`#[serde(deny_unknown_fields)]` + `extensions` 槽；schema id 字段为必填且严格匹配  
  - 测试：JSON/YAML roundtrip（不要求格式一致，但语义一致）

### 5.2 解析（YAML/JSON + dup key 防御）

- [x] **AISRS-SDK-010 YAML 重复 key 检测（安全要求）**  
  - 交付物：`crates/ais-sdk/src/parse/detect_duplicate_keys.rs`  
  - 验收：深层 mapping 也能检测；输出 issues.field_path 尽量精确  
  - 测试：构造 YAML fixture（重复 key）必须报错

- [x] **AISRS-SDK-011 parse_document（discriminator=schema）**  
  - 依赖：AISRS-SDK-001/010  
  - 交付物：`crates/ais-sdk/src/parse/*`  
  - 验收：支持 JSON/YAML；按 `schema` 分发；错误统一为 issues  
  - 测试：每种文档至少 1 个 parse OK + 1 个 parse fail

### 5.3 Resolver / ValueRef / Detect / CEL 集成

- [x] **AISRS-SDK-020 ResolverContext（runtime + registry）**  
  - 交付物：`crates/ais-sdk/src/resolver/context.rs`  
  - 验收：runtime 根（inputs/params/ctx/contracts/nodes/…）与协议注册表；可注入 source 信息用于 issues/reference  
  - 测试：创建 ctx、写入/读取基础 ref

- [x] **AISRS-SDK-021 ValueRef 类型与求值（sync）**  
  - 依赖：AISRS-CORE-001、AISRS-CEL-004  
  - 交付物：`crates/ais-sdk/src/resolver/value_ref.rs`  
  - 验收：支持 `{lit/ref/cel/detect/object/array}`；ref 缺失 -> 返回 readiness 可消费的缺失信息  
  - 测试：lit/ref/object/array/cel 基础用例

- [x] **AISRS-SDK-022 ValueRef 求值（async + detect）**  
  - 依赖：AISRS-SDK-021  
  - 交付物：同上  
  - 验收：detect 通过 trait 调用；支持 root_overrides（params 注入）；错误可映射为 issues  
  - 测试：mock detect provider；确保 allowlist 拦截点可插入

- [x] **AISRS-SDK-023 引用解析（protocol@version/action/query）**  
  - 依赖：AISRS-SDK-020  
  - 交付物：`crates/ais-sdk/src/resolver/reference.rs`  
  - 验收：解析 action_ref/query_ref；链选择（default_chain + node.chain）；deployment/contracts 选择  
  - 测试：缺 ref/多候选/无候选 -> issues

### 5.4 校验（单文档/跨文档/workflow）

- [x] **AISRS-SDK-030 单文档 semantic 校验**  
  - 交付物：`crates/ais-sdk/src/validate/semantic.rs`  
  - 验收：schema id、必填字段、ref 格式、capabilities/execution type 合法性（基础）  
  - 测试：错误映射 `field_path` 稳定

- [x] **AISRS-SDK-031 workspace 校验（requires_pack/includes/chain_scope）**  
  - 交付物：`crates/ais-sdk/src/validate/workspace.rs`  
  - 验收：workflow.requires_pack 闭环命中；节点 chain 在 chain_scope 内；protocol@version 必须 included  
  - 测试：workspace fixtures（正/反例）

- [x] **AISRS-SDK-032 workflow 校验（DAG/deps/ValueRef refs）**  
  - 交付物：`crates/ais-sdk/src/validate/workflow.rs`  
  - 验收：无环；deps 指向存在；ValueRef ref 路径合法（能在 runtime 根下解析）  
  - 测试：cycle/unknown dep/missing ref

### 5.5 Catalog & Candidates

- [x] **AISRS-SDK-040 Catalog 构建（ais-catalog/0.0.1）**  
  - 依赖：AISRS-CORE-003/004  
  - 交付物：`crates/ais-sdk/src/catalog/build.rs`  
  - 验收：stable sort + hash（忽略 created_at）；ActionCard/QueryCard/PackCard 最小字段齐  
  - 测试：hash 稳定；排序稳定

- [x] **AISRS-SDK-041 CatalogIndex + filters**  
  - 交付物：`crates/ais-sdk/src/catalog/index.rs`, `filter.rs`  
  - 验收：filter_by_pack / filter_by_engine_caps / get_executable_candidates 稳定输出  
  - 测试：固定输入输出快照

### 5.6 Planner（Workflow/PlanSkeleton -> ExecutionPlan）

- [x] **AISRS-SDK-050 compile PlanSkeleton -> Plan**  
  - 交付物：`crates/ais-sdk/src/planner/compile_plan_skeleton.rs`  
  - 验收：按 `docs/ais-plan-skeleton.md`；失败返回 issues；成功 plan 中携带 extensions.plan_skeleton policy_hints  
  - 测试：fixtures（swap/approve/wait-until）

- [x] **AISRS-SDK-051 compile Workflow -> Plan（拓扑序稳定）**  
  - 交付物：`crates/ais-sdk/src/planner/compile_workflow.rs`  
  - 验收：deps + 隐式依赖（ValueRef ref）可选；节点 stable order；writes 默认写 `nodes.<id>.outputs`  
  - 测试：同输入多次输出完全一致（stable_json 快照）

- [x] **AISRS-SDK-052 readiness（missing_refs/needs_detect/errors）**  
  - 交付物：`crates/ais-sdk/src/planner/readiness.rs`  
  - 验收：缺 ref/detect -> blocked；condition=false -> skipped（或 ready+skip 标志）  
  - 测试：每类至少 1 个用例

- [x] **AISRS-SDK-053 dry-run（text + json）**  
  - 交付物：`crates/ais-sdk/src/planner/preview.rs`  
  - 验收：json 输出包含 per-node report + issues；text 可读且稳定（避免包含时间戳）  
  - 测试：快照

---

## 6. `ais-engine`（事件/命令/执行循环/策略/checkpoint/trace）

- [x] **AISRS-ENG-001 EngineEvent 类型 + JSONL envelope（ais-engine-event/0.0.3）**  
  - 交付物：`crates/ais-engine/src/events/*`  
  - 验收：schema/run_id/seq/ts；最小事件集合齐；seq 单调递增  
  - 测试：JSONL 行合法 + seq 单调

- [x] **AISRS-ENG-002 Trace JSONL + redaction hook**  
  - 依赖：AISRS-CORE-006、AISRS-ENG-001  
  - 交付物：`crates/ais-engine/src/trace/*`  
  - 验收：`--trace-redact default|audit|off` 可映射；默认强脱敏；允许 allow_path_patterns  
  - 测试：default vs audit vs off 差异快照

- [x] **AISRS-ENG-003 Checkpoint format + store**  
  - 依赖：AISRS-ENG-001  
  - 交付物：`crates/ais-engine/src/checkpoint/*`  
  - 验收：包含 completed node ids、paused state、seen_command_ids、runtime snapshot（可 redact）；可从 checkpoint 恢复继续  
  - 测试：serialize/deserialize；redacted payload 仍能 deserialize

- [x] **AISRS-ENG-004 EngineCommand（stdin JSONL）+ 幂等去重**  
  - 交付物：`crates/ais-engine/src/commands/*`  
  - 验收：支持 `apply_patches/user_confirm/select_provider/cancel`；command id 去重；输出 `command_accepted/rejected` 事件  
  - 测试：重复 command id 行为稳定

- [x] **AISRS-ENG-005 RuntimePatch 应用（强制 guard）+ 事件审计**  
  - 依赖：AISRS-CORE-007/008/009、AISRS-ENG-004  
  - 交付物：`crates/ais-engine/src/engine/runner.rs`（或专门模块）  
  - 验收：`apply_patches` 强制 guard；产出 `patch_applied/patch_rejected`（包含 audit hash）  
  - 测试：默认拒绝 `nodes.*`；审计 hash 稳定

- [x] **AISRS-ENG-006 Solver trait + 默认 solver**  
  - 依赖：AISRS-SDK-052  
  - 交付物：`crates/ais-engine/src/solver/*`  
  - 验收：自动填 contracts（唯一可确定时）；inputs 缺失 -> need_user_confirm；needs_detect -> select_provider 或 need_user_confirm  
  - 测试：blocked -> solver_applied / need_user_confirm 分支

- [x] **AISRS-ENG-007 Executor trait + RouterExecutor（按 chain 精确路由）**  
  - 交付物：`crates/ais-engine/src/executor/*`  
  - 验收：避免跨链误路由；支持多个 executor 并存  
  - 测试：chain mismatch 必须拒绝

- [x] **AISRS-ENG-008 PolicyGate（extract + enforce）**  
  - 交付物：`crates/ais-engine/src/policy/*`  
  - 验收：实现 `PolicyGateInput/Output`；missing/unknown 语义；支持 pack allowlist + 阈值规则（最小闭环）  
  - 测试：ok/need_user_confirm/hard_block 三分支

- [x] **AISRS-ENG-009 confirmation_summary + confirmation_hash**  
  - 依赖：AISRS-CORE-004、AISRS-ENG-008  
  - 交付物：`crates/ais-engine/src/policy/confirm_hash.rs`  
  - 验收：need_user_confirm.details 必含 summary + hash；hash 对 stable_json(summary) 计算，忽略时间戳  
  - 测试：hash 稳定（快照）

- [x] **AISRS-ENG-010 执行循环（plan-first）**  
  - 依赖：AISRS-ENG-001/003/004/006/007/008  
  - 交付物：`crates/ais-engine/src/engine/runner.rs`  
  - 验收：readiness -> solver/executor；policy gate 在写节点执行前强制；无进展 -> engine_paused 等命令  
  - 测试：最小闭环（mock executor + apply_patches + user_confirm）

- [x] **AISRS-ENG-011 Scheduler（并发与 per-chain 限制）**  
  - 交付物：`crates/ais-engine/src/engine/scheduler.rs`  
  - 验收：reads 并行、writes 默认 per-chain 串行；可配置全局/每链并发  
  - 测试：并发顺序与事件 seq 稳定（在测试中用 deterministic scheduler 或限制为 1）

- [x] **AISRS-ENG-020 plan diff（text/json）**  
  - 依赖：AISRS-CORE-003/004  
  - 交付物：`crates/ais-engine/src/plan_diff/*`  
  - 验收：added/removed/changed；changed 至少覆盖 key 字段（deps/chain/execution_type/writes）  
  - 测试：fixtures 对比

- [x] **AISRS-ENG-021 replay（trace/checkpoint）**  
  - 依赖：AISRS-ENG-002/003  
  - 交付物：`crates/ais-engine/src/trace/jsonl.rs`（+ replay helper）  
  - 验收：`replay --trace` 仅回放；`replay --checkpoint` 恢复并可继续（可选 until-node）  
  - 测试：until-node 行为

---

## 7. `ais-evm-executor`（真实 EVM executor，必须使用 alloy）

- [x] **AISRS-EVM-001 alloy 依赖选型与最小 provider 封装**  
  - 交付物：`crates/ais-evm-executor/src/provider.rs`  
  - 验收：明确使用的 alloy crates（provider/transport/rpc-types/primitives/sol-types/json-abi 等）；支持按 chain 配置 RPC URL  
  - 测试：mock transport（或本地 HTTP mock）确保不发真实网络

- [x] **AISRS-EVM-010 supports() + chain 精确匹配**  
  - 交付物：`crates/ais-evm-executor/src/executor.rs`  
  - 验收：只支持 `eip155:`；执行类型覆盖 `evm_read/evm_call/evm_rpc`（若 plan 定义含该类）  
  - 测试：chain mismatch 拒绝

- [x] **AISRS-EVM-011 evm_read：eth_call + ABI decode（alloy）**  
  - 依赖：AISRS-SDK-021/052（参数求值/readiness）  
  - 交付物：同上  
  - 验收：构造 call、发 `eth_call`、解码 outputs、按 writes 产出 runtime patches  
  - 测试：用固定 ABI + returnData fixture 做 decode

- [x] **AISRS-EVM-012 evm_call：交易构造 + 签名 + 发送 + receipt（可选）**  
  - 交付物：`crates/ais-evm-executor/src/signer.rs`  
  - 验收：EVM 交易签名接口必须是可插拔 trait；第一版先对接 runner 配置中的本地私钥 signer（dev），并保留 injected signer 扩展点；缺 signer -> need_user_confirm（提供 tx 摘要）；可配置等待 receipt 与轮询参数  
  - 测试：用 mock provider 验证发送序列与错误处理

- [x] **AISRS-EVM-013 evm_rpc：只读方法 allowlist**  
  - 验收：仅允许 `eth_getBalance/eth_blockNumber/eth_getLogs/eth_call/eth_getTransactionReceipt/eth_simulateV1` 等；禁止写类 RPC  
  - 测试：非 allowlist method 必须报错并映射为 issues

- [x] **AISRS-EVM-020 RPC/tx redaction（与 trace mode 对齐）**  
  - 依赖：AISRS-CORE-006  
  - 交付物：`crates/ais-evm-executor/src/redact.rs`  
  - 验收：default 模式不输出 raw signed tx / 完整 params；audit 输出裁剪版；off 输出完整  
  - 测试：快照

---

## 8. `ais-solana-executor`（真实 Solana executor）

- [x] **AISRS-SOL-001 solana 依赖选型与最小 RPC client 封装**  
  - 交付物：`crates/ais-solana-executor/src/types.rs` + `executor.rs`  
  - 验收：明确使用 `solana-client/solana-sdk`（或等价官方 crates）；支持按 chain 配置 RPC URL/commitment  
  - 测试：mock client（或 trait 抽象）避免真实网络

- [x] **AISRS-SOL-010 supports() + solana_read + solana_instruction**  
  - 验收：支持 `solana:`；覆盖 read 方法（getBalance/getAccountInfo/getTokenAccountBalance/getSignatureStatuses 等最小集）  
  - 测试：每个 read method 至少 1 个 fixture

- [x] **AISRS-SOL-011 solana_instruction：交易构造/签名/发送/确认**  
  - 验收：缺 signer -> need_user_confirm（提供 unsigned tx 摘要）；第一版必须支持 v0/lookup table（不接受仅 legacy 的 v1 限制）  
  - 测试：mock signer + mock rpc 验证流程

- [x] **AISRS-SOL-020 Solana redaction**  
  - 依赖：AISRS-CORE-006  
  - 验收：default 模式不输出完整 raw tx；audit 输出裁剪；off 输出完整  
  - 测试：快照

---

## 9. `ais-runner`（CLI + IO + 集成测试）

- [x] **AISRS-RUN-001 CLI 命令骨架（clap）**  
  - 交付物：`crates/ais-runner/src/cli.rs`, `main.rs`  
  - 验收：`run plan/run workflow/plan diff/replay` 命令解析；帮助文案清晰  
  - 测试：`assert_cmd` smoke（`--help`）

- [x] **AISRS-RUN-002 Workspace 加载（protocol/pack/workflow/plan）**  
  - 依赖：AISRS-SDK-011/031  
  - 交付物：`crates/ais-runner/src/io/read_document.rs`  
  - 验收：从目录读取文件并分类（不要求兼容旧后缀，但要能配置/约定）；错误输出 issues  
  - 测试：tempdir fixtures

- [x] **AISRS-RUN-003 Chain config + executor 装配（EVM alloy + Solana）**  
  - 依赖：AISRS-EVM-*、AISRS-SOL-*、AISRS-ENG-007  
  - 交付物：`crates/ais-runner/src/config.rs`  
  - 验收：按 chain 配置 RPC/signer/并发；RouterExecutor 精确匹配  
  - 测试：错误配置（缺 chain）必须失败并输出 issues

- [x] **AISRS-RUN-010 run plan：dry-run（text/json）**  
  - 依赖：AISRS-SDK-053  
  - 验收：`--dry-run --format json` 输出包含 issues + per-node report；默认 text 可读且稳定  
  - 测试：快照

- [x] **AISRS-RUN-011 run plan：执行 + events-jsonl + trace + checkpoint**  
  - 依赖：AISRS-ENG-010/002/003  
  - 验收：`--events-jsonl -` 输出 JSONL；`--trace` 写 trace；checkpoint 保存/恢复可用  
  - 测试：集成测试（mock executor 跑通 pause->commands->continue）

- [x] **AISRS-RUN-012 commands-stdin-jsonl**  
  - 依赖：AISRS-ENG-004/005  
  - 验收：开启开关后才读取 stdin；支持 apply_patches/user_confirm/cancel；输出 command_accepted/rejected  
  - 测试：用管道喂 JSONL，验证状态机

- [x] **AISRS-RUN-020 plan diff**  
  - 依赖：AISRS-ENG-020  
  - 验收：`--format text|json` 两种输出；exit code 正确  
  - 测试：fixtures

- [x] **AISRS-RUN-021 replay（trace/checkpoint）**  
  - 依赖：AISRS-ENG-021  
  - 验收：until-node；错误输出 issues  
  - 测试：fixtures

---

## 10. Fixtures 与测试数据

- [x] **AISRS-FIX-001 最小 workspace fixtures（protocol/pack/workflow）**  
  - 交付物：`fixtures/`（或 `crates/ais-fixtures/`）  
  - 覆盖：includes/chain_scope/allowlist/policy gate/need_user_confirm/patch guard  
  - 用于：SDK 校验、planner、engine 集成测试

- [x] **AISRS-FIX-002 计划与事件 fixtures（plan/trace/checkpoint）**  
  - 覆盖：plan diff、replay、redaction  
  - 要求：所有 JSON 经过 stable_json 规范化，避免无关字段漂移

---

## 11. 文档交付（与 Prompt 对齐）

- [x] **AISRS-DOC-001 `docs/architecture.md`（Rust 模块与交互架构）**  
- [x] **AISRS-DOC-002 `docs/protocols.md`（事件/命令/plan Rust 类型与版本策略）**  
- [x] **AISRS-DOC-003 `docs/cli.md`（runner 示例与输出格式：text/json/jsonl）**

---

## 12. 风险与需要尽早拍板的决策

- **Workflow schema 版本**：统一使用 `ais-flow/0.0.3`（影响 loader/discriminator/fixtures）。  
- **JSON Schema 校验策略**：是否强依赖 JSON Schema validator crate，还是以 serde strict + 手写校验为主（影响一致性与成本）。  
- **Solana v0/lookup table**：第一版必须支持，相关测试与 mock 作为必做项。  
- **EVM 交易签名接口**：采用可插拔 trait；第一版先接 runner 配置中的本地私钥 signer（dev），后续扩展 prod signer。  

---

## 13. `ais-flow/0.0.3` 语义差异清单 + 对应实现任务

> 说明：本节用于跟踪“已完成版本字符串迁移”之后的语义补齐工作；每项都要求有 fixtures + 测试。

### 13.1 差异清单（对照实现状态）

- [ ] `WF03-DIFF-001` `imports.protocols[]` 语义闭环（`protocol/path/integrity` 字段约束 + 解析可用性）
- [ ] `WF03-DIFF-002` `nodes[].protocol` 必须与 workspace/imports/includes 对齐（引用来源可追溯）
- [ ] `WF03-DIFF-003` `assert` / `assert_message` 语义落地（失败行为、issue/event 映射、错误信息稳定）
- [ ] `WF03-DIFF-004` `calculated_overrides` 表达式求值时序与依赖语义（与 `args`/`deps` 一致）
- [ ] `WF03-DIFF-005` `preflight.simulate` 在 planner/runner/engine 链路的一致行为
- [ ] `WF03-DIFF-006` `policy` 字段在 workflow->plan->engine 的传递与执行一致性
- [x] `WF03-DIFF-007` `condition` 执行前语义（falsy => skipped，truthy => 继续执行）
- [x] `WF03-DIFF-008` `until/retry/timeout_ms` 轮询生命周期语义（停止条件、重试边界、超时边界）

### 13.2 对应实现任务（可追踪）

- [x] **AISRS-SDK-033 Workflow imports 语义校验**
  - 依赖：AISRS-SDK-011/031
  - 交付物：`crates/ais-sdk/src/validate/workflow.rs`
  - 验收：`imports.protocols[]` 字段合法；`protocol` 引用与 imports/workspace 可闭环；错误映射稳定
  - 测试：imports 正/反例 fixtures（缺字段、坏格式、找不到协议）

- [x] **AISRS-SDK-034 Workflow assert 语义编译与校验**
  - 依赖：AISRS-CEL-004/005、AISRS-SDK-032
  - 交付物：`crates/ais-sdk/src/planner/compile_workflow.rs` + `validate/workflow.rs`
  - 验收：`assert`/`assert_message` 编译进 plan；语义错误在 compile 阶段可诊断
  - 测试：assert 成功/失败/类型错误 fixtures

- [x] **AISRS-SDK-035 calculated_overrides 语义对齐**
  - 依赖：AISRS-SDK-021/022
  - 交付物：`crates/ais-sdk/src/planner/compile_workflow.rs` + `resolver/*`
  - 验收：按节点时序求值；依赖缺失可报告；输出稳定
  - 测试：override 链式依赖、缺 ref、循环依赖案例

- [x] **AISRS-ENG-022 workflow assert/preflight 执行语义**
  - 依赖：AISRS-ENG-010、AISRS-SDK-034
  - 交付物：`crates/ais-engine/src/engine/runner.rs`
  - 验收：assert 失败触发预期事件与 pause/stop 策略；`preflight.simulate` 行为一致
  - 测试：engine 集成测试（mock executor + deterministic events）

- [x] **AISRS-RUN-022 run workflow（0.0.3 语义模式）**
  - 依赖：AISRS-RUN-002、AISRS-SDK-033/034/035、AISRS-ENG-022
  - 交付物：`crates/ais-runner/src/run.rs`
  - 验收：`run workflow` 可在 `ais-flow/0.0.3` 语义下完成 compile/validate/execute
  - 测试：端到端 fixtures（success/fail/assert/policy/preflight）

- [x] **AISRS-FIX-003 Workflow 0.0.3 conformance fixtures**
  - 交付物：`fixtures/workflow-0.0.3/*`
  - 验收：覆盖 imports/assert/calculated_overrides/preflight/policy 全路径
  - 测试：供 SDK/Runner/Engine 共用

- [x] **AISRS-DOC-004 `docs/workflow-0.0.3-conformance.md`**
  - 交付物：差异矩阵（spec 字段 -> rust 实现 -> 测试用例）
  - 验收：每个 0.0.3 关键语义都有实现位置与测试链接

- [x] **AISRS-ENG-023 workflow condition 执行语义**
  - 依赖：AISRS-ENG-010
  - 交付物：`crates/ais-engine/src/engine/runner.rs`
  - 验收：`condition` 在执行前求值；`false` 产出 `skipped(reason=condition_false)` 并完成节点；求值失败可诊断
  - 测试：`condition=false` 跳过执行；`condition=true` 正常执行；无效 condition 报错

- [x] **AISRS-ENG-024 workflow until/retry 执行语义**
  - 依赖：AISRS-ENG-023
  - 交付物：`crates/ais-engine/src/engine/runner.rs`
  - 验收：`until=false` 进入重试；遵守 `retry.interval_ms/max_attempts/backoff=fixed`
  - 测试：重试成功、超过最大重试、无 retry 配置三分支

- [x] **AISRS-ENG-025 workflow timeout_ms 执行语义**
  - 依赖：AISRS-ENG-024
  - 交付物：`crates/ais-engine/src/engine/runner.rs`
  - 验收：轮询生命周期超时后停止重试并输出稳定 reason/event
  - 测试：超时触发与边界值（正整数校验）覆盖
