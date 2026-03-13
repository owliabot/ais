# ais-agent-store-sqlite

Purpose:
- provide the first concrete durable backend for `rust/ais-agent`
- keep SQLite schema, migrations, and repository adapters out of `ais-agent-runtime`

Public API entry points:
- `SqliteStore`
- `migrate_connection(...)`
- `SCHEMA_VERSION`

Dependencies on workspace crates:
- `ais-agent-control`
- `ais-agent-core`
- `ais-agent-runtime`

Current implementation status:
- SQLite-backed repository adapters implemented for:
  - `MissionRepository`
  - `RunCatalogRepository`
  - `CheckpointArchive`
  - `EventArchive`
  - `RunClaimRepository`
  - `SignerStateArchive`
  - `RuntimeAuditArchive`
- SQLite now also implements backend-native grouped durable commits through:
  - `DurableMutationExecutor` on `SqliteStore`
  - one SQLite transaction for mission/checkpoint/event/catalog/signer/audit writes
- mission archive adapter now supports both:
  - `insert`
  - `upsert`
- schema bootstrap implemented for:
  - `missions`
  - `run_catalog`
  - `checkpoint_archive`
  - `event_archive`
  - `run_claims`
  - `runtime_audit_archive`
  - `signer_state_archive`
- repository tests currently prove:
  - schema bootstraps cleanly
  - migration is idempotent
  - mission/catalog/checkpoint/event archives round-trip through SQLite
  - checkpoint latest lookup follows monotonic checkpoint truth instead of append order
  - duplicate checkpoint identity append is rejected
  - event archive unlimited and huge-limit reads stay overflow-safe and untruncated
  - pending signer state round-trips through SQLite
  - signer state upsert and clear semantics behave like the in-memory archive
  - run claim acquire / load_active / load_claim round-trip through SQLite
  - active-claim conflict and epoch-mismatch checks are enforced in the SQLite adapter
  - claim expiry and supersede semantics survive durable storage
  - file-backed reopen preserves active claim truth
  - runtime audit archive cursor reads round-trip through SQLite
  - grouped durable commits persist all members on success
  - grouped durable commits roll back earlier writes when a later member fails
- checkpoint archive storage now enforces:
  - unique `(run_id, checkpoint_seq, plan_epoch)` identity
  - indexed latest-checkpoint lookup by checkpoint truth
- run claim storage now enforces:
  - `claim_id` primary-key identity
  - one active claim per run through a partial unique index
  - indexed `(run_id, claim_epoch DESC)` lookup for active/history scans
- SQLite connection defaults now explicitly configure:
  - `foreign_keys = ON`
  - `synchronous = NORMAL`
  - `busy_timeout = 5000ms`
  - `journal_mode = WAL` for file-backed stores
- SQLite-backed runtime/host regressions now also prove:
  - grouped `begin_run` truth replays after restart through real SQLite-backed `RuntimeHostService`
  - inspect after restart from durable mission + latest checkpoint
  - event polling after restart from durable event archive
  - awaiting-evidence resume through real `RuntimeHostService`
  - awaiting-signer resume from durable signer-state archive
  - confirmation-path `cancel_pending` survives restart through durable checkpoint truth
  - signer denial / signer submission survive restart through real `RuntimeHostService`
  - side-effect checkpoint verify-resume truth round-trips through SQLite
  - run-catalog latest pointers stay aligned with checkpoint/event archives under host mutations
  - signer-decision recovery mutations keep checkpoint / signer archive / run catalog aligned on the success path
  - file-backed store pragma defaults are applied on open
  - checkpoint latest/event query plans use supporting indexes

Known gaps:
- no path/file configuration surface beyond direct `SqliteStore` construction
- claim audit does not introduce a second archive here:
  - this crate keeps ownership audit on the existing `runtime_audit_archive` seam
  - claim-specific audit payloads are not yet available from `ais-agent-control`, so this backend stores durable claim truth now and is ready to persist claim audit records through the existing audit table once runtime starts emitting them
