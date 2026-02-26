# AIS-1F: Capabilities — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

Capabilities declare engine features required to execute a spec (CEL profile, chain execution primitives, plugin/runtime features).

AIS is a component spec: protocol authors declare required capabilities, and an engine/agent host decides whether it can execute under current runtime + policy (pack).

## 1. Capability IDs

Capability IDs are opaque strings. This spec standardizes:

- a recommended naming scheme
- a core baseline set
- capability checks at validation/runtime boundaries

### 1.1 Syntax (recommended)

```
<namespace>:<name>
```

Examples:

- `cel:v1`
- `evm:read`, `evm:call`
- `solana:read`, `solana:instruction`
- `bitcoin:psbt`
- `plugin:offchain_apy_query`

Notes:
- Engines MUST treat capability strings as case-sensitive.
- Engines SHOULD ignore unknown IDs unless they are required by document fields.

## 2. Where capabilities are declared

### 2.1 Protocol-level required capabilities

```yaml
schema: "ais/0.0.2"
meta: { protocol: "...", version: "0.0.2" }
capabilities_required: ["cel:v1", "evm:read", "evm:call"]
```

Semantics:
- The protocol spec MUST NOT be executed unless all listed capabilities are supported by the engine.

### 2.2 Action/query-level required capabilities

Actions and queries MAY add extra requirements:

```yaml
actions:
  swap:
    capabilities_required: ["evm:call", "plugin:offchain_apy_query"]
```

Semantics:
- Required capabilities for execution = protocol-level ∪ action/query-level.

## 3. Core capability set (AIS 0.0.2)

Baseline for AIS core-chain engines:

- `cel:v1`
- `evm:read`, `evm:call`
- `solana:read`, `solana:instruction`
- `bitcoin:psbt` (PSBT compilation in-core; broadcasting is host-dependent)

## 4. Packs: provider/plugin gating

Packs are the policy boundary that can enable/disable optional providers/plugins even when engine capabilities exist.

### 4.1 Quote providers

Pack field:

```yaml
providers:
  quote:
    enabled:
      - provider: "uniswap-v3-quoter"
        chains: ["eip155:8453"]
        priority: 10
```

Semantics:
- When a pack is active, engines MUST treat `providers.quote.enabled` as an allowlist.
- Allowlisting does not imply executability; the host still needs a registered implementation.

### 4.2 Execution plugins (non-core execution types)

Pack field:

```yaml
plugins:
  execution:
    enabled:
      - type: "my_plugin_exec_type"
        chains: ["eip155:1"]
```

Semantics:
- `plugins.execution.enabled` is an allowlist for non-core `execution.type`.
- Engines MUST maintain a registry keyed by `execution.type`.
- Engines MUST reject any executable node whose `execution.type` is:
  - not allowlisted by active pack, or
  - not registered by host runtime.

## 5. Missing capability behavior

### 5.1 Load/validation phase

If any required capability is missing, engines MUST fail before producing runnable plans.

Recommended:
- return structured validation issues with explicit missing capability IDs.

### 5.2 Runtime phase

If a runtime path reaches an unavailable optional capability/plugin:

- Engines MUST NOT proceed with unsafe partial execution.
- Engines SHOULD surface a structured blocked/error state (`reason_code` + details).

## 6. Exposing capabilities to expressions (recommended)

Engines SHOULD expose supported capabilities in context:

```yaml
ctx:
  capabilities: ["cel:v1", "evm:read", "evm:call"]
```

This enables CEL guards and deterministic fallback logic without hardcoding engine internals.
