# ais-agent-control

Purpose:
- define stable, transport-neutral host/runtime control-plane contracts
- keep command, event, and identifier types separate from runtime domain aggregates

Public API entry points:
- `audit`
- `commands`
- `events`
- `ids`
- `launch_spec`
- `ownership`
- `patch`
- `recovery`

Dependencies on workspace crates:
- none

Current implementation status:
- run command contract implemented
- run event contract implemented
- stable ids implemented
- `BeginRunCommand` now carries top-level `launch_spec` as the primary runtime ingress:
  - `prebuilt_fragment`
  - `reflection_request`
  - `execution_artifact`
- mutable commands now carry optional `expected_version` preconditions:
  - `checkpoint_seq`
  - `plan_epoch`
- recovery contract DTOs and enums now frozen for:
  - failure taxonomy
  - recovery disposition
  - recovery action kind / priority
  - `RunFailureContext`
  - `RecoverySuggestion`
- bounded patch contract DTOs now also exist for:
  - `PlanPatchSubmission`
  - `PlanPatchTarget`
  - `PlanPatchOperation`
  - `PatchOutcome`
- `RunCommand` now includes `submit_plan_patch`
- failed run events now carry typed `RunFailureCode` instead of free-form code strings
- run event contracts now also carry durable audit variants for:
  - `RecoveryAudit`
  - `GovernorDecision`
  - `PlanPatchAudit`
- runtime-owned durable audit contracts are now also frozen separately from host event streaming:
  - `RuntimeAuditRecord`
  - `RuntimeAudit`
  - recovery/governor/plan-patch payload records
  - first-pass cancellation/interruption/durable-commit audit payload records
- interruption/cancellation DTOs are now also frozen for the next control-plane phase:
  - `InterruptionClass`
  - `InterruptionState`
  - `CancelState`
  - `SideEffectPhase`
  - `RequestCancelRunCommand`
  - `RetryIntent`
- ownership DTOs are now frozen for the next host-collaboration phase:
  - `RunClaim`
  - `RunOwnershipSnapshot`
  - `ClaimTransitionKind`
  - `OwnershipErrorCode`
  - `RunClaimOwnerKind`
  - `RunClaimMode`
  - `RunClaimStatus`
- ownership command DTOs are now also frozen for explicit claim management:
  - `ClaimRunCommand`
  - `RenewRunClaimCommand`
  - `ReleaseRunClaimCommand`
  - `RunCommand::{ClaimRun, RenewRunClaim, ReleaseRunClaim}`
- interruption DTOs now distinguish:
  - `provider_timeout`
  - `provider_unavailable`
  - `confirmation_wait_timeout`
  - `verify_wait_timeout`
- recovery suggestions for step-oriented actions now carry explicit `retry_intent`
- fail-closed `RunFailed` events now include the full typed `RunFailureContext`
- in-code contract validation now exists for the first high-signal recovery invariants
- recovery contract validation now also includes shared `validate_recovery_contract(...)` checks for:
  - `RunFailureContext`
  - `RecoverySuggestion`
  - allowed-action membership
  - suggestion basis version alignment

Known gaps:
- no command/result schema evolution policy yet
- grouped durable commit execution now exists in runtime/store layers; this crate only freezes the command/audit DTO seam
- ownership command DTOs are now implemented in runtime/store layers for acquire / renew / release / guarded supersede; remaining work on this seam is narrower:
  - claim-specific runtime audit payload families are still not frozen here
- interruption/cancellation legality plus transport/restart proof are now implemented; remaining future work on this seam is narrower:
  - any explicit retry-command split beyond `RetryIntent`
  - automatic terminal-resolution policy for post-side-effect cancel flows
- protocol-specific transfer / Uniswap DTOs are no longer part of this crate's public API:
  - package-owned protocol shapes now live on the Owliabot/package side
  - runtime ingress stays centered on `launch_spec` and `execution_artifact`
