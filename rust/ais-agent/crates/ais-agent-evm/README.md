# ais-agent-evm

Purpose:
- host EVM-specific chain capability implementations and EVM reflection builders
- keep EVM details out of generic driver routing and out of non-EVM chain crates

Public API entry points:
- `EvmChainSurface`
- EVM capability adapters under:
  - `read`
  - `simulate`
  - `broadcast`
  - `receipt`
  - `state`
- reflection:
  - `reflect::EvmAbiReflectionAdapter`

Dependencies on workspace crates:
- `ais-agent-core`
- `ais-agent-chain-shared`

Current implementation status:
- EVM capability surface skeleton implemented
- EVM reflection adapter implemented in the EVM chain crate
- EVM reflection output now also emits fragment-level live-binding hints for runtime actuate dispatch
- EVM reflection output now emits a minimal runtime-bindable fragment:
  - simulate
  - actuate
  - verify
  with EVM live-binding hints suitable for `RuntimeDriverBinder`
- first real `alloy`-backed live read port implemented:
  - `read::live::EvmAlloyReadPort`
  - block-number observation
  - native-balance observation
  - storage-slot observation
  - stateless `eth_call`
  - ERC20 `balanceOf`
  - ERC20 `allowance`
  - generic contract-state read payloads
  - generic contract-state reads now also expose `decoded_u256` when the return payload is one 32-byte word
  - undecodable ERC20 amount payloads now fail closed instead of surfacing null-decoded observations, so runtime recovery can classify wrong-token evidence explicitly
- first real `alloy`-backed stateless simulate port implemented:
  - `simulate::live::EvmAlloySimulatePort`
  - `eth_call` simulation report
- first real `alloy`-backed live write/receipt ports implemented:
  - `broadcast::live::EvmAlloyBroadcastPort`
  - `receipt::live::EvmAlloyReceiptPort`
  - `eth_sendRawTransaction` -> tx hash submission
  - `getTransactionReceipt` -> observed/missing receipt view
  - confirmation depth derived from latest block number
- those receipt/read slices now also power runtime effect verification with:
  - normalized receipt payloads
  - live post-state reads
  - real `satisfied / violated / pending` verdicts on the EVM write path
- first real `alloy`/`anvil` simulation-state environment implemented:
  - `simulate::anvil::EvmAnvilSimulationEnv`
  - local or forked anvil spawn
  - `set_balance`
  - `set_storage_at`
  - ERC20 balance-slot patch helper
  - account impersonation
  - snapshot / revert
  - time controls
- live read regressions now use the `alloy` mock provider stack instead of pure local stubs

Known gaps:
- no live trace-call simulator yet
- reflection is still reduced-scope and not yet backed by full `alloy` ABI decoding
- the chain-shared `SimulationCapability` trait is still sync, so live anvil/stateful simulation currently lives as a direct EVM utility instead of a routed capability adapter
