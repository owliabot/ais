# AIS Runner CLI（示例与输出格式：text/json/jsonl）

本文对应 `AISRS-DOC-003`，描述 `ais-runner` 当前命令面与输出契约。

## 1. 命令总览

定义位置：`rust/ais-rs/crates/ais-runner/src/cli.rs:5`

- `ais-runner run plan --plan <file> [--config <file>] [--runtime <file>] [--dry-run]`
- `ais-runner run workflow --workflow <file> [--workspace <dir>] [--config <file>] [--runtime <file>] [--dry-run]`
  - execute 模式可附加 `--outputs <json-file>` 导出 workflow 顶层 `outputs`
- `ais-runner plan diff --before <plan> --after <plan>`
- `ais-runner replay [--trace-jsonl <file> | --checkpoint <file> --plan <file> --config <file>] [--until-node <id>]`

通用输出格式参数：

- `--format text`（默认）
- `--format json`

## 2. stdout 输出（text/json）

### 2.1 `run plan --dry-run`

`text`：来自 `dry_run_text`，包含 `summary`、`nodes`、`issues`。  
`json`：来自 `dry_run_json`，核心字段为 `nodes[]`、`issues[]`。

### 2.2 `run plan`（执行模式）

`text`（示例结构）：

```text
AIS run plan
status: paused|completed|stopped
paused_reason: none|...
resumed_from_checkpoint: true|false
iterations: N
events: N
command_accepted: N
command_rejected: N
completed_nodes: a,b,c
```

`json` schema：`ais-runner-run-plan/0.0.1`，字段：

- `status`
- `paused_reason`
- `resumed_from_checkpoint`
- `iterations`
- `events_emitted`
- `command_accepted`
- `command_rejected`
- `completed_node_ids`

实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:487`

### 2.3 `run workflow`

- `--dry-run` + `text`：工作区统计 + 编译出的 plan dry-run 文本。
- `--dry-run` + `json`：schema `ais-runner-run-workflow/0.0.1`，包含：
  - `workflow` / `workspace`
  - `documents`（protocols/packs/workflows/plans 计数）
  - `plan`（编译结果）
  - `dry_run`（dry-run 结构化结果）
  - `issues`
- 非 dry-run：走 `run plan` 执行路径，输出同 2.2。
- 若指定 `--outputs <json-file>`：
  - 在执行完成后，以最终 runtime 评估 workflow 顶层 `outputs`。
  - 写出文件 schema：`ais-runner-workflow-outputs/0.0.1`。
  - 文件结构：
    - `schema`
    - `outputs`

实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:136`

### 2.4 `plan diff`

- `text`：`plan diff: added=... removed=... changed=...`
- `json`：结构化 diff（含 `summary`）

实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:282`

### 2.5 `replay`

`json` schema：`ais-runner-replay/0.0.1`，字段：

- `status`（`completed|paused|reached_until_node`）
- `events_emitted`
- `completed_node_ids`
- `paused_reason`

`text`：同字段的人类可读格式。  
实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:639`

## 3. JSONL 输入输出边界

### 3.1 事件 JSONL（输出）

- 参数：`run plan|workflow --events-jsonl <path|- >`
- 行格式：每行一个 `EngineEventRecord`（schema `ais-engine-event/0.0.3`）
- 当值为 `-` 时，不输出 summary，stdout 直接写事件 JSONL。

编码函数：`encode_event_jsonl_line`  
实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:429`

### 3.2 Trace JSONL（输出）

- 参数：`--trace <path>`
- 行格式：每行一个经过默认 redaction 的事件 JSON。
- 编码函数：`encode_trace_jsonl_line`

实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:534`

### 3.3 Commands JSONL（输入）

- 参数：`--commands-stdin-jsonl`
- 输入源：stdin，每行一个 `EngineCommandEnvelope`（schema `ais-engine-command/0.0.1`）
- 空行忽略；解码失败返回 `commands stdin jsonl decode failed at line ...`

解码函数：`decode_command_jsonl_line`  
实现位置：`rust/ais-rs/crates/ais-runner/src/run.rs:669`

## 4. 最小示例

### 4.1 plan dry-run（json）

```bash
ais-runner run plan --plan ./plan.json --dry-run --format json
```

### 4.2 plan execute + 落盘 events/trace/checkpoint

```bash
ais-runner run plan --plan ./plan.json --config ./runner.yaml \
  --events-jsonl ./events.jsonl --trace ./trace.jsonl --checkpoint ./checkpoint.json \
  --format json
```

### 4.3 workflow execute + 导出 outputs

```bash
ais-runner run workflow --workflow ./workflow.yaml --workspace ./workspace \
  --config ./runner.yaml --outputs ./workflow.outputs.json --format json
```

### 4.4 replay trace（until node）

```bash
ais-runner replay --trace-jsonl ./trace.jsonl --until-node node-2 --format json
```

## 5. 错误约定（高频）

- 缺执行配置：`run plan`/`run workflow` 非 dry-run 需要 `--config`
- replay 输入缺失：必须给 `--trace-jsonl` 或 `--checkpoint`
- replay checkpoint 模式：额外必须给 `--plan` 与 `--config`
- 解析错误统一映射为 `RunnerError` 文本，便于 CI/脚本直接判定失败
