# AIS-1B: Pack — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

Packs select protocol specs and define policies (risk approvals, hard constraints, token policy, providers).

## 0. Strict fields and `extensions`

AIS 0.0.2 pack objects are **strict**:
- Unknown fields MUST be rejected.
- Extensions MUST live under an `extensions` object (free-form, implementation-defined).

```yaml
schema: "ais-pack/0.0.2"
meta: { name: "safe-defi-pack", version: "0.0.2" }
extensions:
  ui: { badge: "safe" }
```

```yaml
schema: "ais-pack/0.0.2"

meta:
  name: "safe-defi-pack"
  version: "0.0.2"
  description: "..."

includes:
  - protocol: "uniswap-v3"
    version: "0.0.2"
    source: "registry"                  # "registry" | "local" | "uri"
    uri: null                           # if source=uri
    chain_scope: ["eip155:8453"]        # optional

policy:
  approvals:
    auto_execute_max_risk_level: 2
    require_approval_min_risk_level: 3
  hard_constraints_defaults:
    max_slippage_bps: 50
    allow_unlimited_approval: false

token_policy:
  resolution:
    allow_symbol_input: true
    require_user_confirm_asset_address: true
    require_allowlist_for_symbol_resolution: true
  allowlist:
    - { chain: "eip155:8453", symbol: "USDC", address: "0x...", decimals: 6 }

providers:
  quote: { enabled: [ { provider: "uniswap-v3-quoter", chains: ["eip155:8453"], priority: 10 } ] }
  detect: { enabled: [ { kind: "best_quote", provider: "...", candidates: [100,500], priority: 10 } ] }

plugins:
  execution:
    enabled:
      - type: "my_plugin_exec_type"
        chains: ["eip155:1"]

overrides:
  actions:
    "uniswap-v3.swap-exact-in":
      hard_constraints:
        max_slippage_bps: 50
```

Notes:
- `providers.*` and `plugins.*` act as **allowlists** when a pack is active. See `specs/ais-1-capabilities.md`.
