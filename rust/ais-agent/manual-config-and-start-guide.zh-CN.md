# Rust ais-agent 手工配置与启动指南

日期：2026-03-16
适用目录：`/home/xcshuan/work/owlia/ais/rust/ais-agent`

## 1. 目标

这份文档只回答一件事：

- 如何把 `rust/ais-agent` 作为一个本地 HTTP harness 服务启动起来
- 并让 `ref/owliabot` 通过 `execution.aisAgent.transport.endpoint` 接入它

这不是一份 Owliabot 总体联调文档。Discord、LLM、Clawlet、skill 侧配置请看 Owliabot 的手册：

- [manual-discord-wallet-uniswap-integration-guide.zh-CN.md](/home/xcshuan/work/owlia/ais/ref/owliabot/docs/manual-discord-wallet-uniswap-integration-guide.zh-CN.md)

## 2. 系统边界

`rust/ais-agent` 在这条链路里的职责是：

- 接收 host 发来的 `ExecutionArtifact`
- 管理 run 生命周期、checkpoint、pause、resume、verify
- 暴露 HTTP transport 给 Owliabot harness client

它不负责：

- Discord 接入
- LLM provider
- Clawlet 钱包 daemon
- Uniswap / 1inch / CoW 之类外部 API 的业务编排

换句话说：

- Owliabot 决定“要做什么”
- `ais-agent` 负责“按 artifact 把执行控制跑起来”

补充一条当前 signer 边界语义：

- `submitted`
  - host 侧 signer/backend 已经把交易发到链上
  - `ais-agent` 只接 `tx_hash`，然后进入确认与验证
- `signed`
  - host 侧 signer/backend 只返回已签名交易
  - `ais-agent` 自己负责广播、确认与验证

当前这条 Owliabot 临时 signer 主路径走的是 `signed`，不是 `submitted`。

## 3. 外部前提

在启动 `ais-agent` 之前，至少准备好：

- 一个可用的 EVM RPC URL
- 可写的本地磁盘路径
- Rust 工具链

如果你要和 Owliabot 的手工联调指南保持一致，推荐直接使用：

- HTTP bind：`127.0.0.1:3200`
- SQLite 路径：`./var/ais-agent.db`

## 4. 最小配置文件

仓库里已经放了一份示例：

- [`ais-agent.manual-integration.example.yaml`](/home/xcshuan/work/owlia/ais/rust/ais-agent/ais-agent.manual-integration.example.yaml)

本地联调时可以直接从它复制一份，例如生成 `ais-agent.manual-integration.yaml`。

```yaml
service:
  instance_id: ais-agent-local

transport:
  http:
    enabled: true
    bind: 127.0.0.1:3200
  jsonl:
    enabled: false

storage:
  backend: sqlite
  path: ./var/ais-agent.db
  create_if_missing: true

providers:
  evm_rpc_url: ${AIS_AGENT_EVM_RPC_URL}

runtime_defaults:
  claim_lease_seconds: 60
  step_wall_clock_ms: 10000
  confirmation_poll_ms: 2000

observability:
  log_level: info
```

这份配置的目标很收敛：

- 只开 HTTP transport
- 只开 EVM RPC
- 用 SQLite 保留 run / checkpoint / event

注意：

- protocol package 的执行 allowlist 现在只保留在 Owliabot 侧
- `rust/ais-agent` 不再维护服务级 `protocol_packages.allow`
- 哪些 package 允许发起 harness run，由 Owliabot 的 `execution.protocolPackages.allow` 控制

## 5. 配置字段说明

### 5.1 `transport.http`

最关键的是：

- `transport.http.enabled: true`
- `transport.http.bind`

如果是和 Owliabot 本地联调，推荐固定成：

- `127.0.0.1:3200`

这样可以直接对应 Owliabot 侧：

- `execution.aisAgent.transport.endpoint: http://127.0.0.1:3200`

### 5.2 `storage`

本地联调不建议继续用纯内存。

推荐直接用：

- `backend: sqlite`
- `path: ./var/ais-agent.db`

这样重启后更容易排查 run/checkpoint 问题。

### 5.3 `providers`

当前最重要的是：

- `providers.evm_rpc_url`

如果你要跑 Base 上的 transfer / Uniswap V3，这个 RPC 必须可用。

### 5.4 `runtime_defaults`

默认值已经能跑，通常不需要一开始就调：

- `claim_lease_seconds`
- `step_wall_clock_ms`
- `confirmation_poll_ms`

### 5.5 执行白名单在哪里配

`rust/ais-agent` 不再维护 `protocol_packages.allow` 这类 package allowlist。

如果你要限制哪些 skill/package 可以走 harness，这层白名单只保留在 Owliabot 侧：

- `execution.protocolPackages.allow`

原因是：

- skill / package namespace 属于 Owliabot 边界
- `ais-agent` 只负责通用 execution artifact 执行控制
- Rust runtime 不再额外带一层 Owliabot package namespace gate

## 6. 环境变量与覆盖顺序

`ais-agent-cli` 的配置解析顺序是：

1. 内建默认值
2. `--config` 指定的 YAML 文件
3. 环境变量覆盖
4. CLI 参数覆盖

当前代码里已经支持这些环境变量：

- `AIS_AGENT_HTTP_BIND`
- `AIS_AGENT_SQLITE_PATH`
- `AIS_AGENT_EVM_RPC_URL`
- `AIS_AGENT_SOLANA_RPC_URL`
- `AIS_AGENT_CLAIM_LEASE_SECONDS`
- `AIS_AGENT_LOG_LEVEL`

如果只是本地联调，最少通常只需要：

```bash
export AIS_AGENT_EVM_RPC_URL="https://YOUR_EVM_RPC"
```

## 7. 启动命令

### 7.1 直接用 Cargo 启动

```bash
cd /home/xcshuan/work/owlia/ais/rust/ais-agent
mkdir -p ./var
export AIS_AGENT_EVM_RPC_URL="https://YOUR_EVM_RPC"
cargo run -p ais-agent-cli -- --config ./ais-agent.manual-integration.yaml daemon http --bind 127.0.0.1:3200
```

说明：

- `--config` 会加载 YAML
- `daemon http` 会进入 HTTP 服务模式
- `--bind` 会覆盖配置文件中的 `transport.http.bind`

### 7.2 先构建再启动

```bash
cd /home/xcshuan/work/owlia/ais/rust/ais-agent
cargo build -p ais-agent-cli
export AIS_AGENT_EVM_RPC_URL="https://YOUR_EVM_RPC"
./target/debug/ais-agent --config ./ais-agent.manual-integration.yaml daemon http --bind 127.0.0.1:3200
```

## 8. 最小验证

### 8.1 先看端口是否起来

```bash
curl -i "http://127.0.0.1:3200/runs/run-missing/events?after_event_seq=0&limit=1"
```

如果服务正常启动，你应该看到的是一个 HTTP 响应，而不是连接失败。

常见结果是：

- `404`
- 一个 JSON 错误体

这说明 transport router 已经起来了。

### 8.2 再对齐 Owliabot 侧 endpoint

Owliabot 配置里应当对应成：

```yaml
execution:
  aisAgent:
    enabled: true
    mode: harness
    transport:
      kind: http
      endpoint: http://127.0.0.1:3200
```

如果这两个值不一致，Owliabot 会直接连错地址。

## 9. 常见问题

### 9.1 为什么配置里写了 `bind`，命令里还要再写一次 `--bind`

因为手工联调时最容易出问题的就是端口不一致。把 `--bind 127.0.0.1:3200` 明确写在启动命令里，能减少排查成本。

### 9.2 为什么建议 SQLite，而不是内存模式

因为这条链路是手工联调，不只是 smoke test。你通常需要看 run、checkpoint、pause、restart 之后的行为，SQLite 更容易排查。

### 9.3 `ais-agent` 里需要配置 Discord、Clawlet、Uniswap API 吗

不需要。

这些不属于 `rust/ais-agent` 的职责边界。你真正需要保证的是：

- Owliabot 能访问 `ais-agent`
- `ais-agent` 能访问链 RPC

## 10. 和 Owliabot 联调时的推荐启动顺序

推荐固定为：

1. Clawlet
2. `rust/ais-agent`
3. `ref/owliabot`

Owliabot 侧更完整的联调步骤、配置和 Discord 手工测试脚本，请回到：

- [manual-discord-wallet-uniswap-integration-guide.zh-CN.md](/home/xcshuan/work/owlia/ais/ref/owliabot/docs/manual-discord-wallet-uniswap-integration-guide.zh-CN.md)
