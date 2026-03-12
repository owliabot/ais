# ais-agent-chain-shared

Purpose:
- hold chain-agnostic capability types, request/response DTOs, chain-family identifiers, and reflection contracts
- provide the shared low-level contract used by chain-family crates and higher-level driver routing

Public API entry points:
- chain capability types and traits:
  - `ChainFamily`
  - `ChainId`
  - `ChainCapability`
  - `CapabilityKind`
  - `ReadCapability`
  - `SimulationCapability`
  - `BroadcastCapability`
  - `ReceiptCapability`
  - `StateCapability`
- chain I/O DTOs:
  - `ReadRequest`
  - `SimulationRequest`
  - `BroadcastRequest`
  - `ReceiptQuery`
  - `StateQuery`
- reflection contracts:
  - `ReflectionArtifactKind`
  - `ReflectionRequest`
  - `ReflectionDriver`
  - `ReflectionDriverError`
  - `ReflectionDriverOutput`

Dependencies on workspace crates:
- `ais-agent-core`

Current implementation status:
- shared chain capability contracts implemented
- reflection request/trait contract implemented at the shared boundary

Known gaps:
- no persistence or network clients live here
- reflection artifact schemas are still intentionally minimal
