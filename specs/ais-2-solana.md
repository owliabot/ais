# AIS-2S: Solana Execution — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document defines Solana execution specs for `solana:*`.

## 0. Chain pattern matching (ExecutionBlock)

Solana actions/queries use the same ExecutionBlock matching as EVM (see `specs/ais-2-evm.md`).

Normative algorithm:

1) `execution[chain]` (exact CAIP-2)
2) `execution["solana:*"]`
3) `execution["*"]`
4) Otherwise error

## 1. `solana_instruction`

```yaml
type: solana_instruction
program: { ref: "contracts.token_program" }   # ValueRef resolving to base58 pubkey
instruction: "transfer_checked"               # program-specific
discriminator: { lit: "0x0c" }                # optional (8-byte or program-specific)
accounts:
  - name: "source"
    pubkey: { ref: "calculated.sender_ata" }
    signer: { lit: false }
    writable: { lit: true }
  - name: "authority"
    pubkey: { ref: "ctx.wallet_address" }
    signer: { lit: true }
    writable: { lit: false }
data:
  object:
    amount: { ref: "calculated.amount_atomic" }   # u64 string/BigInt
compute_units: { lit: "12000" }                   # optional
lookup_tables: { array: [ { lit: "..." } ] }      # optional
```

Notes:
- `accounts[].pubkey` SHOULD be a ValueRef, not a magic `source` string.
- For ATA/PDA, AIS 0.0.2 prefers explicit derived forms (to be specified) rather than ad-hoc string parsing.

## 2. `solana_read`

Minimal Solana RPC read spec for common on-chain queries (used with `until/retry` polling).

```yaml
type: solana_read
method: "getBalance"     # supported by core executor: getBalance|getTokenAccountBalance|getAccountInfo|getSignatureStatuses
params:
  object:
    address: { ref: "inputs.owner" }  # base58 pubkey (or use `pubkey`)
```

Notes:
- The core SDK/executor supports only a small set of methods (above). Other reads should be provided via plugins/executors.
- Outputs are written to `nodes.<id>.outputs` and can be used in CEL via `nodes.<id>.outputs.*`.
