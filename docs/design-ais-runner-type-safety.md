# Design: `tools/ais-runner` 类型安全改造（去 `any`）

Status: Proposal  
Target: `tools/ais-runner/src/*`  
Motivation: 让 runner 从“能跑”升级到“可维护、可重构、可静态保障”。

---

## 1. 背景与问题

当前 runner 在核心路径使用了大量 `any`：

- `rg -n "\bany\b" tools/ais-runner/src | wc -l` = **143 行**
- 热点文件：
  - `run.ts`（36）
  - `executor-wrappers.ts`（29）
  - `executors.ts`（20）
  - `solver-wrappers.ts`（14）
  - `dry-run.ts`（12）

这会带来几个直接问题：

1. 核心流程签名模糊，改动风险高（例如 `maybeSolveAndRecheck(...)`）。
2. 运行时对象结构（`event`、`solver result`、`executor result`）缺少编译期保护。
3. 业务 bug 容易“编译通过但运行失败”（典型如 node kind / execution type 判定分支不全）。
4. 新增功能时难以判断影响面，代码 review 成本高。

---

## 2. 设计目标

### 2.1 目标

- 将 runner 核心执行链路（run / dry-run / wrappers / executors）中的 `any` 迁移到显式类型。
- 为动态加载的 `ts-sdk` 建立稳定的“类型化门面（typed facade）”。
- 把“不确定输入”放在边界层（`unknown + guard`），而非在核心逻辑层使用 `any`。

### 2.2 非目标

- 不改变 runner 的功能语义（仅类型化，不改业务行为）。
- 不引入兼容层（项目未上线，直接以清晰 API 为先）。

---

## 3. 核心方案

## 3.1 建立 SDK Typed Facade

新增 `tools/ais-runner/src/types/sdk.ts`，统一定义：

- `type RunnerSdk = typeof import('../../../ts-sdk/dist/index.js')`
- `type RunnerExecutionPlan = RunnerSdk['ExecutionPlan']`
- `type RunnerExecutionPlanNode = RunnerSdk['ExecutionPlanNode']`
- `type RunnerNodeReadinessResult = RunnerSdk['NodeReadinessResult']`
- `type RunnerResolverContext = RunnerSdk['ResolverContext']`
- `type RunnerExecutor = RunnerSdk['Executor']`
- `type RunnerSolver = RunnerSdk['Solver']`
- `type RunnerEngineEvent = RunnerSdk['EngineEvent']`

并将 `loadSdk()` 改为：

- `Promise<RunnerSdk>`（替代 `Promise<any>`）

这样可以在 runner 内部直接拿到 ts-sdk 的真实导出类型，不再裸用 `any`。

---

## 3.2 执行链路类型化（关键文件）

### `dry-run.ts`

将：

```ts
async function maybeSolveAndRecheck(
  sdk: any,
  solver: any,
  node: any,
  readiness: any,
  ctx: any
): Promise<any>
```

改为：

```ts
async function maybeSolveAndRecheck(
  sdk: RunnerSdk,
  solver: RunnerSolver,
  node: RunnerExecutionPlanNode,
  readiness: RunnerNodeReadinessResult,
  ctx: RunnerResolverContext
): Promise<RunnerNodeReadinessResult>
```

并同步收紧 `compileNode(...)` 输入输出类型。

### `run.ts`

- 给 `context/result/workflow/wsIssues/opts/baseExecutors` 全部落具体类型。
- `formatEvent` / `applyRunnerSideEffects` 使用 `RunnerEngineEvent` 联合类型分支。
- `synthWorkflow` / `toLitValueRefs` 使用 `Workflow` / `ValueRef` 相关类型（来自 sdk）。

### `executor-wrappers.ts`

- `Executor` 接口替换为 sdk 的 `Executor` 类型。
- `execute(...)` 返回值使用 `ExecutorResult | { need_user_confirm: ... }` 的显式联合。
- `classifyIo(...)` 输入改为 `ExecutionPlanNode`。

### `solver-wrappers.ts`

- 明确 `SolverResult`、`RuntimePatch`、`NodeReadinessResult` 类型。
- `computeCalculatedFieldPatches(...)` 返回类型从匿名联合提炼成命名类型。

### `executors.ts`

- 对 EVM/Solana signer、transport、provider 定义最小接口（避免 `any` 泄漏）。
- `createExecutorsFromConfig(...)` 返回 `Promise<RunnerExecutor[]>`。

---

## 3.3 边界层策略：`unknown + guard`

把“不可控输入”集中在边界，并做显式收窄：

- `JSON.parse(...)` 结果先 `unknown`，再用 guard / validator 校验。
- CLI / config / detect provider 外部输入都走同一套路。
- shim / third-party 边界也保持 `unknown` 输入，不向核心逻辑泄漏 `any`。

---

## 3.4 `any` 收敛目标

- 当前：`143` 行命中。
- 目标：
  - 核心执行链路（`run.ts`、`dry-run.ts`、`executor-wrappers.ts`、`solver-wrappers.ts`、`executors.ts`）**0 显式 `any`**。
  - 全仓 runner（`tools/ais-runner/src`）**0 显式 `any`**。

---

## 4. 分阶段实施计划

### Phase A：类型底座（低风险）

1. 新增 `src/types/sdk.ts`（typed facade）。
2. `sdk.ts` 改成返回 `RunnerSdk`。
3. `deps.ts` 的返回类型从 `any` 收紧为 `unknown` 或最小接口。

**Done when**:
- `loadSdk()` 不再返回 `any`。
- runner 能编译，现有测试全部通过。

### Phase B：核心执行链路（中风险）

1. 类型化 `dry-run.ts`（含 `maybeSolveAndRecheck`）。
2. 类型化 `run.ts` 主流程、事件分发、side effects。 
3. 类型化 `plan-print.ts`、`output.ts`。

**Done when**:
- `run.ts` / `dry-run.ts` 无显式 `any`。
- `npm -C tools/ais-runner test` 全通过。

### Phase C：wrappers + executors（中高风险）

1. 类型化 `executor-wrappers.ts`（`ExecutorResult` 联合清晰）。
2. 类型化 `solver-wrappers.ts`（`SolverResult` / patch 联合清晰）。
3. 类型化 `executors.ts`（provider/signer 最小接口）。

**Done when**:
- wrappers / executors 核心文件无显式 `any`。
- 集成测试通过，行为与当前一致。

### Phase D：边界清理与收尾（低风险）

1. `config-validate.ts` / `detect.ts` 的 `any` 改为 `unknown + guard`。
2. 统一更新 `tools/ais-runner/README.md`：补充类型设计约束与扩展指南。
3. 增加 1 组类型回归测试（至少覆盖 `maybeSolveAndRecheck` 与 event 分支）。

**Done when**:
- runner 总 `any` 行数降到目标范围。
- 文档与代码一致。

---

## 5. 风险与应对

### 风险 1：动态 import 与类型路径耦合

- 风险：`typeof import('../../../ts-sdk/dist/index.js')` 依赖 ts-sdk 已构建。
- 应对：这是 runner 的既有前置条件（README 已要求 ts-sdk/dist 存在），可接受。

### 风险 2：EngineEvent 联合过大，迁移成本高

- 风险：一次性强类型会改动较多分支。
- 应对：先在 `formatEvent` / `applyRunnerSideEffects` 做窄化函数，逐步替换。

### 风险 3：第三方库类型不完整

- 风险：`ethers` / `yaml` / shim 可能出现类型缺口。
- 应对：在边界使用“最小接口 + unknown + guard”，不让未收窄类型扩散到业务层。

---

## 6. 可追踪 TODO

Status legend:
- `[ ]` pending
- `[~]` in progress
- `[x]` done

- [x] TYPE-001 引入 `RunnerSdk` typed facade（`src/types/sdk.ts` + `sdk.ts`）
  - Done when: `loadSdk(): Promise<RunnerSdk>`，无 `Promise<any>`。

- [x] TYPE-002 类型化 `dry-run.ts`（含 `maybeSolveAndRecheck`）
  - Done when: 该文件无显式 `any`，函数签名使用 `ExecutionPlanNode/NodeReadinessResult/ResolverContext`。

- [x] TYPE-003 类型化 `run.ts`（主循环、event、side effects）
  - Done when: `run.ts` 显式 `any` 清零（或仅保留边界 <=2 处且有注释说明）。

- [x] TYPE-004 类型化 `executor-wrappers.ts`
  - Done when: wrapper 输入输出联合清晰，`classifyIo` 基于强类型节点。

- [x] TYPE-005 类型化 `solver-wrappers.ts`
  - Done when: `computeCalculatedFieldPatches` 返回命名联合类型，patch 类型来自 sdk。

- [x] TYPE-006 类型化 `executors.ts`
  - Done when: provider/signer/transport 使用最小接口，不再在核心逻辑使用 `any`。

- [x] TYPE-007 边界层清理（`detect.ts` / `config-validate.ts` / `output.ts`）
  - Done when: 外部输入全部 `unknown + guard`。

- [x] TYPE-008 文档同步（`tools/ais-runner/README.md`）
  - Done when: README 增加“类型边界与扩展约束”小节，和代码一致。

- [x] TYPE-009 回归验证
  - Done when: `npm -C tools/ais-runner test` 通过，且 `rg -n "\bany\b" tools/ais-runner/src` 显著下降到目标范围。
