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
  - runtime audit archive cursor reads round-trip through SQLite
  - grouped durable commits persist all members on success
  - grouped durable commits roll back earlier writes when a later member fails
- checkpoint archive storage now enforces:
  - unique `(run_id, checkpoint_seq, plan_epoch)` identity
  - indexed latest-checkpoint lookup by checkpoint truth
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
