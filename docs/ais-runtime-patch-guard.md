# AIS RuntimePatch Guard And Audit

本文件定义 AGT008 的运行时 patch 防护与审计约定。

## 1. RuntimePatch 权威结构

`RuntimePatch`:

- `op`: `set | merge`
- `path`: 非空字符串
- `value`: 任意 JSON 值
- `extensions?`: 可选扩展字段（对象）

SDK 提供：

- `RuntimePatchSchema`
- `validateRuntimePatch(...)`

## 2. Guard 安全默认（AGT008A）

默认允许根命名空间：

- `inputs.*`
- `ctx.*`
- `contracts.*`
- `policy.*`

默认禁止：

- `nodes.*`

可配置项（Runner/调用方传入）：

- `allow_roots: string[]`
- `allow_path_patterns?: string[]`（正则）
- `allow_nodes_paths?: string[]`（正则，显式放开 `nodes.*` 子路径）

SDK 提供：

- `DEFAULT_RUNTIME_PATCH_GUARD_POLICY`
- `checkRuntimePatchPathAllowed(...)`
- `buildRuntimePatchGuardPolicy(...)`

## 3. applyRuntimePatches Guard 行为

当 `guard.enabled = true`：

- 越权 path 会抛 `RuntimePatchError`（`code=guard_rejected`）
- 错误详情包含 `index/path/reason/patch/policy` 等结构化字段

返回值包含审计摘要：

- `audit.patch_count`
- `audit.applied_count`
- `audit.rejected_count`
- `audit.affected_paths`
- `audit.partial_success`
- `audit.hash`（sha256）

## 4. Runner Agent-Mode 审计事件（AGT008B）

Runner 在 `apply_patches` 命令路径强制启用 guard，并输出：

- `patch_applied`
- `patch_rejected`

事件字段至少包含：

- `command.id`（命令 ID）
- patch 摘要（计数、影响路径、`partial_success`）
- `summary.hash`（用于 trace/复盘对账）

同时保留 `command_accepted/command_rejected` 事件，便于上层保持命令级状态机。
