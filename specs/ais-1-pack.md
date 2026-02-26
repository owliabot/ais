# AIS-1B: Pack — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

Packs select protocol specs and define policies (risk approvals, hard constraints, token policy, providers/plugins).

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
    mode: "safe"                         # optional: "safe" | "assist" | "yolo"
    auto_execute_max_risk_level: 2
    require_approval_min_risk_level: 3
    llm_may_approve_max_risk_level: 3    # optional
  protocol_install:
    mode: "safe"                         # optional: "safe" | "assist" | "yolo"
    allowed_sources: ["local_path", "registry_ref"]
    require_signature: true
    extensions:
      registry_allowlist: ["trusted/*"]  # optional, host-defined extension
      domain_allowlist: ["raw.githubusercontent.com"]
      trusted_publishers: ["trusted-org"]
  constraints:
    - id: "limit-slippage"
      effect: "hard_block"               # "hard_block" | "need_user_confirm"
      expr: "input.slippage_bps <= 50"
      message: "slippage exceeds policy limit"
    - id: "high-spend-confirm"
      effect: "need_user_confirm"
      expr: "uint(input.spend_amount) > 1000000"
      message: "spend amount exceeds soft threshold"

token_policy:
  resolution:
    allow_symbol_input: true
    require_user_confirm_asset_address: true
    require_allowlist_for_symbol_resolution: true
  allowlist:
    - { chain: "eip155:8453", symbol: "USDC", address: "0x...", decimals: 6 }

providers:
  quote: { enabled: [ { provider: "uniswap-v3-quoter", chains: ["eip155:8453"], priority: 10 } ] }

plugins:
  execution:
    enabled:
      - type: "my_plugin_exec_type"
        chains: ["eip155:1"]

overrides:
  actions:
    "uniswap-v3.swap-exact-in":
      risk_tags: ["dex", "swap"]
```

Notes:
- `providers.*` and `plugins.*` act as **allowlists** when a pack is active. See `specs/ais-1-capabilities.md`.

Additional plugin rules (normative):
- `plugins.execution.enabled` is an allowlist for **plugin execution types** (non-core `execution.type` values).
- Allowlisting does not imply executability: engines MUST also have a registered handler for the execution type.
- Packs MAY define different approval modes (e.g. “safe/assist/yolo”), but no mode may bypass:
  - protocol includes/chain scope, or
  - plugin handler registration, or
  - plugin execution type allowlisting.

Approval modes (normative):

- `mode = safe` (default if omitted):
  - `need_user_confirm` requires an explicit human confirmation.
- `mode = assist`:
  - an LLM agent MAY approve `need_user_confirm` automatically up to `llm_may_approve_max_risk_level` (if configured by the host/runner),
  - higher-risk confirmations still require a human.
- `mode = yolo`:
  - the host/runner MAY auto-approve all `need_user_confirm` (including via LLM),
  - but MUST still enforce hard blocks, allowlists, and handler registration.

Intent-mode overlay (normative):

- When run is initiated from `ais-agent-intent/0.0.1`, approval mode still follows `safe|assist|yolo`, with additional guard:
  - if intent sets `constraints.must_confirm=true`, transfer/write actions MUST require manual human confirmation regardless of mode.
- `assist` auto-approval MUST be bounded by `llm_may_approve_max_risk_level`.
- `yolo` MAY auto-approve confirms but MUST NOT bypass `hard_block`.
- hosts SHOULD emit stable reason_code values aligned with policy-gate intent-mode rules.

Threshold interaction (recommended):

- `auto_execute_max_risk_level` and `require_approval_min_risk_level` define the default confirmation boundary.
- In `assist`/`yolo` modes, hosts SHOULD still record an auditable confirmation summary/hash for each auto-approved action.

Protocol-install policy (normative):

- `policy.protocol_install.mode` controls dynamic protocol import/install governance:
  - `safe` (default): dynamic `remote_url` and `llm_generated` sources MUST be rejected.
  - `assist`: dynamic install MAY be allowed, but host SHOULD issue `need_user_confirm` unless source is explicitly low-risk per policy.
  - `yolo`: host MAY auto-approve install decisions, but MUST still enforce source/integrity constraints and auditable recording.
- `allowed_sources` restricts permitted `ProtocolSource.kind` values.
- `require_signature=true` requires a verifiable signature for non-local dynamic sources.
- `policy.protocol_install.extensions` MAY carry host-specific controls (for example registry/domain/publisher allowlists), but these are not part of the core decision contract.
- No mode may bypass:
  - execution handler registration checks, or
  - plugin/action allowlists defined by active pack.

CEL constraints (normative):

- `policy.constraints[]` is the canonical extensible constraint mechanism.
- Each constraint entry:
  - `id` (stable identifier)
  - `effect` (`hard_block` or `need_user_confirm`)
  - `expr` (CEL expression evaluated against normalized policy-gate input)
  - `message` (optional human-readable reason)
- Constraint evaluation order:
  1) `hard_block_fields` / missingness base rules (see policy-gate spec)
  2) CEL constraints in list order
- Conflict/precedence:
  - Any matching `hard_block` constraint wins over confirm/ok.
  - If no hard block matches but at least one `need_user_confirm` matches, output confirm.

## 1. Approval decision algorithm (normative)

This section standardizes how `policy.approvals` settings map an action `risk_level` to:

- whether confirmation is required, and
- who is allowed to approve (human vs LLM vs auto).

Definitions:

- `risk_level` is the action `risk_level` (integer 1..5).
- `mode` is `policy.approvals.mode` (`safe|assist|yolo`), default `safe`.

### 1.1 Configuration validity

Recommended validity rule (engines/runners SHOULD validate and error early):

- If both thresholds are present, `auto_execute_max_risk_level` MUST be strictly less than `require_approval_min_risk_level`.

Rationale:
- avoids ambiguous boundaries.

### 1.2 Determine whether confirmation is required

Let:

- `A = auto_execute_max_risk_level` (optional)
- `R = require_approval_min_risk_level` (optional)

Decision (normative):

1) If `R` is present and `risk_level >= R`, confirmation is required.
2) Else if `A` is present and `risk_level <= A`, confirmation is not required.
3) Else, confirmation is required (conservative default for “gap” or missing thresholds).

### 1.3 Determine the allowed confirmer

If confirmation is required:

- `mode = safe`: human confirmation is required.
- `mode = assist`: an LLM MAY auto-approve if `llm_may_approve_max_risk_level` is present and `risk_level <= llm_may_approve_max_risk_level`; otherwise human confirmation is required.
- `mode = yolo`: the host/runner MAY auto-approve (including via LLM), but MUST still enforce allowlists, handler registration, and hard blocks.

If confirmation is not required:

- the host/runner MAY proceed without any confirmation prompt.

### 1.4 Traceability (normative)

Whenever a confirmation is auto-approved by an LLM or yolo policy, hosts/runners MUST record auditable evidence in the event/command log.

Recommended approach:

- Emit a `need_user_confirm` event with a stable `confirmation_hash`, then immediately apply a `user_confirm` command whose `id` deterministically derives from that hash.

Note:
- The concrete `confirmation_hash` contract is defined by the policy gate spec (see `AISSPEC-005` in `docs/TODO-ais-agent-rust-v2.md`).
