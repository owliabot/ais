# Proposal: Solana Support for AIS Spec & SDK

> **Author:** Lay2  
> **Date:** 2026-02-05  
> **Status:** Draft — Awaiting Review

---

## 1. Executive Summary

本文档调研 AIS 规范及 TypeScript SDK 对 Solana 的支持方案。目标是让 Agent 能够通过统一的 AIS 接口与 Solana DeFi 协议（Jupiter、Raydium、Marinade 等）交互。

**关键差异点：**
- Solana 使用 **账户模型**（Account Model）而非 EVM 的状态模型
- 交易结构是 **Instruction** 而非 calldata
- 需要显式声明所有涉及的账户及其权限（signer/writable）
- 地址派生机制：**PDA（Program Derived Address）** 和 **ATA（Associated Token Account）**

---

## 2. Solana vs EVM 核心差异

| 维度 | EVM | Solana |
|------|-----|--------|
| **地址格式** | 0x + 40 hex (20 bytes) | Base58 (32 bytes public key) |
| **Chain ID** | 数字 (1, 8453, 42161...) | Genesis hash 或 `solana:mainnet` |
| **交易结构** | to + data + value | Instructions[] + signers[] |
| **状态存储** | 合约内部 mapping | 独立 Account（需显式传入） |
| **Token 标准** | ERC20 (approve/transferFrom) | SPL Token (delegate/transfer) |
| **账户派生** | 无 | PDA (seeds + program_id) |
| **Token 账户** | 钱包即账户 | ATA (需创建) |
| **Gas** | gas * gasPrice | Compute Units + priority fee |
| **并行执行** | 串行 | 并行（账户无冲突时） |

---

## 3. AIS-2 Spec 现状分析

AIS-2 已定义 `solana_instruction` 类型：

```yaml
execution:
  "solana:*":
    type: solana_instruction
    program: string                   # Program ID (base58)
    instruction: string               # Instruction name (from IDL)
    idl: string                       # Optional. IPFS URI to Anchor IDL
    discriminator: string             # Optional. 8-byte hex if no IDL
    
    accounts:
      - name: string
        signer: boolean
        writable: boolean
        source: string                # wallet | params.* | constant:* | derived
        derived: "ata" | "pda" | null
        seeds: [string]
        program: string
    
    mapping: object
    compute_units: integer
    lookup_tables: [string]
```

### 3.1 需要增强的部分

#### 3.1.1 账户来源扩展

当前 `source` 支持：
- `wallet` — 签名者钱包
- `params.*` — 从参数获取
- `constant:<address>` — 固定地址
- `derived` — 派生地址

**建议新增：**
- `query.*` — 从 query 结果获取（如 Jupiter route 返回的账户）
- `system` — 系统程序 (11111111111111111111111111111111)
- `sysvar:<name>` — Sysvar 账户 (rent, clock, etc.)
- `token_program` — SPL Token Program
- `associated_token_program` — ATA Program

#### 3.1.2 ATA 派生语法

```yaml
accounts:
  - name: user_token_account
    writable: true
    source: derived
    derived: ata
    seeds:
      wallet: "ctx.wallet_address"
      mint: "params.token.address"
    # ATA = findProgramAddress([wallet, TOKEN_PROGRAM, mint], ATA_PROGRAM)
```

#### 3.1.3 PDA 派生语法

```yaml
accounts:
  - name: pool_authority
    writable: false
    source: derived
    derived: pda
    seeds:
      - "pool"
      - "params.pool_id"
    program: "constant:RaydiumAMMv4..."
```

#### 3.1.4 Lookup Table 支持

Solana v0 transactions 支持 Address Lookup Tables 减少交易大小：

```yaml
execution:
  "solana:*":
    type: solana_instruction
    lookup_tables:
      - "constant:D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6"  # Jupiter LUT
      - "query.route.lookup_tables"  # 动态获取
```

---

## 4. SDK 实现方案

### 4.1 新增模块结构

```
ts-sdk/src/
├── execution/
│   ├── solana/
│   │   ├── builder.ts        # Solana instruction 构建
│   │   ├── accounts.ts       # 账户解析与派生
│   │   ├── pda.ts            # PDA 计算
│   │   ├── ata.ts            # ATA 派生
│   │   ├── idl.ts            # Anchor IDL 解析
│   │   ├── serializer.ts     # Borsh 序列化
│   │   └── index.ts
│   └── ...
├── schema/
│   └── solana.ts             # Solana 特定类型定义
```

### 4.2 核心 API 设计

```typescript
// 构建 Solana 指令
interface SolanaInstructionResult {
  programId: PublicKey;
  keys: AccountMeta[];           // { pubkey, isSigner, isWritable }[]
  data: Buffer;                  // Borsh 序列化的指令数据
  computeUnits?: number;
  lookupTables?: AddressLookupTableAccount[];
}

// 主入口
async function buildSolanaInstruction(
  protocol: Protocol,
  actionId: string,
  params: Record<string, unknown>,
  context: ResolverContext,
  options: SolanaBuilderOptions
): Promise<SolanaInstructionResult>;

// 账户解析
async function resolveAccounts(
  accounts: SolanaAccountSpec[],
  context: ResolverContext,
  connection: Connection
): Promise<AccountMeta[]>;

// PDA 派生
function derivePDA(
  seeds: (string | Buffer)[],
  programId: PublicKey
): [PublicKey, number];

// ATA 派生
function deriveATA(
  wallet: PublicKey,
  mint: PublicKey
): PublicKey;
```

### 4.3 依赖选择

**方案 A：最小依赖（推荐）**
- `@solana/web3.js` — 核心类型 (PublicKey, AccountMeta)
- 自实现 Borsh 序列化（轻量级）

**方案 B：完整依赖**
- `@solana/web3.js`
- `@coral-xyz/anchor` — IDL 解析
- `@solana/spl-token` — Token 操作

建议先用方案 A，按需引入 Anchor。

### 4.4 IDL 处理

Solana 程序通常用 Anchor 框架，IDL 描述指令和账户结构：

```typescript
interface AnchorInstruction {
  name: string;
  discriminator: number[];  // 8 bytes
  accounts: {
    name: string;
    isMut: boolean;
    isSigner: boolean;
    pda?: { seeds: PdaSeed[] };
  }[];
  args: { name: string; type: IdlType }[];
}

// 从 IDL 生成 discriminator
function getInstructionDiscriminator(name: string): Buffer {
  // sha256("global:" + name)[0..8]
}
```

---

## 5. 示例协议

### 5.1 SPL Token Transfer

基础示例 — 转账 SPL Token：

```yaml
schema: "ais/1.0"

meta:
  protocol: "spl-token"
  version: "1.0.0"
  name: "SPL Token"
  description: "Solana Program Library Token operations"

capabilities_required:
  - "cel_v1"
  - "solana_instruction"

deployments:
  - chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"  # Mainnet
    contracts:
      token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
      ata_program: "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"

actions:
  transfer:
    description: "Transfer SPL tokens to another wallet"
    risk_level: 2
    risk_tags: ["transfer"]

    params:
      - name: token
        type: asset
        description: "Token to transfer (mint address)"
      - name: amount
        type: token_amount
        description: "Amount to transfer"
        asset_ref: "token"
      - name: recipient
        type: address
        description: "Recipient wallet address"

    requires_queries:
      - "balance"

    calculated_fields:
      amount_atomic:
        expr: "to_atomic(params.amount, params.token)"
        inputs: ["params.amount", "params.token"]
      sender_ata:
        expr: "derive_ata(ctx.wallet_address, params.token.address)"
        inputs: ["ctx.wallet_address", "params.token"]
      recipient_ata:
        expr: "derive_ata(params.recipient, params.token.address)"
        inputs: ["params.recipient", "params.token"]

    execution:
      "solana:*":
        type: solana_instruction
        program: "token_program"
        instruction: "transfer"
        discriminator: "0x03"  # SPL Token transfer = 3
        
        accounts:
          - name: source
            writable: true
            signer: false
            source: "calculated.sender_ata"
          - name: destination
            writable: true
            signer: false
            source: "calculated.recipient_ata"
          - name: authority
            writable: false
            signer: true
            source: "wallet"
        
        mapping:
          amount: "calculated.amount_atomic"
        
        compute_units: 10000
```

### 5.2 Jupiter Swap

Jupiter 聚合器示例（核心逻辑）：

```yaml
schema: "ais/1.0"

meta:
  protocol: "jupiter"
  version: "6.0.0"
  name: "Jupiter Aggregator"
  description: "Solana DEX aggregator for optimal swap routing"
  homepage: "https://jup.ag"
  tags: ["dex", "swap", "aggregator"]

capabilities_required:
  - "cel_v1"
  - "solana_instruction"
  - "http_query"  # Jupiter API

deployments:
  - chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
    contracts:
      jupiter_v6: "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4"

supported_assets:
  - symbol: "SOL"
    name: "Solana"
    decimals:
      solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp: 9
    addresses:
      solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp: "So11111111111111111111111111111111111111112"
    tags: ["native", "wrapped"]
    
  - symbol: "USDC"
    name: "USD Coin"
    decimals:
      solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp: 6
    addresses:
      solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    tags: ["stable"]

queries:
  get-quote:
    description: "Get swap quote from Jupiter API"
    params:
      - name: input_mint
        type: address
        description: "Input token mint"
      - name: output_mint
        type: address
        description: "Output token mint"
      - name: amount
        type: uint64
        description: "Input amount in atomic units"
      - name: slippage_bps
        type: uint32
        description: "Slippage tolerance in basis points"
        default: 50

    returns:
      - name: out_amount
        type: uint64
        description: "Expected output amount"
      - name: price_impact_pct
        type: float
        description: "Price impact percentage"
      - name: route_plan
        type: object
        description: "Routing details"
      - name: swap_instruction
        type: object
        description: "Pre-built swap instruction"

    cache_ttl: 5

    execution:
      "solana:*":
        type: http_query
        url: "https://quote-api.jup.ag/v6/quote"
        method: GET
        params:
          inputMint: "params.input_mint"
          outputMint: "params.output_mint"
          amount: "params.amount"
          slippageBps: "params.slippage_bps"

  get-swap-instructions:
    description: "Get serialized swap transaction from Jupiter"
    params:
      - name: quote
        type: object
        description: "Quote from get-quote"
      - name: user_public_key
        type: address
        description: "User wallet address"

    returns:
      - name: swap_instruction
        type: object
        description: "Serialized instruction"
      - name: address_lookup_tables
        type: array
        description: "Required lookup tables"

    execution:
      "solana:*":
        type: http_query
        url: "https://quote-api.jup.ag/v6/swap-instructions"
        method: POST
        body:
          quoteResponse: "params.quote"
          userPublicKey: "params.user_public_key"

actions:
  swap:
    description: "Swap tokens via Jupiter aggregator"
    risk_level: 3
    risk_tags: ["mev_exposure", "slippage"]

    params:
      - name: token_in
        type: asset
        description: "Input token"
      - name: token_out
        type: asset
        description: "Output token"
      - name: amount_in
        type: token_amount
        description: "Input amount"
        asset_ref: "token_in"
      - name: slippage_bps
        type: uint32
        description: "Max slippage in basis points"
        default: 50
        constraints:
          min: 1
          max: 5000

    requires_queries:
      - "get-quote"
      - "get-swap-instructions"

    calculated_fields:
      amount_in_atomic:
        expr: "to_atomic(params.amount_in, params.token_in)"
        inputs: ["params.amount_in", "params.token_in"]

    hard_constraints:
      max_slippage_bps: "params.slippage_bps"

    execution:
      "solana:*":
        type: solana_instruction
        # Jupiter 返回序列化的指令，直接使用
        from_query: "get-swap-instructions"
        instruction_path: "swap_instruction"
        lookup_tables_path: "address_lookup_tables"
        compute_units: 400000

risks:
  - level: "warning"
    text: "Jupiter routes through multiple DEXes. Verify the route before confirming."
    applies_to: ["swap"]
  - level: "info"
    text: "Priority fee may be needed during high congestion."
    applies_to: ["swap"]
```

### 5.3 Raydium AMM Swap

原生 Raydium 示例（不走 Jupiter）：

```yaml
schema: "ais/1.0"

meta:
  protocol: "raydium-amm"
  version: "4.0.0"
  name: "Raydium AMM V4"
  description: "Raydium constant product AMM swaps"
  homepage: "https://raydium.io"
  tags: ["dex", "swap", "amm"]

capabilities_required:
  - "cel_v1"
  - "solana_instruction"

deployments:
  - chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
    contracts:
      amm_program: "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8"
      serum_program: "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"

actions:
  swap:
    description: "Swap tokens via Raydium AMM pool"
    risk_level: 3
    risk_tags: ["mev_exposure", "slippage"]

    params:
      - name: pool_id
        type: address
        description: "Raydium pool address"
      - name: token_in
        type: asset
        description: "Input token"
      - name: token_out
        type: asset
        description: "Output token"
      - name: amount_in
        type: token_amount
        description: "Input amount"
        asset_ref: "token_in"
      - name: min_amount_out
        type: token_amount
        description: "Minimum output amount"
        asset_ref: "token_out"

    requires_queries:
      - "pool-state"

    calculated_fields:
      amount_in_atomic:
        expr: "to_atomic(params.amount_in, params.token_in)"
        inputs: ["params.amount_in", "params.token_in"]
      min_out_atomic:
        expr: "to_atomic(params.min_amount_out, params.token_out)"
        inputs: ["params.min_amount_out", "params.token_out"]

    execution:
      "solana:*":
        type: solana_instruction
        program: "amm_program"
        instruction: "swap"
        discriminator: "0x09"  # Raydium swap instruction

        accounts:
          # Token program
          - name: token_program
            source: "system:token_program"
            signer: false
            writable: false

          # Pool accounts (from query or params)
          - name: amm
            source: "params.pool_id"
            signer: false
            writable: true
          - name: amm_authority
            source: derived
            derived: pda
            seeds: ["amm_authority"]
            program: "amm_program"
            signer: false
            writable: false
          - name: amm_open_orders
            source: "query.pool-state.open_orders"
            signer: false
            writable: true
          - name: amm_target_orders
            source: "query.pool-state.target_orders"
            signer: false
            writable: true
          - name: pool_coin_vault
            source: "query.pool-state.coin_vault"
            signer: false
            writable: true
          - name: pool_pc_vault
            source: "query.pool-state.pc_vault"
            signer: false
            writable: true

          # Serum market accounts
          - name: serum_program
            source: "serum_program"
            signer: false
            writable: false
          - name: serum_market
            source: "query.pool-state.serum_market"
            signer: false
            writable: true
          - name: serum_bids
            source: "query.pool-state.serum_bids"
            signer: false
            writable: true
          - name: serum_asks
            source: "query.pool-state.serum_asks"
            signer: false
            writable: true
          - name: serum_event_queue
            source: "query.pool-state.serum_event_queue"
            signer: false
            writable: true
          - name: serum_coin_vault
            source: "query.pool-state.serum_coin_vault"
            signer: false
            writable: true
          - name: serum_pc_vault
            source: "query.pool-state.serum_pc_vault"
            signer: false
            writable: true
          - name: serum_vault_signer
            source: "query.pool-state.serum_vault_signer"
            signer: false
            writable: false

          # User accounts
          - name: user_source_token
            source: derived
            derived: ata
            wallet: "ctx.wallet_address"
            mint: "params.token_in.address"
            signer: false
            writable: true
          - name: user_dest_token
            source: derived
            derived: ata
            wallet: "ctx.wallet_address"
            mint: "params.token_out.address"
            signer: false
            writable: true
          - name: user_owner
            source: wallet
            signer: true
            writable: false

        mapping:
          amount_in: "calculated.amount_in_atomic"
          minimum_amount_out: "calculated.min_out_atomic"

        compute_units: 200000

queries:
  pool-state:
    description: "Fetch Raydium pool state and associated accounts"
    params:
      - name: pool_id
        type: address
        description: "Pool address"

    returns:
      - name: coin_vault
        type: address
      - name: pc_vault
        type: address
      - name: open_orders
        type: address
      - name: target_orders
        type: address
      - name: serum_market
        type: address
      - name: serum_bids
        type: address
      - name: serum_asks
        type: address
      - name: serum_event_queue
        type: address
      - name: serum_coin_vault
        type: address
      - name: serum_pc_vault
        type: address
      - name: serum_vault_signer
        type: address

    cache_ttl: 60

    execution:
      "solana:*":
        type: solana_account_read
        account: "params.pool_id"
        parse_as: "raydium_amm_v4"  # Known account struct
```

---

## 6. 新增执行类型

### 6.1 `solana_account_read`

读取并解析 Solana 账户数据：

```yaml
execution:
  "solana:*":
    type: solana_account_read
    account: string                   # 账户地址
    parse_as: string                  # 解析格式 (idl_name | raw | json)
    idl: string                       # Optional IDL URI
    offset: number                    # Optional 起始偏移
    length: number                    # Optional 读取长度
```

### 6.2 `http_query`

用于调用链下 API（Jupiter、1inch 等聚合器）：

```yaml
execution:
  "*":
    type: http_query
    url: string
    method: "GET" | "POST"
    headers: object
    params: object                    # GET query params
    body: object                      # POST body
    response_path: string             # JSON path to extract
```

---

## 7. CEL 扩展函数

为 Solana 新增的内置函数：

| 函数 | 描述 |
|------|------|
| `derive_ata(wallet, mint)` | 计算 Associated Token Account 地址 |
| `derive_pda(seeds[], program)` | 计算 Program Derived Address |
| `base58_decode(str)` | Base58 解码为 bytes |
| `base58_encode(bytes)` | bytes 编码为 Base58 |

---

## 8. 实现路线图

### Phase 1: 基础支持 (1-2 weeks)
- [ ] SDK: `solana/` 模块骨架
- [ ] 账户解析器 (ATA/PDA 派生)
- [ ] Borsh 序列化（基础类型）
- [ ] 示例: SPL Token transfer

### Phase 2: 协议支持 (2-3 weeks)
- [ ] Jupiter swap 完整实现
- [ ] `http_query` 执行类型
- [ ] `solana_account_read` 执行类型
- [ ] 示例: Jupiter swap

### Phase 3: 高级功能 (1-2 weeks)
- [ ] Anchor IDL 解析器
- [ ] Lookup Table 支持
- [ ] Raydium 完整实现
- [ ] Marinade staking 示例

### Phase 4: 测试与文档 (1 week)
- [ ] 单元测试 (>80% coverage)
- [ ] 集成测试 (devnet)
- [ ] 文档更新

**总估时：5-8 weeks**

---

## 9. 开放问题

1. **IDL 托管**：Anchor IDL 放 IPFS 还是 repo 内？
2. **Priority Fee**：是否在 spec 中显式支持动态优先费？
3. **Versioned Transactions**：默认 legacy 还是 v0？
4. **错误处理**：如何映射 Solana 程序错误到 AIS 错误码？

---

## 10. 参考资料

- [Solana Cookbook](https://solanacookbook.com/)
- [Jupiter API Docs](https://station.jup.ag/docs/apis/swap-api)
- [Anchor Framework](https://www.anchor-lang.com/)
- [SPL Token Program](https://spl.solana.com/token)
- [CAIP-2 Chain IDs](https://github.com/ChainAgnostic/CAIPs/blob/main/CAIPs/caip-2.md)
