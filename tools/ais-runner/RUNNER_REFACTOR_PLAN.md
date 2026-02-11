# AIS Runner (`tools/ais-runner/src`) 重构方案

## 目标

- 将目前 `src/*.ts` 的“平铺 + 超长文件”拆分为职责清晰的小模块，强关联代码放入同一子目录。
- 降低耦合：命令解析、配置、工作区加载/校验、执行器工厂、执行器 wrapper、输出格式化各自独立。
- 保持对外 API 稳定：`dist/*.js` 的现有入口与导出尽量不变（测试当前直接从 `../dist/*.js` import）。

## 现状与主要痛点（基于当前代码）

- `src/run.ts` 同时承担：CLI 路由、workspace 加载、workflow/action/query 三种模式编排、校验、执行器创建、事件打印、side-effect、资源清理。
  - 大量重复：workflow/action/query 分支里“broadcast 检查 -> createExecutorsFromConfig -> wrapper 链 -> runPlan -> destroy”逻辑几乎一致。
  - helper 混杂在同文件底部（`splitRef`/`parseJsonObject`/`synthWorkflow`/workspace 查找/`formatEvent`/side-effect 等）。
- `src/executor-wrappers.ts` 聚合了多种 wrapper（Policy/Broadcast/Preflight/StrictSuccess/CalculatedFields）以及一堆内部工具函数，边界不清晰。
- `src/executors.ts` 同时做：链路由、EVM transport、EVM signer（nonce 管理、gas 估算、fee data）、Solana connection、Solana signer、各种 config 解析工具。
- `src/config-validate.ts` 体量大且是典型“按字段分段验证”的代码，适合拆成小 validator。
- pack / workspace 相关工具有重复实现（`run.ts` 的 `findRequiredPackDocument` vs `detect.ts` 的 `findRequiredPack`）。

## 设计原则（拆分边界）

- **命令层只做编排**：输入是“解析后的 request + 已加载的 sdk/config/workspace”，输出是“退出码/打印内容/结果文件”。
- **纯逻辑尽量纯**：解析/校验/格式化函数不直接读写 `process`、文件系统或 stdout（方便测试/复用）。
- **依赖收敛**：wrapper/执行器只依赖 `Pick<RunnerSdkModule, ...>` 的最小子集（保持当前模式）。
- **文件移动最小化**：优先新增 `src/runner/`，逐步把实现挪进去；保留原文件作为薄的 re-export/forwarder，避免破坏 `dist/*` 导出路径。

## 目标目录结构（建议）

在 `tools/ais-runner/src/` 下新增 `runner/` 子目录，将“强关联但内部化”的 runner 编排代码集中管理：

```text
src/
  main.ts                      # 保持不动：bin 入口
  run.ts                       # 保持导出 run()；内部转发到 runner/router
  cli.ts                       # 可保持导出；内部转发到 runner/cli
  config.ts                    # 保持导出 loadRunnerConfig/RunnerConfig；内部转发到 config/*
  config-validate.ts           # 逐步拆分到 config/validate/*
  executors.ts                 # 保持导出 createExecutorsFromConfig/ChainBoundExecutor；内部转发到 runner/executors/*
  executor-wrappers.ts         # 保持导出 wrapper 类；内部转发到 runner/executors/wrappers/*
  detect.ts                    # 可逐步迁到 runner/detect/*
  output.ts                    # 可保持：输出评估与 json 写入
  runtime.ts                   # 可保持：类型 coercion
  types.ts                     # 可保持：ts-sdk 类型别名
  runner/
    router.ts                  # run() 的模式分发（workflow/action/query/help）
    commands/
      run-workflow.ts          # workflow 模式编排（importsOnly/strictImports/dry-run/broadcast...）
      run-action.ts            # action 模式编排（synthWorkflow + workflow 执行）
      run-query.ts             # query 模式编排（synthWorkflow + workflow 执行）
    engine/
      execute-plan.ts          # 统一的“构造 opts + runPlan + 打印事件 + destroy + outputs”
      side-effects.ts          # applyRunnerSideEffects（RUN-011）
      events.ts                # formatEvent / 事件筛选/打印策略
    workspace/
      load.ts                  # loadDirectoryAsContext / importsOnly 的 workspace 加载策略
      validate.ts              # workspace issues 过滤（relevantPaths）与 workflow validate
      resolve.ts               # findProtocolPathByRef / findRequiredPackDocument / collectRelevantWorkspacePaths
    workflow/
      synth.ts                 # synthWorkflow / toLitValueRefs / splitRef
    io/
      json.ts                  # parseJsonObject / stringifyWithBigInt（如需统一）
    executors/
      factory.ts               # createExecutorsFromConfig（入口）
      chain-bound.ts           # ChainBoundExecutor
      evm/
        transport-ethers.ts    # createEthersTransport + provider cleanup
        signer-private-key.ts  # createEvmSignerFromConfig + EthersBackedEvmSigner + nonce/fee/gas
      solana/
        connection.ts          # createSolanaConnection
        signer-keypair.ts      # createSolanaSignerFromConfig
      wrappers/
        strict-success.ts
        broadcast-gate.ts
        action-preflight.ts
        policy-gate.ts
        calculated-fields.ts
        write-preview.ts       # compileWritePreview + classifyIo + solana guard
        util.ts                # asRecord / uniqStrings / topoOrderCalculatedFields 等
```

说明：
- `src/*.ts` 继续作为“公开面”（dist 同路径），但逐步变薄，只负责 re-export 或调用 `src/runner/*`。
- `runner/` 目录下是内部实现，允许更激进的拆分与改名，但对外保持兼容。

## 文件拆分映射（从现有实现抽取到新模块）

### `src/run.ts` -> `src/runner/*`

- `run(argv)` 只保留“parseCliArgs + 路由到命令”的薄层，迁到 `runner/router.ts`。
- `workflow/action/query` 三段分支分别迁到：
  - `runner/commands/run-workflow.ts`
  - `runner/commands/run-action.ts`
  - `runner/commands/run-query.ts`
- 复用抽取（消除重复）：
  - “broadcast signer 检查”抽到 `runner/engine/execute-plan.ts` 或 `runner/engine/guards.ts`
  - “createExecutorsFromConfig + wrapper 链组装”抽到 `runner/executors/build.ts`（返回 `RunnerDestroyableExecutor[]`）
  - “runPlan 事件循环 + endedEarly + destroy + outputs”抽到 `runner/engine/execute-plan.ts`
- helpers 迁移：
  - `splitRef`、`synthWorkflow`、`toLitValueRefs` -> `runner/workflow/synth.ts`
  - `parseJsonObject` -> `runner/io/json.ts`
  - `collectRelevantWorkspacePaths` / `findProtocolPathByRef` / `findRequiredPackDocument` -> `runner/workspace/resolve.ts`
  - `formatEvent` -> `runner/engine/events.ts`
  - `applyRunnerSideEffects` -> `runner/engine/side-effects.ts`
  - `destroyExecutors` -> `runner/executors/destroy.ts` 或 `runner/engine/execute-plan.ts` 内部函数

### `src/executor-wrappers.ts` -> `src/runner/executors/wrappers/*`

- 每个 wrapper 单文件，文件名与类名一致，避免“一个文件装五个 class + 十几个 helper”。
- `compileWritePreview` 与 `classifyIo` 拆到 `wrappers/write-preview.ts`；其中 `compileEvmExecution/solana.compileSolanaInstruction` 的依赖通过 `WrapperSdk` 注入。
- 纯工具函数集中在 `wrappers/util.ts`：
  - `asRecord` / `uniqStrings` / `isEvmFailureStatus` / `topoOrderCalculatedFields` / `extractCalculatedDep` 等。
- 保持现有导出路径：
  - `src/executor-wrappers.ts` 继续 `export { ... } from './runner/executors/wrappers/...';`

### `src/executors.ts` -> `src/runner/executors/*`

建议把“链绑定/工厂/各链实现细节”拆开：

- `ChainBoundExecutor` -> `runner/executors/chain-bound.ts`
- `createExecutorsFromConfig` -> `runner/executors/factory.ts`
- EVM（ethers）相关：
  - `createEthersTransport` -> `runner/executors/evm/transport-ethers.ts`
  - `createEvmSignerFromConfig` / `EthersBackedEvmSigner` / `isNonceExpiredError` -> `runner/executors/evm/signer-private-key.ts`
- Solana 相关：
  - `createSolanaConnection` -> `runner/executors/solana/connection.ts`
  - `createSolanaSignerFromConfig` -> `runner/executors/solana/signer-keypair.ts`
- 通用小工具（`expandTilde`/`asRecord`/`getString`/`isNumberArray`/`toCommitment`/`toSendOptions`）按就近原则放在对应子模块，避免“utils 大杂烩”。

保持对外导出：
- `src/executors.ts` 继续导出 `ChainBoundExecutor` 与 `createExecutorsFromConfig`（测试 `router.test.js` 依赖 `dist/executors.js` 的 `ChainBoundExecutor`）。

### `src/config.ts` + `src/config-validate.ts` -> `src/config/*`

将配置作为“单独领域”而不是 runner 编排的一部分：

- 新增 `src/config/`（可选，若希望 `src/` 顶层更干净）：
  - `config/load.ts`：读取文件、`${ENV}` 展开、yaml parser 加载策略
  - `config/types.ts`：`RunnerConfig` 类型
  - `config/validate/index.ts`：`validateRunnerConfigOrThrow`
  - `config/validate/engine.ts` / `chains.ts` / `runtime.ts` / `signer.ts` / `send-options.ts`：按字段拆 validator
  - `config/validate/shared.ts`：`asNonEmptyString`/`asPositiveInt`/`isRecord`/`formatPath`
- `src/config.ts` 作为向后兼容面：re-export `loadRunnerConfig` 与 `RunnerConfig`。
- `src/config-validate.ts` 逐步变薄：最终仅 re-export `validateRunnerConfigOrThrow`。

## 关键复用点（建议新增的“骨架函数”）

1. `buildExecutors(sdk, config, { broadcast, yes, packDoc? })`
- 输入：基础 executors（来自 config）+ flags
- 输出：已经包好 wrapper 链的 executors
- 统一 wrapper 组合顺序，避免 `run.ts` 里散落三套逻辑

2. `executePlan({ sdk, context, workflow, plan, config, requestFlags, workspaceDocs? })`
- 负责：
  - broadcast 前置检查（`missingSignerChains`）
  - detect/solver/tracing/checkpoint_store 组装
  - `for await (ev of sdk.runPlan(...))` 循环
  - event 格式化与 side-effect
  - destroy executors（best-effort）
  - 若未提前结束：评估 outputs 并写文件

3. `loadWorkspaceAndWorkflow(...)`
- 统一 `importsOnly` vs `loadDirectoryAsContext` 的分支
- 统一“workspace issues 过滤到 relevantPaths”的逻辑

## 迁移步骤（低风险、可增量合并）

1. 先只“搬运 + 转发”，不改行为：
  - 新增 `src/runner/**` 目录与文件骨架
  - 将 `run.ts` 的 helper 与分支逻辑逐块挪过去
  - `src/run.ts` 保持 `export async function run(argv)` 的签名与行为
2. 抽取复用点（减少重复）：
  - workflow/action/query 三条路径共用 `executePlan` 与 `buildExecutors`
3. 执行器与 wrapper 拆分：
  - `executors.ts` / `executor-wrappers.ts` 变薄为 re-export
  - 运行 `npm test` 确认 `dist/*` 导出不破坏现有测试
4. 配置验证拆分：
  - 逐步把 `config-validate.ts` 拆到 `src/config/validate/*`，保持报错 path 文案一致（`config.test.js` 依赖具体 path 串）
5. 去重 pack/workspace 工具：
  - `findRequiredPack*` 合并成一个实现（供 `detect.ts` 与 runner 使用）

## 验证与回归点

- 单元测试：
  - `tools/ais-runner/test/router.test.js`（`ChainBoundExecutor` 导出/行为）
  - `tools/ais-runner/test/config.test.js`（报错 path 以及 env 展开）
  - `tools/ais-runner/test/runtime.test.js`（coercion 逻辑）
- 运行方式：
  - `cd tools/ais-runner && npm test`
- 行为一致性重点检查：
  - `--broadcast` gating 与 signer 缺失提示
  - `--imports-only` 的 workspace 加载与严格 imports 校验
  - `--checkpoint/--resume` 与 `engine_paused`/`error` 提前结束路径
  - `RUN-011` query_result side-effect（`runtime.query` 的兼容）

## TODO（可追踪）

- [x] RUNNER-REF-001 建立 `src/runner/router.ts`，让 `src/run.ts` 只做转发
- [x] RUNNER-REF-002 拆出 `runner/commands/run-workflow.ts` 并迁移 workflow 分支
- [x] RUNNER-REF-003 拆出 `runner/commands/run-action.ts` 并迁移 action 分支
- [x] RUNNER-REF-004 拆出 `runner/commands/run-query.ts` 并迁移 query 分支
- [x] RUNNER-REF-005 新增 `runner/engine/execute-plan.ts`，抽取 runPlan 循环/endedEarly/destroy/outputs
- [x] RUNNER-REF-006 新增 `runner/executors/build.ts`，统一 wrapper 链组装（Broadcast/Policy/Preflight/StrictSuccess/CalculatedFields）
- [x] RUNNER-REF-007 将 `executor-wrappers.ts` 拆到 `runner/executors/wrappers/*` 并保留原文件 re-export
- [x] RUNNER-REF-008 将 `executors.ts` 拆到 `runner/executors/{factory,chain-bound,evm/*,solana/*}` 并保留原文件 re-export
- [x] RUNNER-REF-009 把 `run.ts` 的 workflow/action/query 重复逻辑改为共用 `executePlan`（确保行为无变化）
- [x] RUNNER-REF-010 合并 pack/workspace 查找工具（`findRequiredPack*`）到 `runner/workspace/resolve.ts` 并复用到 `detect.ts`
- [x] RUNNER-REF-011 拆分 `config-validate.ts` 到 `src/config/validate/*`，保持错误 path 文案完全一致
- [x] RUNNER-REF-012 补齐/细化回归用例：broadcast disabled 的 `need_user_confirm.details`、calculated_fields topo 排序、policy gate 记忆化 key
