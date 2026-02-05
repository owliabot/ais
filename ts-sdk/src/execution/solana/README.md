# Solana Execution Module

Instruction building, account resolution, and serialization for Solana programs defined in AIS specs.

## File Structure

| File | Purpose |
|------|---------|
| `constants.ts` | Program IDs, sysvars, instruction discriminators |
| `types.ts` | Type definitions for Solana instructions and accounts |
| `base58.ts` | Base58 encoding/decoding (zero dependencies) |
| `pubkey.ts` | PublicKey creation and utilities |
| `sha256.ts` | SHA-256 for PDA derivation (Web Crypto + pure JS fallback) |
| `pda.ts` | Program Derived Address (PDA) derivation |
| `ata.ts` | Associated Token Account (ATA) derivation |
| `borsh.ts` | Borsh serialization for instruction data |
| `accounts.ts` | Account resolution from AIS specs |
| `builder.ts` | Main instruction builder |
| `index.ts` | Re-exports all APIs |

## Core Concepts

### Solana vs EVM

| Aspect | EVM | Solana |
|--------|-----|--------|
| Address | 20 bytes hex | 32 bytes Base58 |
| Transaction | to + data + value | Instructions + Accounts |
| State | Contract storage | Separate accounts |
| Token | ERC20 (approve) | SPL Token (delegate) |
| Derived | N/A | PDA, ATA |

### Account Resolution

Solana instructions require explicit account lists. Each account has:
- **pubkey**: The account address
- **isSigner**: Whether this account signs the transaction
- **isWritable**: Whether this account is modified

AIS specs define accounts with `source` expressions:

```yaml
accounts:
  - name: source
    source: "calculated.sender_ata"   # Expression
    signer: false
    writable: true
  - name: authority
    source: wallet                     # Signer's wallet
    signer: true
    writable: false
  - name: token_program
    source: "system:token_program"     # Well-known program
    signer: false
    writable: false
```

### Derived Accounts

**ATA (Associated Token Account):**
```yaml
- name: user_token_account
  source: derived
  derived: ata
  wallet: "ctx.wallet_address"
  mint: "params.token.address"
```

**PDA (Program Derived Address):**
```yaml
- name: pool_authority
  source: derived
  derived: pda
  seeds: ["pool", "params.pool_id"]
  program: "constant:RaydiumAMMv4..."
```

## API Usage

### Building Instructions from AIS Spec

```typescript
import { buildSolanaInstruction, SolanaResolverContext } from '@owliabot/ais-ts-sdk/execution/solana';

const ctx: SolanaResolverContext = {
  walletAddress: 'YOUR_WALLET_PUBKEY',
  chainId: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp',
  params: {
    token: { address: 'MINT_ADDRESS' },
    amount: '1000000',
    recipient: 'RECIPIENT_WALLET',
  },
  contracts: {
    token_program: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  },
  calculated: {
    amount_atomic: '1000000',
    sender_ata: 'DERIVED_ATA_ADDRESS',
    recipient_ata: 'RECIPIENT_ATA_ADDRESS',
  },
  query: {},
};

const result = buildSolanaInstruction(executionSpec, ctx);
// result.instruction: { programId, keys, data }
// result.preInstructions: optional create-ATA instructions
// result.computeUnits: suggested CU limit
```

### Direct SPL Token Transfer

```typescript
import { 
  buildSplTransfer,
  buildSplTransferWithAtaCreation,
  getAssociatedTokenAddressSync,
} from '@owliabot/ais-ts-sdk/execution/solana';

// Simple transfer (ATAs must exist)
const ix = buildSplTransfer(
  sourceAta,
  destAta,
  ownerWallet,
  BigInt('1000000')  // amount in atomic units
);

// Transfer with automatic ATA creation
const result = buildSplTransferWithAtaCreation(
  wallet,
  mint,
  recipient,
  BigInt('1000000'),
  { checked: true, decimals: 6 }
);
// result.preInstructions contains create-ATA instruction
// result.instruction is the transfer
```

### PDA Derivation

```typescript
import { findProgramAddressSync, toPublicKey } from '@owliabot/ais-ts-sdk/execution/solana';

const [pda, bump] = findProgramAddressSync(
  ['pool', poolId],  // seeds (strings or Uint8Array)
  programId          // program to derive from
);
```

### ATA Derivation

```typescript
import { getAssociatedTokenAddressSync } from '@owliabot/ais-ts-sdk/execution/solana';

const ata = getAssociatedTokenAddressSync(
  walletAddress,  // owner
  mintAddress     // token mint
);
```

### Borsh Serialization

```typescript
import { BorshWriter, BorshReader } from '@owliabot/ais-ts-sdk/execution/solana';

// Serialize
const writer = new BorshWriter();
writer.writeU8(3);              // instruction discriminator
writer.writeU64(BigInt(1000));  // amount
const data = writer.toBytes();

// Deserialize
const reader = new BorshReader(data);
const instruction = reader.readU8();
const amount = reader.readU64();
```

## Constants

### Program IDs

```typescript
import {
  SYSTEM_PROGRAM_ID,           // 11111111111111111111111111111111
  TOKEN_PROGRAM_ID,            // TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
  TOKEN_2022_PROGRAM_ID,       // TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb
  ASSOCIATED_TOKEN_PROGRAM_ID, // ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL
} from '@owliabot/ais-ts-sdk/execution/solana';
```

### SPL Token Instructions

```typescript
import { SPL_TOKEN_INSTRUCTIONS } from '@owliabot/ais-ts-sdk/execution/solana';

SPL_TOKEN_INSTRUCTIONS.Transfer       // 3
SPL_TOKEN_INSTRUCTIONS.Approve        // 4
SPL_TOKEN_INSTRUCTIONS.TransferChecked // 12
```

## Implementation Notes

- **Zero external dependencies**: Uses pure JS implementations (Base58, SHA-256, Borsh)
- **Web Crypto**: Uses `crypto.subtle` for SHA-256 when available
- **Type safety**: Full TypeScript types for all Solana structures
- **AIS integration**: Account resolution follows AIS spec expressions

## Dependencies

- `../schema/` — For Protocol/Action types
- `../resolver/` — For expression evaluation (when used with full SDK)
