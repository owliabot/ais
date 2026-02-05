# Handoff: Solana Support for AIS

> **Status:** Phase 1 Complete  
> **Last Updated:** 2026-02-05  
> **Commit:** `99d0215`

---

## Summary

Added foundational Solana support to the AIS SDK with zero external dependencies. Implemented SPL Token operations as the first working example.

---

## What's Done ✅

### SDK Module: `ts-sdk/src/execution/solana/`

| File | Purpose | Status |
|------|---------|--------|
| `constants.ts` | Program IDs, sysvars, instruction discriminators | ✅ |
| `types.ts` | TypeScript type definitions | ✅ |
| `base58.ts` | Base58 encoding/decoding | ✅ |
| `sha256.ts` | Pure JS SHA-256 (Web Crypto fallback) | ✅ |
| `pubkey.ts` | PublicKey creation and utilities | ✅ |
| `pda.ts` | Program Derived Address derivation | ✅ |
| `ata.ts` | Associated Token Account derivation | ✅ |
| `borsh.ts` | Borsh serialization (basic types) | ✅ |
| `accounts.ts` | Account resolution from AIS specs | ✅ |
| `builder.ts` | Instruction builder | ✅ |

### Example Protocol

- `examples/spl-token.ais.yaml` — SPL Token transfer, transfer-checked, approve, revoke

### Tests

- 28 new tests in `ts-sdk/tests/solana.test.ts`
- All 249 tests passing

### Documentation

- `docs/proposal-solana-support.md` — Full proposal with roadmap
- `ts-sdk/src/execution/solana/README.md` — Module documentation

---

## What's NOT Done ❌

### SDK Features

- [ ] **IDL parsing** — Anchor IDL support for arbitrary programs
- [ ] **`solana_account_read`** — Query execution type for reading account data
- [ ] **`http_query`** — For Jupiter/aggregator API calls
- [ ] **Lookup Tables** — Address Lookup Table support for v0 transactions
- [ ] **Priority fees** — Compute unit price configuration

### Spec Enhancements (AIS-2)

- [ ] Extended account `source` types (`query.*`, `sysvar:*`)
- [ ] Composite execution for Solana (multi-instruction)
- [ ] `solana_account_read` execution type definition

### Example Protocols

- [ ] Jupiter Swap (requires `http_query`)
- [ ] Raydium AMM (requires account read)
- [ ] Marinade Staking

---

## Known Limitations

1. **PDA bump values** — The SDK uses a simplified on-curve check. Derived addresses are correct, but bump values may differ from `@solana/web3.js`. For canonical bumps, use the official Solana SDK.

2. **No transaction building** — The SDK builds individual instructions. Actual transaction construction (blockhash, signatures) requires `@solana/web3.js`.

3. **No RPC calls** — SDK is offline-only. Checking balances, account existence, etc. requires external RPC integration.

---

## How to Continue

### Adding a New Protocol

1. Create `examples/<protocol>.ais.yaml` following the SPL Token pattern
2. Define `deployments`, `actions`, `queries`
3. Use `solana_instruction` execution type
4. Test with the SDK parser

### Adding IDL Support

1. Add `ts-sdk/src/execution/solana/idl.ts`
2. Parse Anchor IDL JSON format
3. Generate discriminators from instruction names
4. Map IDL types to Borsh serialization

### Adding Jupiter Support

1. Implement `http_query` execution type
2. Add Jupiter quote/swap API integration
3. Handle dynamic account lists from API response

---

## File Locations

```
ais/
├── docs/
│   ├── proposal-solana-support.md    # Full proposal
│   └── HANDOFF-solana-support.md     # This file
├── examples/
│   └── spl-token.ais.yaml            # SPL Token example
└── ts-sdk/
    ├── src/execution/solana/         # Solana module
    │   ├── README.md
    │   ├── index.ts
    │   ├── constants.ts
    │   ├── types.ts
    │   ├── base58.ts
    │   ├── sha256.ts
    │   ├── pubkey.ts
    │   ├── pda.ts
    │   ├── ata.ts
    │   ├── borsh.ts
    │   ├── accounts.ts
    │   └── builder.ts
    └── tests/
        └── solana.test.ts            # 28 tests
```

---

## References

- [Solana Cookbook](https://solanacookbook.com/)
- [SPL Token Program](https://spl.solana.com/token)
- [Anchor Framework](https://www.anchor-lang.com/)
- [Jupiter API](https://station.jup.ag/docs/apis/swap-api)
