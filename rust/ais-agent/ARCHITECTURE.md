# AIS-Agent Architecture

Purpose:
- define the single current architecture truth for `rust/ais-agent`
- keep implementation-oriented architectural guidance close to the code
- reduce drift across historical design documents under `/docs`

Status:
- authoritative for current crate boundaries and runtime collaboration model
- historical design docs in `/docs` remain useful as decision history, not as the primary implementation reference

Current closeout status:
- the intended product ingress is now `ExecutionArtifact` plus Owliabot package/runtime orchestration
- protocol-specific Rust public ABI for `transfer` / `uniswap` has been removed from `ais-agent-control` and `ais-agent-core`
- Owliabot shared `ais-agent` TS barrel no longer acts as a protocol-specific ingress surface
- an EVM-first acceptance baseline is now archived in:
  - [`acceptance-proof-matrix-ais-agent-final-goal-2026-03-16.md`](/home/xcshuan/work/owlia/ais/docs/ais-agent-design/acceptance-proof-matrix-ais-agent-final-goal-2026-03-16.md)
- Solana package/product proof remains explicitly deferred:
  - current Solana status is planner/runtime skeleton parity, not closeout-level protocol coverage

## Goal

`ais-agent` is not a general-purpose primary agent.

It is a chain-execution harness for a host agent such as OwliaBot:
- host agent owns open-ended reasoning, user interaction, external information gathering, and signer policy
- `ais-agent` owns execution control, guarded state transitions, checkpoint/recovery, and machine-verifiable effect closure

The target property is:
- host-driven
- recoverable
- auditable
- concurrency-safe
- chain-extensible

Current launch-contract direction:
- `LaunchSpecSubmission::ExecutionArtifact` is now part of the transport-neutral contract and is the intended product path
- `ExecutionArtifact` branch/value expression surfaces are aligned around `literal | ref | cel` and are expected to reuse `ais-agent-expr` rather than grow a second artifact-only expression DSL
- `ais-agent-runtime` now dispatches `ExecutionArtifact` planning by chain family into family-owned crates instead of hosting one monolithic planner
- protocol-specific Rust binder modules are no longer part of the product launch path
- protocol-specific Rust public DTO/effect-template modules are also no longer part of the public crate boundary

## Family Planner Matrix

| Family | Planner crate | Entry point | `observe` stage | `transaction` stage | Expected effects |
| --- | --- | --- | --- | --- | --- |
| EVM | `ais-agent-evm` | `artifact_planner::plan_execution_artifact` | yes | yes | yes |
| Solana | `ais-agent-solana` | `artifact_planner::plan_execution_artifact` | yes | yes | skeleton parity only |

Notes:
- branch and continuation semantics remain runtime-generic and are not family-owned
- Solana planner parity currently means typed observe/simulate/actuate/verify graph assembly, not full protocol/package breadth
- current closeout status is therefore:
  - EVM-first proof set archived
  - Solana deferred, not implicitly complete

## Control Model

The system is a two-loop controller:

- outer loop: host agent
  - submits missions
  - inspects state
  - provides evidence
  - provides signer decisions
  - decides whether to continue, retry, or cancel

- inner loop: `ais-agent`
  - turns mission + evidence + envelopes into guarded execution
  - advances one or more internal transitions
  - stops only at stable boundaries
  - persists recovery state before returning control

Control may transfer only at stable boundaries:
- `awaiting_evidence`
- `awaiting_signer`
- `awaiting_confirmation`
- `paused`
- `completed`
- `failed`
- `cancelled`

## Core Runtime Truth

The runtime truth is:
- `Mission`
- `ActiveRun`
- `CheckpointSnapshot`
- `EvidenceGraph`
- `ActionGraph`
- `ActuationRecord`
- `RunEventEnvelope`

Important rule:
- checkpoints are persistence and recovery boundaries
- `ActiveRun` is the currently loaded hot state
- durable execution truth is now frozen as:
  - `Mission`
  - `CheckpointSnapshot`
  - `RunEventEnvelope`
  while `ActiveRun` remains the currently loaded aggregate

## Durable Persistence Contract

The durable storage contract is frozen in `ais-agent-runtime` before the first
concrete backend lands.

Current repository boundaries:
- `MissionRepository`
  - durable mission archive keyed by run identity
- `RunCatalogRepository`
  - durable latest-pointer summary for one run
- `CheckpointArchive`
  - append-only checkpoint history
  - archive entry kinds:
    - `boundary`
    - `progress`
    - `side_effect`
- `EventArchive`
  - append-only `RunEventEnvelope` history
  - cursor reads remain:
    - `run_id`
    - `after_event_seq`
    - `limit`
- `RuntimeAuditArchive`
  - append-only runtime-owned audit history distinct from host-visible event polling
  - current frozen payload families are:
    - recovery
    - governor decision
    - plan patch
    - cancellation
    - interruption
    - durable commit

Grouped durable truth contract:
- runtime persistence now also freezes a write-set DTO for one logical durable success path:
  - `DurableMutationUnit`
  - `DurableMutationKind`
  - mission/checkpoint/event/catalog/signer/audit write members
- runtime persistence also now exposes a runtime-owned executor seam for those grouped units:
  - `DurableMutationExecutor`
  - `DurableCommitReceipt`
  - `DurableCommitError`
  - `LinearDurableMutationExecutor` as the current reference implementation
- current validation rejects:
  - run-id drift across write-set members
  - non-monotonic event or audit batches
  - catalog/checkpoint tail mismatch
  - malformed `run_begin` units without mission-insert semantics
- backend-native grouped commit execution now exists in SQLite for mission/checkpoint/event/catalog/wait-state/audit writes through `DurableMutationExecutor`
- runtime host-service success paths now build `DurableMutationUnit` and commit through that executor seam for begin, step, host mutations, and terminal cancel flows
- scheduler-produced checkpoint writes are now captured first and only then committed with event/catalog/wait-state/audit truth as one host-visible durable success unit

Current SQLite observability / retention shape:
- the durable schema is now fixed around:
  - `runs`
  - `run_inputs`
  - `run_events`
  - `run_audits`
  - `run_checkpoints`
  - `run_wait_states`
  - `run_claim_history`
  - `maintenance_journal`
  - `store_maintenance_state`
- `ais-agent-store-sqlite` is the owner of:
  - repository adapters
  - inspect queries
  - retention / purge / vacuum maintenance

Frozen durability cuts:
- `begin_run`
  - mission + initial checkpoint + initial events must become one durable truth unit
- stable-boundary return
  - checkpoint and host-visible events must already be durable before control returns
- side-effect cut
  - irreversible external effects like successful broadcast must flush a dedicated durable checkpoint archive entry instead of living only in hot memory
  - the scheduler now forces this cut immediately when a transition enters `awaiting_confirmation` with:
    - `pending_confirmation_id`
    - a matching `BroadcastSubmitted` actuation record carrying the chain-native tx id
  - the resulting latest checkpoint stays typed as `side_effect` instead of being overwritten by a duplicate boundary append on the same return path

Current durable-first write wiring:
- `begin_run`
  - mission archive write
  - initial checkpoint archive append
  - initial event archive append
  - run-catalog latest-pointer upsert
  - only then hot-cache insert and success return
- mutating host commands
  - durable checkpoint/event/catalog writes happen before hot runtime cache save
  - host-side durable mutations now also advance checkpoint identity, not just hot revision, for:
    - `submit_evidence`
    - `submit_envelope`
    - `submit_signer_resolution`
  - `RunCatalogRepository` tracks latest checkpoint seq, latest event seq, lifecycle status, boundary kind, and revision
  - failure-injection regressions prove durable write errors do not return success and do not front-run the hot cache save

Current recovery wiring:
- `restore_active_run(...)`
  - loads mission from `MissionRepository`
  - loads latest checkpoint from `CheckpointRepository`
  - latest checkpoint selection now follows monotonic checkpoint truth (`checkpoint_seq`, then `plan_epoch`) instead of raw append order
  - loads durable wait-state truth from `RunWaitStateStore`
  - reconstructs `ActiveRun` from durable truth instead of requiring a preexisting hot runtime copy
  - now fails closed when a checkpoint claims `awaiting_confirmation` but is missing:
    - `pending_confirmation_id`
    - effect-contract entries required by pending verify nodes
- `RuntimeHostService`
  - prefers the hot `RunRepository`
  - falls back to durable restore on cache miss
  - repopulates the hot cache after successful restore
  - `inspect_run` is a lighter read path:
    - prefers hot runtime state when present
    - otherwise projects directly from durable mission + latest checkpoint
    - does not force hot-cache rehydration for archive-backed reads
    - when a durable run exists but the in-memory host-session link was lost, `inspect_run` is also the deterministic relink seam:
      - it may attach the run to the requesting `HostSessionId` only if no live link currently exists in the process
      - it does not steal a run from a different live session; that still fails as `session_run_mismatch`
      - mutating commands fail closed as `session_relink_required` until this relink happens
  - `list_events(...)` now reads from durable `EventArchive`
  - restored hot runtimes realign `event_seq` from the archive's latest durable event so post-restart emissions remain monotonic

Confirmation / verify resume truth:
- side-effect checkpoints must carry enough runtime-owned truth to continue receipt/status polling and effect verification:
  - confirmation id (`tx_hash` / signature)
  - pending verify node linkage through action-graph refs
  - checkpoint-owned `effect_contracts`
  - any already-attached pre-state evidence required by verify refs
- crash/restart regressions now prove that both:
  - EVM broadcast success
  - Solana broadcast success
  can be restored from durable mission + latest side-effect checkpoint alone and then finished through verify without reusing stale hot runtime state

## Execution Model

The runtime executes an `ActionGraph` whose nodes are:
- `Observe`
- `Derive`
- `Simulate`
- `Actuate`
- `Verify`
- `Recover`

The runtime advances through:
- `StepOnce`
- `StepScheduler::step_until_boundary(...)`

This means:
- internal work may take multiple local transitions
- host-visible control still returns only at stable boundaries or budget exhaustion
- runtime observability now exposes scheduler/service truth seams through `tracing`, including:
  - transition application
  - stop reason / stable-boundary exit
  - side-effect durability cut decisions
  - restore-source decisions
- durable write failures

## Service Configuration Boundary

Service/deployment configuration is a runtime wiring seam, not the product truth for one run.

That means:
- storage / transport / provider / observability / timeout defaults belong in service config
- recipient / amount / slippage / LP parameters belong in mission submission and evidence

Current implementation note:
- the current service config still carries per-family feature toggles such as transfer enablement
- treat those toggles as bounded rollout guards, not as the long-term capability model

Provider wiring is now modeled by canonical chain scope first:

- runtime config should prefer provider entries keyed by canonical chain scope such as:
  - `eip155:8453`
  - `eip155:1`
  - `solana:mainnet`
- each chain entry owns its connection contract:
  - EVM: `http_url`, optional `ws_url`
  - Solana: `http_url`, optional `ws_url`
- live binding resolution requires exact chain entry lookup

Implication:

- launch-spec chain scope remains the source of truth for which chain a run targets
- runtime wiring resolves that chain scope into a concrete provider binding
- if no exact entry exists, runtime fails closed
- if the resolved provider family does not match the artifact chain family, runtime fails closed

Forward-looking constraint:
- do not keep expanding `ais-agent` by adding one hard-coded feature flag or action-family-specific branch per new capability
- new execution slices should prefer:
  - artifact-first launch contracts
  - generic graph synthesis / branch evaluation
  - generic runtime wiring over feature-specific special cases
- the current package allowlist is only an enablement boundary; protocol-specific launch seams are no longer part of the architecture

## Launch Contract Boundary

Current launch-spec families:

- `prebuilt_fragment`
- `reflection_request`
- `execution_artifact`

Intended role of each:

- `execution_artifact`
  - target product contract
  - host provides protocol-resolved execution semantics
  - runtime owns graph synthesis, branch evaluation, verification, continuation, and recovery
- `prebuilt_fragment`
  - low-level escape hatch for explicitly authored graph fragments
- `reflection_request`
  - reserved / not yet implemented

What `execution_artifact` is for:

- package-owned transaction candidates
- stage-bound expected effects that compile into runtime-owned effect verification
- generic branch predicates and actions
- exported verified outputs
- continuation points for downstream artifact stages

What `execution_artifact` is not:

- a protocol-specific DTO surface
- a per-protocol binder registry
- a requirement for Owliabot to hand-author full runtime node graphs

## Host Collaboration Contract

The host-facing command plane is intentionally small:
- `begin_run`
- `inspect_run`
- `step_run`
- `submit_evidence`
- `submit_envelope`
- `submit_plan_patch`
- `submit_signer_resolution`
- `cancel_run`

For `begin_run`, the current architectural target is:

- host submits `mission`
- host submits `launch_spec`
- `launch_spec.kind = execution_artifact` should become the primary product path
- host/outer-agent remains responsible for protocol semantics, external quote/evidence gathering, and staged continuation artifact construction
- `ais-agent` remains responsible for guarded execution, branch judgment, signer/confirmation, verification, output export, and recovery

Signer boundary semantics are now explicit:

- `submit_signer_resolution(decision = submitted, tx_hash = ...)`
  - means the host-side backend already broadcast the transaction
  - `ais-agent` skips local broadcast and moves directly into confirmation waiting
- `submit_signer_resolution(decision = signed, signed_payload = ...)`
  - means the host-side backend only signed
  - `ais-agent` remains the owner of local broadcast, confirmation polling, and verify

This distinction exists so host-side signer backends can evolve independently:

- temporary or hardened signer-only backends can use `signed`
- signer-and-send backends can continue using `submitted`

The host-facing read plane is:
- `InspectSnapshot`
- `PauseBundle`
- event polling over `RunEventEnvelope`

Current host read wiring:
- `inspect_run`
  - hot runtime first
  - durable mission + latest checkpoint fallback
  - host-direct projector entry points now share the same checkpoint-recovery classifier as runtime-backed inspect, instead of maintaining a weaker fallback recovery path
  - runtime now validates the derived recovery contract before projecting host-visible inspect/pause views; malformed recovery truth fails closed as `recovery_contract_invalid`
- event polling
  - durable `EventArchive` first
  - HTTP transport now classifies polling failures by host error class instead of flattening them all to `404`
  - no longer depends on hot `ActiveRun.event_log`
  - empty archive is treated as an empty batch when the run exists
  - the same stream now also carries recovery / audit truth for:
    - recovery classification and suggestions
    - governor decisions
    - plan patch submitted / applied / rejected

Current restart proof:
- in-memory restart regressions cover runtime control-flow recovery boundaries
- SQLite-backed restart regressions now prove:
  - begin-run grouped durable truth replays after restart through `inspect_run` and durable event polling
  - inspect after restart
  - inspect-driven relink restores host operability after session-store loss
  - patch-required recovery can continue from durable truth after restart
  - event polling after restart
  - awaiting-evidence resume through `RuntimeHostService`
  - awaiting-signer resume from durable mission/checkpoint/signer archives
  - confirmation-path `cancel_pending` survives restart as durable checkpoint truth
  - retry-ready confirmation timeout interruption survives restart as durable checkpoint truth
  - signer denial survives restart and clears durable signer truth once the denial boundary is consumed

Current transport proof for execution-control semantics:
- JSONL and HTTP preserve runtime-owned interruption/cancel fields without transport-owned reinterpretation
- real-runtime transport e2e now proves:
  - confirmation-wait `request_cancel_run -> cancel_pending`
  - restart relink before post-restart mutation
  - recovery patch/envelope loops through live host commands
- adapter-level wire regressions now also prove:
  - retry-ready inspect payloads round-trip unchanged
  - await-user-input pause payloads round-trip unchanged

Current grouped-commit failure proof:
- SQLite executor rollback is backend-native and atomic for mission/checkpoint/event/catalog/signer/audit writes
- reference in-memory executors remain linear rather than transaction-native, so failure-injection proofs there focus on:
  - fail-closed host errors
  - durable-truth-over-hot-cache on follow-up recovery
  - restart-safe continuation from whatever durable checkpoint truth did land

The transport layer must not invent semantics.
It only adapts the host/runtime contract over:
- JSONL
- HTTP
- CLI shell

Current transport adaptation status:
- JSONL / HTTP now carry recovery-aware inspect and pause fields without introducing transport-specific recovery DTOs
- `PauseBundle` now uses explicit `ais-agent/pause_bundle/v2` payloads so `required_actions` can expose both:
  - typed `action_kind`
  - executable host command string `action`
  - explicit `retry_intent` for step-oriented retry vs confirmation-poll semantics
  This removes the previous ambiguity where confirmation waits could emit two different semantics through the same `step_run` string alone.
- inspect / pause / terminal result payloads now also surface runtime-owned:
  - `interruption_class`
  - `cancel_state`
  - `side_effect_phase`
- interruption truth is now durable in checkpoint lifecycle through explicit `InterruptionState`, not inferred only from transient step returns
- recovery DTOs are now enforced at runtime boundaries instead of relying on best-effort construction:
  - checkpoint persistence rejects malformed recovery contracts before archive append
  - host inspect rejects malformed recovery truth before transport exposure
- HTTP router/state/error organization is kept thin and aligned with the local `ref/axum/examples` style
- transport e2e now proves real recovery mutation loops, not just projection passthrough:
  - JSONL `await_patch -> submit_plan_patch -> step`
  - HTTP `await_envelope -> submit_envelope -> step`
  - stale / illegal patch and wrong-envelope errors stay machine-readable end-to-end

Current recovery-slice status:
- executable recovery boundaries now exist for:
  - `missing_evidence`
  - `stale_evidence`
  - `simulation_rejected`
  - `governor_denied`
  - `signer_denied`
  - `signer_expired`
  - `envelope_invalid`
- envelope replacement is now a first-class host loop:
  - durable checkpoints carry `pending_envelope_refs`
  - recovery projection emits `await_envelope` when a blocked actuation can be resumed by replacing a known envelope ref
  - `submit_envelope` validates the pending replacement ref and re-arms the blocked node into `running/recovering`
- signer denial / expiry no longer masquerade as a simple return to `awaiting_signer`; they stop at patch-required pause boundaries

## Crate Roles

- `ais-agent-control`
  - control-plane DTOs, ids, commands, events
- `ais-agent-core`
  - domain objects and pure decision logic
- `ais-agent-expr`
  - reduced expression engine for local derivation, policy, verification
- `ais-agent-runtime`
  - real runtime controller, stepper, persistence orchestration, event emission
- `ais-agent-store-sqlite`
  - concrete SQLite durable archive backend for runtime persistence contracts
  - file-backed stores now open with explicit:
    - `journal_mode = WAL`
    - `synchronous = NORMAL`
    - `busy_timeout = 5000ms`
    instead of relying on ambient SQLite defaults
- `ais-agent-host`
  - host session, inspect, signer, ingest, control surfaces
- `ais-agent-transport`
  - JSONL/HTTP adapters only
- `ais-agent-cli`
  - thin shell only
- `ais-agent-chain-shared`
  - chain capability contracts
- `ais-agent-evm`
  - EVM live/read/simulate/broadcast/receipt/state implementations
- `ais-agent-solana`
  - Solana live/read/simulate/broadcast/receipt/state implementations
- `ais-agent-drivers`
  - standard / reflection / API-native / raw-envelope driver integration

## Boundary Rules

- `ais-agent-core` must not depend on runtime, transport, or CLI
- `ais-agent-runtime` may depend on core/control/host and chain-family crates needed for live execution
- `ais-agent-transport` must stay adapter-only
- chain-specific reflection or live implementations stay in chain-family crates
- `ais-agent-drivers` may consume chain-family crates; chain-family crates must not depend on drivers
- no crate here may import modules from `rust/ais-rs`

## Live Execution Binding

Live execution is bound in four layers:

1. driver fragment live-binding hints
   - `ActionGraphFragment.live_binding_hints`
   - `DriverNodeLiveBindingHint`

2. typed action binding
   - chain-scoped live wrappers on action payloads:
     - `ObserveLiveBinding`
     - `SimulateLiveBinding`
     - `ActuateLiveBinding`
     - `VerifyLiveBinding`
   - with family-specific typed bindings inside them:
     - `EvmObserveBinding`
     - `EvmSimulateBinding`
     - `EvmActuateBinding`
     - `EvmVerifyBinding`
     - `SolanaObserveBinding`
     - `SolanaSimulateBinding`
     - `SolanaActuateBinding`
     - `SolanaVerifyBinding`

3. typed live request
   - `EvmConnectionSpec`
   - `EvmObserveRequest`
   - `EvmCallRequest`
   - `SolanaConnectionSpec`
   - `SolanaObserveRequest`
   - `SolanaTransactionRequest`
     - `Legacy`
     - `V0 { address_lookup_tables }`

4. chain-family live ports
   - `alloy` for EVM
   - `solana_sdk` + `solana-client` for Solana
   - current Solana live slices now cover:
     - read
     - simulate
     - broadcast
     - signature-status receipt projection

5. runtime transitions
   - observe -> evidence
   - simulate -> report
   - actuate -> tx submission / signer / receipt wait
   - verify -> receipt / post-state effect verdict

Current frozen rule:
- drivers do not inject protocol-specific runtime logic
- drivers emit fragment-level live-binding hints
- runtime normalizes those hints into typed node payloads before live dispatch
- runtime attaches driver-produced fragments through a single binder:
  - normalize hints
  - inject runtime connection/envelope context
  - merge nodes / requirements / effect contracts into the hot checkpoint graph
- API-native direct-envelope outputs now normalize into the same binder shape:
  - runtime envelope
  - guarded fragment
  - effect contract
- raw envelopes now also enter guarded execution through the runtime binder; they do not bypass:
  - governor
  - signer
  - broadcast wait
  - effect verification
- current runtime proof now covers a mixed-path matrix:
  - standard
  - reflection
  - API-native direct-envelope
  - raw-envelope
  all converge into the same guarded runtime write signature

## Current Simplifications

The current implementation is intentionally narrower than the long-term design language used in older docs.

What exists now:
- evidence graph instead of a fully separated `StateEstimator`
- driver registry instead of a generalized `Capability Router`
- runtime stepper plus explicit transitions instead of a multi-layer planner stack

This is intentional.
These are not contradictions; they are the current executable subset.

## Current Solana Live Slice

The current Solana live path is intentionally narrower than EVM.

Implemented:
- read live port:
  - slot
  - account lamports
  - SPL token-account balance
  - account data
  - signature status
- simulate live port:
  - legacy transaction requests
  - v0 transaction requests with lookup-table accounts
- broadcast / receipt live port:
  - signed transaction broadcast
  - signature-status receipt polling
  - confirmation-depth aware receipt projection
- runtime `Observe` / `Simulate` / `Actuate` / `Verify` transitions now dispatch typed Solana `live` bindings into those ports

Still pending:
- provider-specific API-native Solana clients beyond normalized transaction envelopes
- broader protocol-driver coverage layered on top of the current live slice

## Current High-Priority Gaps

These are the most important remaining architectural gaps:

1. grouped durable transaction orchestration
   - SQLite now provides one backend-native grouped durable transaction for:
     - mission
     - checkpoint
     - event
     - run-catalog
     - signer-state
     - runtime-audit
   - `RuntimeHostService` now routes recovery-sensitive writes through that grouped mutation seam with hot-cache save still happening last
   - the remaining gap is narrower:
     - only SQLite is transaction-native today
     - the in-memory/reference executor remains linear and fail-closed for tests and non-SQLite wiring

2. cross-process durable host-session ownership
   - durable `RunClaimRepository` truth and SQLite-backed claim storage now exist
   - host-service mutating commands now enforce active-claim legality and `begin_run` seeds an initial exclusive claim
   - `inspect_run` still stays a soft relink, while legacy pre-claim durable runs can bootstrap a bounded claim on first legal mutation
   - runtime now also handles:
     - explicit `claim_run`
     - `renew_run_claim`
     - `release_run_claim`
     - compare-and-supersede on paused pre-side-effect runs
     - explicit reacquire after durable lease expiry
   - restart/transport proof now also exists for:
     - active claim surviving restart
     - released claim remaining inspect-readable but mutation-closed after restart
     - expired claim requiring explicit reacquire after restart
     - JSONL release-style handoff across separate service instances sharing SQLite durable claim truth
     - HTTP expiry-style takeover across separate service instances sharing SQLite durable claim truth
     - stale old-owner mutation rejection after takeover
   - the remaining gap is broader ownership across processes or horizontally scaled service instances:
     - claim transition audit payload emission
     - shared lease ownership beyond a single service instance

3. long-running step cancellation/interruption
   - interruption DTOs and runtime classification are now implemented
   - cancellation legality and `request_cancel_run` host/runtime handling are now implemented for:
     - pre-side-effect terminal cancel
     - confirmation-path `cancel_pending`
     - machine-readable `cancel_rejected`
   - restart/e2e proof is now in place for:
     - confirmation-path `cancel_pending`
     - retry-ready confirmation timeout truth
     - JSONL/HTTP confirmation-wait cancel flows
   - remaining gap is automatic terminal resolution for `cancel_pending` and any broader future separation between retry/poll/cancel command surfaces

4. push-based event streaming and richer provider-native integrations
   - durable cursor polling exists today
   - push streaming and richer API-native provider adapters remain future work

## Documentation Policy

Going forward:
- `rust/ais-agent/ARCHITECTURE.md` is the primary architecture reference
- crate-local `README.md` files describe boundaries and current implementation status
- detailed plans may stay under `/docs`
- future implementation-specific design docs should preferably live under `rust/ais-agent/` when they are primarily about this workspace rather than repository-wide strategy
