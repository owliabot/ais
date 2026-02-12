# AIS-1D: Catalog Cards (`ais-catalog/0.0.1`)

Status: Draft  
Spec Version: 0.0.2  

Catalog 的目标是让 agent 进行检索与选择时不需要吞全量 protocol/pack/workflow 原文，而是基于“可检索卡片摘要”进行工作。

## 1. 设计目标

- 可检索：字段尽量扁平、可索引（按 protocol/version/action/query/risk/capability/chain）。
- 可对账：输出稳定排序，可计算 hash，便于缓存与差异对比。
- 可增量更新：支持记录来源文档的 hash，方便只重算变更部分（实现可选）。

## 2. 顶层结构

建议 JSON 结构（单文件）：

- `schema`: `"ais-catalog/0.0.1"`
- `created_at`: RFC3339
- `hash`: sha256（对规范化后的 catalog 内容）
- `documents`: 来源文档摘要列表（可选，但推荐）
- `actions`: `ActionCard[]`
- `queries`: `QueryCard[]`
- `packs`: `PackCard[]`
- `extensions?`: 扩展槽

## 3. ActionCard

推荐字段集合（最小可用）：

- `ref`: `protocol@version/actionId`
- `protocol`, `version`, `id`
- `description?`
- `risk_level`, `risk_tags?`
- `params?`: `{ name, type, required?, asset_ref? }[]`
- `returns?`: `{ name, type }[]`
- `requires_queries?`: `string[]`
- `capabilities_required?`: `string[]`
- `execution_types`: `string[]`（按 execution block 汇总）
- `execution_chains`: `string[]`（execution block 的链 key 汇总）

## 4. QueryCard

推荐字段集合（最小可用）：

- `ref`: `protocol@version/queryId`
- `protocol`, `version`, `id`
- `description?`
- `params?`, `returns?`
- `capabilities_required?`
- `execution_types`
- `execution_chains`

## 5. PackCard

推荐字段集合（最小可用）：

- `name`, `version`, `description?`
- `includes`: `{ protocol, version, chain_scope? }[]`
- `policy?`: approvals 与 hard constraints 摘要
- `token_policy?`: allowlist/解析策略摘要
- `providers?`: detect/quote provider allowlist 摘要
- `plugins?`: execution allowlist 摘要
- `overrides?`: actions override 摘要（数量/键列表可选）

## 6. 稳定性规则

- 数组必须稳定排序（协议+版本+id 的字典序；数值优先级字段按数值再按字符串）。
- 不应包含随机/不稳定字段（除 `created_at` 外）。
- hash 计算必须基于规范化结构（忽略 `created_at`）。
