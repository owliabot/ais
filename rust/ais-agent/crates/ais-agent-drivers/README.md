# ais-agent-drivers

Purpose:
- host driver implementations that map protocol, reflection, and API-native inputs into core action/effect objects
- keep protocol-specific logic out of the core runtime

Public API entry points:
- current modules:
  - `registry`
  - `standard`
  - `reflect`
  - `api_native`
- current public registry types:
  - `DriverRegistry`
  - `DriverCapability`
  - `DriverCandidate`
  - `DriverPathKind`
- current standard-driver types:
  - `StandardDriver`
  - `StandardDriverRequest`
  - `StandardDriverOutput`
  - `ActionGraphFragment`
- current reflection types:
  - `ReflectionDriver`
  - `ReflectionRequest`
  - `ReflectionDriverOutput`
  - `EvmAbiReflectionAdapter`
  - `SolanaIdlReflectionAdapter`
- current API-native types:
  - `ApiNativeAdapter`
  - `ApiNativeRequest`
  - `ApiNativeOutput`
  - `ApiNativeProviderKind`
  - `QuoteApiAdapter`
  - `RouteApiAdapter`
  - `DirectEnvelopeApiAdapter`
  - `NativeEnvelopeArtifact`

Dependencies on workspace crates:
- `ais-agent-core`
- `ais-agent-chain-shared`
- `ais-agent-evm`
- `ais-agent-solana`

Current implementation status:
- driver registry and routing heuristics implemented
- standard protocol driver interface implemented
- fragment-level live-binding hint contract is now frozen via `ais-agent-core::driver`
- standard-driver and reflection regressions now prove a shared fragment binding shape
- runtime now has a single fragment-attach path for:
  - standard-like outputs
  - EVM reflection outputs
- reflection driver interface is routed from `ais-agent-drivers`
- chain-specific reflection implementations now live in:
  - `ais-agent-evm`
  - `ais-agent-solana`
- API-native adapter interface implemented
- API-native normalization currently supports:
  - quote provider -> evidence
  - route provider -> evidence
  - direct-envelope provider -> runtime envelope + native envelope artifact + guarded action fragment + effect contract
- API-native direct-envelope output now binds into the same guarded runtime path as other families:
  - simulate
  - govern
  - signer
  - broadcast
  - verify
- raw-envelope execution is now also normalized through the runtime binder instead of bypassing the guarded path
- capability routing now produces ordered candidates across:
  - standard driver
  - reflection path
  - API-native path
  - raw envelope fallback
- direct-envelope normalization uses native chain types:
  - EVM envelope typing uses `alloy`
  - Solana envelope typing uses `solana_sdk`

Known gaps:
- no standard protocol drivers
- no concrete external API provider clients yet
- API-native normalization is still provider-agnostic; provider-specific schemas and adapters are not implemented
- runtime-bound execution is now proven for:
  - standard-like fragments
  - EVM reflection fragments
  - API-native direct-envelope fragments
  - raw-envelope binding
- routing is still heuristic and not yet backed by concrete live execution across all chain families
