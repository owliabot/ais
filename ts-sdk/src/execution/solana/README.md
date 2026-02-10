# Execution / Solana (AIS 0.0.2)

Solana execution compilation for AIS `solana_instruction`.

- Uses `@solana/web3.js` types (`PublicKey`, `TransactionInstruction`)
- Uses `@solana/spl-token` for SPL Token / ATA instructions where possible
- Performs **no network IO**

## File Structure

| File | Purpose |
|------|---------|
| `compiler.ts` | Compile `solana_instruction` into `TransactionInstruction` |
| `registry.ts` | `(programId, instruction)` compiler registry for protocol-specific encoders |
| `index.ts` | Re-exports Solana helpers + compiler |
| `README.md` | Module docs |

## Core API

### `compileSolanaInstruction()`

Compiles an AIS `solana_instruction` execution block into a Solana `TransactionInstruction`.

Supported `instruction` values (initial set):
- `transfer` (SPL Token)
- `transfer_checked` (SPL Token)
- `approve` (SPL Token)
- `create_idempotent` (Associated Token Account)
- Other values fall back to a **generic** builder that requires `data` to resolve to bytes (`0x..` or `Uint8Array`)

### `compileSolanaInstructionAsync()`

Use the async compiler if your execution spec contains async `{ detect: ... }` ValueRefs (e.g. routes/quotes).

```ts
import { solana } from '@owliabot/ais-ts-sdk';

const compiled = await solana.compileSolanaInstructionAsync(exec, ctx, {
  chain: 'solana:mainnet',
  params: { amount: '1000000' },
  detect, // optional
});
```

### Protocol-specific compilers (registry)

For non-standard programs (Anchor/Borsh/custom layouts), register an instruction compiler keyed by `(programId, instruction)` and pass it via `CompileSolanaOptions.compiler_registry`.

```ts
import { createContext, solana } from '@owliabot/ais-ts-sdk';
import { PublicKey, TransactionInstruction } from '@solana/web3.js';

const ctx = createContext();
const programId = new PublicKey('BPFLoaderUpgradeab1e11111111111111111111111');

const registry = solana.createDefaultSolanaInstructionCompilerRegistry();
registry.register(programId, 'custom_ix', ({ programId, accounts }) => {
  return new TransactionInstruction({
    programId,
    keys: accounts.map((a) => ({ pubkey: a.pubkey, isSigner: a.isSigner, isWritable: a.isWritable })),
    data: Buffer.from([0x01]),
  });
});

const compiled = solana.compileSolanaInstruction(
  {
    type: 'solana_instruction',
    program: { lit: programId.toBase58() },
    instruction: 'custom_ix',
    accounts: [{ name: 'a', pubkey: { lit: programId.toBase58() }, signer: { lit: false }, writable: { lit: false } }],
    data: { object: {} }, // registry compiler decides encoding
  },
  ctx,
  { chain: 'solana:mainnet', compiler_registry: registry }
);
```

```ts
import { createContext, solana } from '@owliabot/ais-ts-sdk';

const ctx = createContext();
ctx.runtime.ctx.wallet_address = 'BPFLoaderUpgradeab1e11111111111111111111111';
ctx.runtime.calculated.sender_ata = 'DYw8jCTfwHNRJhhmFcbXvVDTqWMEVFBX6ZKUmG5CNSKK';
ctx.runtime.calculated.recipient_ata = '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';
ctx.runtime.calculated.amount_atomic = '1000000';
ctx.runtime.contracts.token_program = solana.TOKEN_PROGRAM_ID.toBase58();

const compiled = solana.compileSolanaInstruction(
  {
    type: 'solana_instruction',
    program: { ref: 'contracts.token_program' },
    instruction: 'transfer',
    accounts: [
      { name: 'source', pubkey: { ref: 'calculated.sender_ata' }, signer: { lit: false }, writable: { lit: true } },
      { name: 'destination', pubkey: { ref: 'calculated.recipient_ata' }, signer: { lit: false }, writable: { lit: true } },
      { name: 'authority', pubkey: { ref: 'ctx.wallet_address' }, signer: { lit: true }, writable: { lit: false } },
    ],
    data: { object: { amount: { ref: 'calculated.amount_atomic' } } },
  },
  ctx,
  { chain: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp' }
);

compiled.tx; // TransactionInstruction
```
