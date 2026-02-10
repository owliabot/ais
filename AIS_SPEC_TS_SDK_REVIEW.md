# AIS Specs & TS SDK 评审（不考虑历史兼容）

日期：2026-02-05  
范围：`specs/`（AIS-1/2/3）、`examples/`、`docs/`、`ts-sdk/`  
目标：找出规范与实现的问题、提出改进建议，并评估 TS SDK 的结构质量（优雅/低耦合）。

---

## 0. 快速结论（TL;DR）

- **当前最大风险不是“缺功能”，而是“语义不唯一/不一致”**：同一概念在 `specs/`、`examples/`、`ts-sdk` 的 schema/实现/README 中出现多种表述，导致 Agent 很难稳定地产出正确 YAML，也很难写出严格的验证器。
- **EVM 侧的执行（AIS-2）在“ABI 表达 + mapping → 参数顺序/tuple”上规范不足**，TS SDK 也因此实现不完备（tuple/struct 编码、mapping 顺序绑定、BigInt 精度）。
- **Solana 支持在 repo 内已经出现“事实标准”**（`docs/proposal-solana-support.md`、`ts-sdk/src/execution/solana/`、`examples/spl-token.ais.yaml`），但 AIS-2 规范正文与 TS SDK 的通用执行模型（`composite`）尚未吸收这些变化，造成断裂。
- **建议把 AIS vNext 的核心改动集中在 3 件事**：  
  1) 统一“引用/取值”语法（不要让字符串同时扮演字面量与引用）；  
  2) 统一 EVM ABI 描述（使用标准 JSON ABI 或显式的 `inputs:[{name,type}]`）；  
  3) 统一数值域（token_amount/uint256/CEL）为“精确十进制字符串 + BigInt 运算”，禁止 JS `number` 参与关键金额计算。

---

## 1. Specs 设计评审：问题与建议

### 1.1 文档与示例不一致（会直接伤害 Agent 生成 YAML 的正确率）

**现状：**
- `specs/ais-1-core.md` 给出 Pack 的简化结构（`name/version/includes:[string]`），但 `examples/safe-defi-pack.ais-pack.yaml` 采用 `meta + includes:[{protocol,version,source,...}] + providers + overrides` 的更丰富形态。
- `specs/ais-2-execution.md` 的 `composite.steps[].type` 示例主要面向 EVM（`evm_call|evm_read`），但 `examples/spl-token.ais.yaml` 使用了 `composite` + `solana_instruction` 作为 step。
- `specs/ais-1-core.md` 的 CEL “内置函数”列表未包含 `examples/spl-token.ais.yaml` 里出现的 `derive_ata()`。

**建议（vNext）：**
- 以 `examples/` 作为“事实基线”反向修订 `specs/`：把示例里已出现的字段与语义写成**规范性（MUST/SHOULD）**条款。
- 给每个文档加“**规范状态**”与“**与实现对齐的 commit/tag**”（例如 `docs/HANDOFF-solana-support.md` 已经记录 commit），避免 README/Spec 漂移。

### 1.2 “引用/字面量”语义冲突：字符串到底是值还是引用？

**现状：**
- AIS-1 Workflow 的 `args` 在描述上是“引用 `inputs.*` / `nodes.*`”，但示例 `examples/swap-to-token.ais-flow.yaml` 直接写 `"inputs.token_in"` 这类字符串；TS SDK 的 resolver/validator 更偏向 `${...}` 模板（`ts-sdk/src/resolver/expression.ts`）。
- 同样的问题也出现在 execution `mapping`：字符串既可能是 `"0"` 字面量，也可能是 `"params.token_in.address"` 引用，还可能是 CEL 表达式。

**风险：**
- Agent 无法可靠地区分 `"inputs.token_in"` 是“字符串常量”还是“引用”；这会让 lint/validate/build 三者都变得不可靠。

**建议（强烈，vNext 关键）：**
- 引入**显式引用类型**，不要用“裸字符串”承载引用语义。例如：
  - `{"ref":"inputs.token_in"}`
  - `{"cel":"floor(nodes.q_quote.outputs.amount_out_atomic * (1 - inputs.slippage_bps/10000))"}`
  - `{"lit":"0"}` 或 `{"lit":0}`
- 允许模板字符串作为可选糖：`"Swap ${ref:inputs.token_in.symbol}"`，但底层依旧解析成结构化 AST（避免与 YAML 字面量混淆）。

### 1.3 类型系统与数值域：金额/精度在规范层必须“可证明正确”

**现状：**
- AIS-1 表述里 `uint256` 倾向“字符串表示”，`token_amount` 是人类可读金额；但规范没有强制统一“内部计算域”（string/BigInt/decimal）。
- CEL 在规范中用于关键计算（minOut、deadline、approval_amount），若实现用 JS `number` 会出现精度灾难（尤其 18 位小数与大额）。

**建议（vNext）：**
- **规范化数值表示：**
  - `uint* / int*`：十进制字符串（禁止 YAML number），执行引擎内部用 `BigInt`。
  - `token_amount`：十进制字符串（允许小数），并且必须能无损转 `atomic`（通过字符串小数 → BigInt 乘 10^decimals 的精确算法）。
  - CEL 的数值类型：对关键金额计算应支持 `decimal`（字符串十进制）或 `int`（BigInt），禁止落入 IEEE754。
- `to_atomic/to_human` 的输入/输出类型在规范中写死（例如 `to_atomic(token_amount, asset)->uint256_string`），并给出**测试向量**。

### 1.4 EVM ABI 描述与 `mapping` 的“顺序”问题（规范缺口导致实现必然脆弱）

**现状：**
- AIS-2 用 `abi: "(address,uint256,...)"` 描述参数类型，但没有参数名；`mapping` 是 object（无强顺序语义）。一旦实现用“遍历对象的插入顺序”去对齐 ABI，属于隐式约定。
- tuple/struct 是现实刚需（`examples/uniswap-v3.ais.yaml` 的 `exactInputSingle`），但 AIS-2 与 TS SDK 的 EVM encoder 都未把 tuple 作为一等公民来规范化（仅靠 `abi` 字符串表达嵌套 tuple 非常易错）。

**建议（vNext，二选一即可，推荐 A）：**
- A) **使用标准 JSON ABI**（包含 `inputs:[{name,type,components?...}]`），`mapping` 以 `input.name -> valueRef` 对齐；tuple 由 `components` 描述，编码无需再做脆弱的字符串解析。
- B) 保留 `abi` 字符串，但新增 `inputs:[{name,type}]` 明确顺序与名字；`mapping` 必须覆盖所有 `inputs[].name`。

### 1.5 `composite` 应该变成“通用步骤容器”，而不是 EVM 专属

**现状：**
- AIS-2 的 `composite` 描述以 EVM 为主，但仓库已经出现 Solana “多 instruction”需求（`examples/spl-token.ais.yaml`）。

**建议（vNext）：**
- 把 `composite.steps[]` 定义为 **`ExecutionSpec` 的内联实例**（或 `step.execution: ExecutionSpec`），并允许 step 级别的 `condition`、`id`、`description`。
- 对“多链多步骤”提供最小一致语义：  
  - `condition` 的上下文与可引用命名空间；  
  - step 输出/回填（如果支持的话）；  
  - preflight/simulation 钩子。

### 1.6 Solana 规范应吸收 repo 中已实现的扩展点

**现状：**
- `docs/proposal-solana-support.md` 与 `ts-sdk/src/execution/solana/accounts.ts` 已经定义了 `system:*`、`sysvar:*`、`query.*` 等来源；但 AIS-2 正文只写了较窄的 `source` 描述。

**建议（vNext）：**
- 将 Solana `accounts[].source` 规范化为枚举或结构化形式，至少覆盖：
  - `wallet` / `params.*` / `calculated.*` / `query.*`
  - `system:<key>`（token_program、associated_token_program、system_program…）
  - `sysvar:<name>`（rent、clock…）
  - `constant:<address>`
- 明确 ATA/PDA 的表达：推荐用结构化 `derived`（而不是用 CEL 函数去派生地址），例如：
  - `derived: { kind: "ata", owner: {ref:"ctx.wallet_address"}, mint:{ref:"params.token.address"} }`
  - `derived: { kind: "pda", program:{lit:"..."} , seeds:[{lit:"pool"}, {ref:"params.pool_id"}] }`

### 1.7 AIS-3 Registry：`skillId` 与 `update(version)` 语义矛盾

**现状：**
- AIS-3 定义：`skillId = keccak256(owner, protocol, version)`，但 `update(skillId, version, ...)` 又允许变更 version。若 version 变了，理论上 skillId 应该也变，语义冲突。

**建议（vNext）：**
- 选择一种一致模型：
  - 模型 1（推荐）：`skillId` 不包含 version（例如 `keccak256(owner, protocol)`），version 作为可变字段；  
  - 模型 2：`skillId` 包含 version，但 **update 不允许改 version**；改 version 必须 `register` 新 skill，并由 `latestByName` 指向最新。
- 规范 `specHash` 的“哈希对象”：YAML 非规范化，建议明确“哈希的是原始字节”还是“canonical JSON”等，以免不同序列化导致 hash 不一致。

---

## 2. TS SDK 是否符合 Spec：差异与调整建议

### 2.1 Schema 层（`ts-sdk/src/schema/*`）

**总体：**Protocol/Pack/Workflow 的顶层 `schema: ais/1.0 | ais-pack/1.0 | ais-flow/1.0` 与 `examples/` 基本一致；但存在几个会“卡死真实用例”的点：

1) **`risk_tags` 被限制为枚举**（`ts-sdk/src/schema/protocol.ts`），但规范（AIS-1）与示例更像“开放字符串集合”，且 `examples/spl-token.ais.yaml` 使用了 `"transfer"` 这类非枚举 tag。  
   - 调整建议：schema 放宽为 `z.array(z.string())`；把“推荐 tag 集”挪到 lint 规则（best practice），不要在 parse/validate 阶段 hard fail。

2) **`composite.steps[].type` 仅允许 `evm_call|evm_read`**（`ts-sdk/src/schema/execution.ts`），与 `examples/spl-token.ais.yaml` 的 `solana_instruction` step 不兼容。  
   - 调整建议：按 1.5 把 composite step 变成通用容器（或新增 `solana_composite`）。

3) Param type 未使用 `AISTypeSchema` 强约束（`ParamSchema.type: z.string()`），导致“schema 通过但执行/计算时失败”的概率增大。  
   - 调整建议：用 `AISTypeSchema` 约束基础类型；对 `array/tuple` 引入更严格解析；对 `asset/token_amount` 加结构约束（至少在 runtime normalize 前验证形状）。

### 2.2 Resolver/表达式（`ts-sdk/src/resolver/*`）

**现状：**
- resolver 偏向 `${...}` 模板（`extractExpressions/resolveExpressionString`），而 `examples/swap-to-token.ais-flow.yaml` 使用裸引用字符串（`"inputs.token_in"`）。

**问题：**
- 裸引用在 TS SDK 中会被当作字面量字符串传递，导致 workflow 无法真正“连线”。

**调整建议（不考虑兼容）：**
- 直接采用 vNext 的结构化引用（`{ref:...}` / `{cel:...}` / `{lit:...}`），并在 resolver 中统一解析；保留 `${...}` 作为可选模板糖，最终也编译到结构化引用 AST。

### 2.3 CEL 引擎（`ts-sdk/src/cel/*`）与金额精度

**现状：**
- `to_atomic/to_human` 在 `ts-sdk/src/cel/evaluator.ts` 使用 `parseFloat` + `Math.pow` + `number`，对大数与 18 位小数会产生不可接受的精度误差。
- `execution/builder.ts` 把 `BigInt` 转为 `Number` 塞入 CEL 上下文（`toCELValue()`），并声称“比较仍然可用”，这在大额 allowance/balance 场景不成立。

**调整建议（vNext 必做）：**
- CEL 数值域升级为：
  - `int`：BigInt
  - `decimal`：十进制字符串（实现精确四则运算/比较，至少覆盖 spec 用到的 `floor/ceil/round` 与乘除）
- `to_atomic`/`to_human` 必须完全精确（基于字符串小数拆分实现），并返回 `uint256` 字符串或 BigInt。

### 2.4 EVM 执行构建（`ts-sdk/src/execution/*`）

**关键不符合点：**
1) **不支持 tuple/struct ABI 编码**：`ts-sdk/src/execution/encoder.ts` 明确写了 “Tuple/struct support would go here”；而 `examples/uniswap-v3.ais.yaml` 的核心调用就是 tuple。  
2) **`buildEvmCall()` 用逗号 split 解析 ABI**（`ts-sdk/src/execution/builder.ts`），无法正确处理嵌套 tuple。  
3) **参数顺序依赖 `Object.entries(mapping)`**：mapping 是 object，规范层未定义顺序语义，这会导致编码时参数错位风险。  
4) **不支持 `asset` 复合参数与 `token_amount` 绑定语义**：`resolveParamValue()` 把 `asset` 映射成 `address`，要求输入是 `0x...` 字符串；但 AIS-1 的 `asset` 是对象（含 address/decimals），示例也用 `params.token_in.address` 访问字段。

**调整建议（不考虑兼容，按 1.4 的 ABI 方案走）：**
- 采用 JSON ABI（或显式 `inputs[]`）后，编码器自然获得顺序与 tuple 描述；mapping 按 name 对齐，不再依赖对象遍历顺序。
- 引入 “runtime normalization” 步骤：把输入 `params` 标准化成规范的结构（asset/token_amount），并把常用投影写入上下文（`params.token.address`、`params.token.decimals`、`params.amount_human`、`calculated.amount_atomic`）。

### 2.5 Solana 执行（`ts-sdk/src/execution/solana/*`）

**现状：**
- Solana 已有独立模块（账户解析、ATA/PDA、Borsh），并支持 `system:*`/`sysvar:*` 等（见 `ts-sdk/src/execution/solana/accounts.ts`）。
- 但它与通用 `buildTransaction()`/workflow 执行路径尚未统一：Solana 需要 instruction 列表/transaction 组装策略，而 EVM builder 输出的是 `to/data/value`。

**调整建议（vNext）：**
- 抽象统一的执行产物：
  - `EvmTxRequest[]`（现有）
  - `SolanaInstructionPlan`（instructions + computeUnits + lookupTables + preInstructions）
- `composite` 统一后，Solana/EVM 都走同一套 step 编排语义（condition、依赖、输出引用）。

---

## 3. TS SDK 结构评估：是否合理、优雅、低耦合？

### 3.1 优点

- 目录分层清晰：`schema/`、`parser/loader`、`resolver`、`validator`、`execution`、`cel`、`builder`、`cli`（见 `ts-sdk/AGENTS.md` 的结构说明）。
- Zod schema + `z.infer` 的组合，让“解析即类型”在 TS 侧体验较好。
- Solana 子模块走“零依赖”路线，便于在受限环境运行（尽管需要更严格的测试向量来证明正确性）。

### 3.2 主要结构问题（导致耦合与一致性问题）

1) **三套表达式/引用体系并存**：  
   - workflow `${...}` 模板（resolver）  
   - execution mapping 的 `"params.*"` 字符串引用  
   - CEL 表达式字符串  
   这会让“静态验证”和“运行时求值”难以统一，也让 spec 很难写清楚。

2) **`ResolverContext` 是可变的 string-key 字典**（`variables: Record<string, unknown>`），execution builder 会往里面塞 `params.*`/`calculated.*`，容易产生隐式依赖与键冲突，且难以做类型检查/自动补全。

3) **执行层重实现 ABI/Borsh 时缺乏“规范输入”**：因为上游没有统一的结构化 ABI/引用模型，下游只能用脆弱的字符串解析和对象遍历顺序。

4) **文档漂移明显**：`ts-sdk/README.md` 与 `ts-sdk/src/builder/README.md` 的示例字段（`ais_version/type/method` 等）与当前 schema/实现不一致，降低可维护性与外部使用者信任。

### 3.3 建议的目标架构（vNext，低耦合）

- **核心统一：引用 AST + 精确数值域**
  - `ValueRef = Lit | Ref(path) | Cel(expr) | Detect(...)`
  - `Numeric = bigint | decimal_string`
- **执行层只做两件事：**
  1) 把 `ValueRef` 在给定上下文下求值（纯函数，易测）；  
  2) 把“已求值的参数”编码成链上交易结构（EVM/Solana 各自实现）。
- **Context 结构化**（不要用 string-key 扁平字典）：
  - `ctx: { wallet_address, chain_id, now, policy... }`
  - `params: { ... }`
  - `query: { [id]: { ... } }`
  - `contracts: { ... }`
  - `calculated: { ... }`

### 3.4 建议的改造优先级（不考虑兼容）

P0（会决定能否正确执行）：
- 结构化引用（替代裸字符串引用）+ 统一 resolver/validator
- 精确金额与 BigInt（替换 `number` 的关键路径）
- EVM JSON ABI + tuple 编码支持（或引入成熟库）
- mapping 与 ABI 输入顺序绑定（按 name 对齐）

P1（提升可用性与覆盖链）：
- `detect` provider 接口规范化 + SDK 插件化实现
- Query 输出 decode（eth_call 返回值解析）+ query→calculated 的闭环
- composite 跨链统一 + workflow 端到端执行计划生成

P2（工程质量）：
- 清理重复测试目录（`ts-sdk/test/` vs `ts-sdk/tests/`）与 README 同步
- 为 spec/examples/sdk 增加一致性 CI（schema validate + golden tests）

---

## 附：本次评审主要参考文件

- 规范：`specs/ais-1-core.md`、`specs/ais-2-execution.md`、`specs/ais-3-registry.md`
- 示例：`examples/uniswap-v3.ais.yaml`、`examples/aave-v3.ais.yaml`、`examples/safe-defi-pack.ais-pack.yaml`、`examples/swap-to-token.ais-flow.yaml`、`examples/spl-token.ais.yaml`
- Solana 方案与交接：`docs/proposal-solana-support.md`、`docs/HANDOFF-solana-support.md`
- SDK 关键实现：`ts-sdk/src/schema/*`、`ts-sdk/src/execution/builder.ts`、`ts-sdk/src/execution/encoder.ts`、`ts-sdk/src/execution/solana/*`、`ts-sdk/src/cel/evaluator.ts`、`ts-sdk/src/resolver/*`

