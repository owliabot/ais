# ais-agent-solana

Purpose:
- host Solana-specific chain capability implementations and Solana reflection builders
- keep Solana details out of generic driver routing and out of non-Solana chain crates

Public API entry points:
- `SolanaChainSurface`
- Solana capability adapters under:
  - `read`
  - `simulate`
  - `broadcast`
  - `receipt`
  - `state`
- reflection:
  - `reflect::SolanaIdlReflectionAdapter`

Dependencies on workspace crates:
- `ais-agent-core`
- `ais-agent-chain-shared`

Current implementation status:
- Solana capability surface skeleton implemented
- Solana reflection adapter implemented in the Solana chain crate
- typed Solana minimal live-execution contract now frozen in `ais-agent-core` for:
  - observe
  - simulate transaction
  - broadcast signed transaction
  - signature-status verification
- typed Solana transaction requests now distinguish:
  - legacy transactions
  - v0 transactions with explicit lookup-table accounts
- runtime now has Solana binding resolution helpers mirroring the existing EVM binding helpers
- live Solana read port implemented for:
  - slot
  - account lamports
  - SPL token-account balance
  - account data
  - signature status
- live Solana simulate port implemented for:
  - legacy transactions
  - v0 transactions with lookup-table accounts
- live Solana broadcast port implemented for:
  - signed legacy transactions
  - signed v0 transactions with lookup-table accounts
- live Solana receipt port implemented for:
  - signature status polling
  - confirmation-depth projection
- live Solana transport now uses:
  - `solana-client` for RPC
  - `solana_sdk` concrete types for requests and transactions

Known gaps:
- reflection is still reduced-scope and not yet backed by full `solana_sdk` instruction assembly
- no full Solana state-reconcile layer beyond signature-status and simple read/simulate/broadcast slices yet
