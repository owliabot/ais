# ais-agent-transport

Purpose:
- expose JSONL and HTTP adapters over the `ais-agent` host/control contracts
- keep transport semantics thin and subordinate to runtime contracts

Public API entry points:
- current modules:
  - `jsonl`
  - `http`
- current JSONL entry points:
  - `JsonlInboundFrame`
  - `JsonlOutboundFrame`
  - `JsonlServer`
  - `decode_inbound_line(...)`
  - `encode_outbound_frame(...)`
- current HTTP entry points:
  - `build_http_router(...)`

Dependencies on workspace crates:
- `ais-agent-control`
- `ais-agent-host`

Current implementation status:
- JSONL codec and server loop implemented
- JSONL transport delegates all semantics to host/control contracts
- JSONL now supports event polling through a dedicated `PollEvents` inbound frame and `EventBatch` outbound frame
- HTTP transport implemented as a thin `POST /commands` adapter over the same host service contract
- HTTP now exposes `GET /runs/{run_id}/events` for cursor-style event polling
- transport surfaces now carry recovery-aware host projections unchanged, including:
  - `recovery_disposition`
  - `failure_context`
  - `recovery_suggestions`
  - `allowed_recovery_actions`
  - `interruption_class`
  - `cancel_state`
  - `side_effect_phase`
  - `PauseBundle.required_actions[*].action_kind`
  - `PauseBundle.required_actions[*].retry_intent`
  - terminal `run_result`
- outbound transport events now carry sequenced runtime envelopes instead of raw `RunEvent`
- JSONL and HTTP now consume the async `HostCommandService` directly; no sync adapter remains on the hot path
- HTTP router organization now follows the local `ref/axum/examples` style more closely through:
  - route-builder helpers
  - explicit shared state injection
  - typed API error responses
  - error classification for event polling based on host error codes instead of a blanket `404`
- transport e2e now runs against real `RuntimeHostService`
- runtime-backed transport regressions cover:
  - JSONL:
    - confirmation-wait pause payloads preserve typed `required_actions[*].action_kind` on the wire
    - `begin -> inspect -> cancel`
    - confirmation-wait `request_cancel_run -> cancel_pending` through real runtime service
    - preloaded `awaiting_evidence -> submit_evidence -> step -> complete`
    - restart relink loop:
      - `submit_evidence -> session_relink_required`
      - `inspect -> relink`
      - `submit_evidence -> step -> complete`
    - restart patch loop:
      - `inspect -> await_patch`
      - `submit_plan_patch`
      - `step -> complete`
    - preloaded `await_patch -> submit_plan_patch -> step -> complete`
    - stale patch rejection and illegal patch rejection round-trip
    - patch-audit event visibility on successful patch loops
    - recovery-aware inspect round-trip for `awaiting_evidence`
    - retry-ready inspect payload round-trip
    - await-user-input pause payload round-trip
  - HTTP:
    - confirmation-wait pause payloads preserve typed `required_actions[*].action_kind` on the wire
    - confirmation-wait `request_cancel_run -> cancel_pending` through real runtime service
    - event-poll error mapping:
      - `run_not_found -> 404`
      - conflict/state-mismatch -> `409`
      - backend/archive failure -> `503`
    - preloaded `awaiting_signer -> submit_signer_decision -> step -> complete`
    - preloaded `await_envelope -> submit_envelope -> step -> complete`
    - wrong replacement envelope rejection round-trip
    - recovery-aware inspect/pause round-trip for:
      - `awaiting_signer`
      - `awaiting_confirmation`
      - `await_envelope`
      - retry-ready interruption inspect
      - await-user-input interruption pause
    - host collaboration loop through:
      - `inspect`
      - `GET /runs/{run_id}/events`
      - `submit_signer_decision`
      - `step`
      - final `inspect`
    - guarded EVM collaboration proof through:
      - `inspect`
      - `GET /runs/{run_id}/events`
      - host signer approval submission
      - `step`
      - completion event observation
    - guarded Solana collaboration proof through:
      - `inspect`
      - `GET /runs/{run_id}/events`
      - host signer submission
      - `step`
      - confirmation-pause observation
  - transport patch/envelope loops now run against real runtime recovery semantics, not mocked adapter-only shells

Known gaps:
- no long-lived push stream yet; current event surface is polling-oriented
- HTTP still does not expose richer run-management endpoints beyond commands and event polling
