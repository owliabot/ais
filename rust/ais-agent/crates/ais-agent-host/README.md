# ais-agent-host

Purpose:
- define host-facing session, inspect, evidence ingest, envelope ingest, and signer surfaces
- provide the stable collaboration layer between `ais-agent` and an upper main agent

Public API entry points:
- current public modules:
  - `control`
  - `events`
  - `session`
  - `evidence`
  - `envelope`
  - `ingest`
  - `inspect`
  - `signer`

Dependencies on workspace crates:
- `ais-agent-control`
- `ais-agent-core`

Current implementation status:
- host session/run linkage implemented:
  - `HostSessionId`
  - `HostRunLink`
  - `HostCommandEnvelope`
  - `InMemoryHostSessionStore`
  - idempotency hooks scoped by host session, with mutating-command replay additionally scoped by claim lineage
  - cached replay outcomes for host-driven command retries
  - relink semantics now remove a run from the previous session snapshot before attaching it to a new session
- host command result/service contract implemented:
  - `HostCommandResponse`
  - `HostCommandOutcome`
  - `HostCommandService`
  - `HostCommandOutcome.events` now carries sequenced `RunEventEnvelope` values
  - `HostCommandService` is now async end-to-end via boxed futures so transport and runtime no longer need sync shims
- host event-query contract implemented:
  - `HostRunEventQuery`
  - `HostRunEventBatch`
  - `HostRunEventService`
- host ingest surfaces implemented:
  - `HostEvidenceSubmission`
  - `HostEnvelopeSubmission`
  - `HostSignerDecision`
  - `HostIngestSubmission`
- inspect projection now includes:
  - `InspectSnapshot`
  - `PauseBundle`
  - `ProgressView`
- inspect/pause projection is now recovery-aware and includes:
  - `recovery_disposition`
  - `failure_context`
  - `recovery_suggestions`
  - `allowed_recovery_actions`
  - `interruption_class`
  - `cancel_state`
  - `side_effect_phase`
  - `ownership`
  - `PauseBundle.required_actions[*].action_kind`
  - `PauseBundle.required_actions[*].requires_mutation_claim`
  - terminal `RunResultView` on inspect for completed / failed / cancelled runs
- inspect/pause projection now distinguishes:
  - host signer waits
  - chain confirmation waits (`chain_confirmation`)
- host projector can now distinguish:
  - ordinary waits
  - patch-required pauses
  - fail-closed terminal results
- projector helpers map `Mission + CheckpointSnapshot` into host-facing views
- projector entry points now also accept explicit runtime-owned `RecoveryView` input so transport/session layers do not become the semantic owner of recovery classification
- default projector entry points now also use the shared core checkpoint-recovery classifier, so direct host projector usage stays aligned with runtime-backed inspect semantics
- default projector entry points now also use the shared core claim-policy classifier, so direct host projector usage stays aligned with runtime ownership semantics even before durable claims are enforced
- `PauseBundle` now emits schema `ais-agent/pause_bundle/v2`, where each `required_actions` entry carries:
  - typed `action_kind`
  - host command string `action`
  - `requires_mutation_claim`
  - `retry_intent` for step-oriented retry/poll actions
  so confirmation-wait payloads can distinguish `retry_step` from `await_confirmation` without transport-specific heuristics
- inspect regressions now cover restored checkpoints that still carry pending signer waits
- inspect regressions now also cover:
  - patch-required paused projections
  - terminal failed projections with typed recovery context
  - `AwaitEnvelope` projection from envelope-invalid pause
  - richer verify-mismatch recovery suggestions on the direct projector path
  - confirmation-wait projection with disambiguated step actions
  - ownership snapshots and mutation-claim requirements on inspect/pause/result views
- signer request / decision host contract implemented
- raw envelope host contract implemented
Known gaps:
- host session store is still in-memory only:
  - cross-process/shared durable session ownership is not implemented
  - restart operability is currently recovered through runtime-owned deterministic relink on `inspect_run`, not through durable session persistence
- no push-style subscription surface over session/run events yet
