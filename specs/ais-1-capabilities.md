# AIS-1F: Capabilities — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

Capabilities declare engine features required to execute a spec (CEL profile, detect providers, multicall, permit, etc.).

AIS is a *component spec*: protocol authors declare required capabilities, and an engine/agent host decides whether it can safely execute under its current runtime + policy (pack).

## 1. Capability IDs

Capabilities are opaque strings, but this spec standardizes:

- a **recommended naming scheme** (for interoperability)
- a **core capability set** (minimum for AIS core chains)
- how capabilities interact with packs/providers/plugins

### 1.1 Syntax (recommended)

Use a namespace prefix followed by a capability name:

```
<namespace>:<name>
```

Examples:

- `cel:v1`
- `evm:read`, `evm:call`
- `solana:read`, `solana:instruction`
- `bitcoin:psbt`
- `detect:choose_one`, `detect:best_quote`, `detect:best_path`, `detect:protocol_specific`

Notes:
- Engines MUST treat capability strings as case-sensitive.
- Engines SHOULD ignore unknown capability IDs unless they are *required* by a document.

### 1.2 Scope taxonomy (informative)

To keep AIS minimal, capabilities should describe **engine features**, not “user configuration”.

Recommended scopes:

- **Engine** (pure feature): CEL profile, plan runner, trace, etc. (e.g. `cel:v1`)
- **Chain execution** (execution type support): EVM/Solana/Bitcoin core operations (e.g. `evm:call`)
- **Provider** (plugin/provider support): detect kinds/providers, quote providers, routing providers
- **Wallet / signing**: SHOULD be modeled as runtime configuration (executor returning `need_user_confirm`), not capability flags.

## 2. Where capabilities are declared

### 2.1 Protocol-level required capabilities

Protocol spec top-level:

```yaml
schema: "ais/0.0.2"
meta: { protocol: "...", version: "0.0.2" }
capabilities_required: ["cel:v1", "evm:read", "evm:call"]
```

Semantics:
- The protocol spec MUST NOT be executed unless all listed capabilities are supported by the engine.

### 2.2 Action-level required capabilities

Actions MAY add additional requirements:

```yaml
actions:
  swap:
    capabilities_required: ["detect:best_quote"]
```

Semantics:
- Required capabilities for executing an action = protocol-level ∪ action-level.

### 2.3 Detect-level capability requirements

`ValueRef.detect` MAY declare `requires_capabilities`:

```yaml
detect:
  kind: "best_quote"
  requires_capabilities: ["detect:best_quote"]
```

Semantics:
- Engines MUST check `requires_capabilities` before attempting to resolve a detect.
- This is the correct place to express “this detect kind is only valid if the engine supports X”.

## 3. Core capability set (AIS 0.0.2)

This set is the baseline for “AIS core chains” engines:

- `cel:v1`
- `evm:read`, `evm:call`
- `solana:read`, `solana:instruction`
- `bitcoin:psbt` (PSBT compiler; broadcasting is out of core)

Detect kinds (ValueRef layer):
- `detect:choose_one` (MUST be supported by all engines; deterministic fallback is allowed)
- `detect:best_quote`, `detect:best_path`, `detect:protocol_specific` (optional; provider-driven)

## 4. Packs: provider/plugin gating

Packs are the **policy boundary** that can enable/disable providers/plugins even if the engine supports them.

### 4.1 Detect providers

Pack field:

```yaml
providers:
  detect:
    enabled:
      - kind: "best_quote"
        provider: "uniswap-v3-fee-detect"
        chains: ["eip155:8453"]
        priority: 10
```

Semantics:
- A detect provider is identified by `(kind, provider)`.
- When a pack is active, engines MUST treat `providers.detect.enabled` as an **allowlist**.
- If `ValueRef.detect.provider` is set, engines MUST reject resolution unless the pair is enabled (and chain matches).
- If `ValueRef.detect.provider` is omitted, engines MAY pick an enabled provider for the `kind` (recommended: highest `priority` among matching `chains`).

### 4.2 Quote providers (for detect / workflows)

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
- Quote providers are engine-defined plugins used by detect providers and/or solver logic.
- When a pack is active, engines MUST treat `providers.quote.enabled` as an **allowlist**.

### 4.3 Execution plugins (non-core execution types)

AIS core execution types are fixed in AIS-2. Everything else is a plugin execution type.

Pack field:

```yaml
plugins:
  execution:
    enabled:
      - type: "my_plugin_exec_type"
        chains: ["eip155:1"]
```

Semantics:
- When a pack is active, engines MUST treat `plugins.execution.enabled` as an **allowlist** for plugin execution types.
- Engines MUST reject any plugin execution spec whose `execution.type` is not in the allowlist (and chain matches), even if a plugin is installed.

## 5. Engine behavior when capabilities are missing

### 5.1 At load/validation time (required)

If any `capabilities_required` (protocol or action) are not supported, engines MUST fail before producing a runnable plan.

Recommended behavior:
- raise a structured validation error (or `error` event) that lists missing capability IDs.

### 5.3 Exposing capabilities to expressions (recommended)

Engines SHOULD expose their supported capabilities to ValueRef/CEL as:

```yaml
ctx:
  capabilities: ["cel:v1", "evm:read", "evm:call"]
```

This allows:
- CEL guards to branch safely, and
- detect providers to introspect engine support when needed.

### 5.2 At runtime (detect / plugin)

If a detect/plugin requires capabilities that are missing:

- Engines MUST NOT broadcast transactions under partial support.
- Engines SHOULD surface the problem as a blocked node that can be handled by a solver:
  - either by applying patches (if possible), or
  - by producing `need_user_confirm` (manual input / approval), or
  - by failing with a clear error if no safe fallback exists.

This keeps AIS minimal: “AI constructs workflow”, while “engine enforces capability+policy safety”.
