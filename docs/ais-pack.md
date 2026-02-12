# AIS Pack (`ais-pack/0.0.2`) 使用与约束说明（结合 TS SDK 实现）

本文面向在本仓库中编写/校验 `.ais-pack.yaml` 的使用者，内容以规范 `specs/ais-1-pack.md` 与 TypeScript SDK 实现为准：
- Schema 定义：`ts-sdk/src/schema/pack.ts`
- 解析与校验：`ts-sdk/src/parser.ts`
- 约束校验（policy gate）：`ts-sdk/src/validator/constraint.ts`
- 跨文件/工作区关系校验：`ts-sdk/src/validator/workspace.ts`

## 1. Pack 是什么

Pack 是 **策略边界**（policy boundary）和 **协议集合**（protocol bundle）：
- 通过 `includes[]` 选择允许使用的协议 spec（protocol@version）。
- 通过 `policy` / `token_policy` / `providers` / `plugins` 约束运行时行为（例如：滑点上限、是否允许无限授权、token allowlist、detect providers allowlist、执行插件 allowlist 等）。

规范侧的关键点（见 `specs/ais-1-pack.md`）：
- Pack 对象是 **strict**：未知字段必须拒绝。
- 扩展字段必须放进 `extensions`（自由结构，解释权在实现）。
- `providers.*` 和 `plugins.*` 在 pack 生效时应作为 **allowlist**（见 `specs/ais-1-capabilities.md`）。

## 2. YAML 怎么写（最小可用）

SDK 的 `PackSchema` 要求顶层必须包含：
- `schema: "ais-pack/0.0.2"`
- `includes: [...]`

另外 pack **必须有 name+version**（用于 `workflow.requires_pack` 与 workspace 校验），但 name/version 可以放在两种位置之一：
- 顶层 `name` / `version`（SDK builder 默认产出这种形式）
- `meta: { name, version, ... }`（更贴近 spec 示例）

示例（偏向 spec 写法）：

```yaml
schema: "ais-pack/0.0.2"

meta:
  name: "safe-defi-pack"
  version: "0.0.2"
  description: "..."

includes:
  - protocol: "uniswap-v3"
    version: "0.0.2"
    chain_scope: ["eip155:8453"]

policy:
  approvals:
    auto_execute_max_risk_level: 2
    require_approval_min_risk_level: 3
  hard_constraints_defaults:
    max_slippage_bps: 50
    allow_unlimited_approval: false

token_policy:
  resolution:
    require_allowlist_for_symbol_resolution: true
  allowlist:
    - { chain: "eip155:8453", symbol: "USDC", address: "0x0000000000000000000000000000000000000000", decimals: 6 }

providers:
  detect:
    enabled:
      - kind: "best_quote"
        provider: "uniswap-v3-fee-detect"
        chains: ["eip155:8453"]
        priority: 10

plugins:
  execution:
    enabled:
      - type: "my_plugin_exec_type"
        chains: ["eip155:1"]
```

## 3. SDK 里 pack 怎么用

### 3.1 解析 YAML 并做 schema 校验

SDK 提供 `parsePack(yaml)`，会：
- 用 `yaml` 包解析 YAML（开启 `uniqueKeys: true`，重复 key 会报错）
- 用 `PackSchema` 做严格校验（`.strict()`，未知字段报错）

```ts
import { parsePack } from '@owliabot/ais-ts-sdk';

const pack = parsePack(packYaml, { source: '.ais-pack.yaml' });
```

注意：`parsePack` 目前只做结构校验，不做 “语义/跨文件” 校验（例如：includes 引用的协议是否存在，见 3.3）。

### 3.2 用 Builder 生成 pack

SDK 提供 `PackBuilder`（`pack(name, version)`）来程序化生成 pack，并在 `.build()` 时做 `PackSchema` 校验。

```ts
import { pack } from '@owliabot/ais-ts-sdk';

const p = pack('defi-essentials', '0.0.2')
  .description('Essential DeFi protocols')
  .include('uniswap-v3', '0.0.2', { chain_scope: ['eip155:8453'] })
  .maxSlippage(100)
  .disallowUnlimitedApproval()
  .approvals({ auto_execute_max_risk_level: 2, require_approval_min_risk_level: 3 })
  .build();
```

Builder 覆盖的字段范围是 “常用子集”，目前不支持直接设置：
- `meta`（只能用顶层 `name/version/description`）
- `providers.detect`（只有 `quoteProvider` / `routingProviders`）
- `plugins` / `overrides` / `extensions`

如果需要这些字段，建议直接写 YAML 或在 `.buildUnsafe()` 后自行补齐，再用 `PackSchema.parse()` 二次校验。

### 3.3 工作区（workflow/pack/protocol）联动校验

单文件 schema 校验不能保证 pack 真能“约束住”工作区里的 workflow/protocol。

SDK 提供 `validateWorkspaceReferences({ protocols, packs, workflows })` 做跨文件校验（关键行为）：
- pack `includes[]` 必须能在 workspace 里找到对应 `protocol@version`
- workflow 如果设置了 `requires_pack`：
  - pack 必须存在
  - workflow 节点使用的 `protocol@version` 必须被 pack includes
  - 若 pack include 配了 `chain_scope`，则 node.chain（或 workflow.default_chain）必须落在 scope 内
  - workflow 使用的 `ValueRef.detect` kind/provider 必须被 pack `providers.detect.enabled` allowlist 覆盖（至少存在一个 provider）
- pack 的 allowlist 不应与它 includes 的协议 spec 自相矛盾（能力边界检查）：
  - 如果协议 spec 内引用了某 detect provider（`detect.kind + detect.provider`），但 pack 未启用该 pair，则报错
  - 如果协议 spec 使用了非 core 的 execution type（plugin execution type），但 pack `plugins.execution.enabled` 未 allowlist 该 type，则报错

这部分是 “pack 能实现哪些约束” 在 SDK 中最完整的落地。

## 4. Pack 在 SDK 中“可实现”的约束有哪些

这里按 “SDK 现有实现” 来说，而不是 spec 理想状态。

### 4.1 结构约束（schema 层）

由 `PackSchema` / 相关子 schema 直接保证：
- strict：未知字段报错（对象均 `.strict()`）
- `schema` 只能是 `ais-pack/0.0.2`
- `meta.version` 必须是 semver（`x.y.z`）
- `includes[].source` 只能是 `registry|local|uri`
- `chain_scope`/`chains` 必须符合 CAIP-2 格式（如 `eip155:1`）
- `providers.detect.enabled[].kind` 只能是 `choose_one|best_quote|best_path|protocol_specific`

### 4.2 工作区约束（workflow/pack/protocol 关系）

由 `validateWorkspaceReferences()` 落地的约束：
- 协议集合约束：workflow 在 requires_pack 场景下只能用 pack includes 的协议
- 链范围约束：include.chain_scope 限制 workflow 节点可用链
- detect allowlist 约束：workflow 使用的 detect 必须被 pack 启用
- “pack 不允许协议使用被禁用能力”的约束：
  - 禁用的 detect provider（kind:provider）不能被 included protocol 引用
  - 禁用的 plugin execution type 不能被 included protocol 使用

### 4.3 运行时约束（policy gate）

由 `validateConstraints(pack.policy, pack.token_policy, input)` 实现的约束：
- token allowlist：
  - 若 `token_policy.allowlist` 非空，会检查输入 token（address 或 symbol，带可选 chain）是否在 allowlist
  - 若 `token_policy.resolution.require_allowlist_for_symbol_resolution` 为真，则不在 allowlist 直接视为硬违规（`violations`）
  - 否则会返回 `requires_approval=true`（软门槛，需要人工确认）
- 滑点上限：`policy.hard_constraints_defaults.max_slippage_bps`
- 禁止无限授权：`policy.hard_constraints_defaults.allow_unlimited_approval === false`
- 风险阈值：
  - `policy.approvals.auto_execute_max_risk_level` / `require_approval_min_risk_level`
  - legacy：`policy.risk_threshold`
  - legacy：`policy.approval_required`（risk tags 命中则 require approval）

## 5. SDK 支持度总结（现状与缺口）

### 5.1 已支持（可用）

- pack YAML 的解析与严格 schema 校验：`parsePack` + `PackSchema`
- 常用 pack 构建（includes + approvals + 基础 hard constraints + token allowlist + quote/routing providers）：`PackBuilder`
- policy gate（token allowlist / slippage / unlimited approval / risk approval）：`validateConstraints`
- 工作区级 pack 约束：
  - workflow 的 requires_pack 与 includes/chain_scope 的一致性
  - detect allowlist（workflow 与 protocol 两侧）
  - plugin execution type allowlist（protocol 侧）

### 5.2 规范里有，但 SDK 目前未完整落地的点

- `hard_constraints_defaults` 中的 `max_spend` / `max_approval` / `max_approval_multiplier`：
  - schema 支持，但 `validateConstraints` 目前未做任何数值/单位解析与比较（`spend_amount`/`approval_amount` 输入也未使用）
- `providers.quote.enabled`：
  - schema/builder 支持，但 workspace 校验与 constraint 校验都不消费该 allowlist
- `providers.detect.enabled[].chains/priority`：
  - schema 支持，但 workspace 校验目前只检查 `(kind, provider)` 是否被启用，不校验 chain/priority 选择逻辑
- `plugins.execution.enabled[].chains`：
  - schema 支持，但 workspace 校验目前只按 `type` 做 allowlist，不校验 chain 维度
- `overrides.actions.*`：
  - schema 支持，但 validator/runner 侧暂未看到使用点（不会自动把 override 的 hard constraints 合并进 gate）
- `includes[].source/uri` 的语义：
  - schema 支持，但 SDK 当前不负责从 registry/local/uri 加载协议文件（只做关系校验，实际加载在别的层/工具里完成）

如果你的目标是 “在执行前严格阻断不符合 pack 的行为”，建议把 pack 的这些缺口放到 runner/executor 层补齐（例如：对 quote provider、detect chain/priority、overrides 合并、max_spend/max_approval 解析等做 runtime enforcement）。

