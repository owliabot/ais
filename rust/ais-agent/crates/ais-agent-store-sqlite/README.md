# ais-agent-store-sqlite

Purpose:
- provide the durable SQLite backend for `rust/ais-agent`
- own SQLite schema, migrations, repository adapters, and store-side inspect queries
- keep runtime persistence policy out of `ais-agent-runtime`

Public API entry points:
- `SqliteStore`
- `migrate_connection(...)`
- `SCHEMA_VERSION`
- `inspect_store(...)`
- `StoreInspectCommand`
- stored run/query types:
  - `StoredRunHead`
  - `StoredRunInput`
  - `StoredRunEvent`
  - `StoredRunAudit`
  - `StoredRunCheckpoint`
  - `StoredRunWaitState`
  - `StoredRunClaim`
- `append_maintenance_journal(...)`
- `list_maintenance_journal(...)`
- `load_store_maintenance_state(...)`
- `upsert_store_maintenance_state(...)`
- `prune_retention(...)`
- `purge_retention(...)`

Dependencies on workspace crates:
- `ais-agent-control`
- `ais-agent-core`
- `ais-agent-runtime`

Current implementation status:
- SQLite-backed adapters are implemented for:
  - `MissionRepository`
  - `RunCatalogRepository`
  - `CheckpointArchive`
  - `EventArchive`
  - `RuntimeAuditArchive`
  - `RunWaitStateStore`
  - signer compatibility shim via `SignerStateStore`
  - `RunClaimRepository`
  - `DurableMutationExecutor`
- The durable schema is now a single final table set:
  - `runs`
  - `run_inputs`
  - `run_events`
  - `run_audits`
  - `run_checkpoints`
  - `run_wait_states`
  - `run_claim_history`
  - `maintenance_journal`
  - `store_maintenance_state`
- Store-side inspect queries are owned here for:
  - run overview summaries
  - per-run mission/checkpoint/wait-state/claim aggregation
  - event/audit/checkpoint/claim timeline reads
  - retention summaries over `runs`, `run_checkpoints`, and the singleton `store_maintenance_state` metadata snapshot
  - storage summaries over SQLite page stats, table row counts, and recent maintenance growth/reclaim deltas
  - raw read-only SQL escape hatches
- Store-side maintenance execution is now owned here for:
  - terminal checkpoint pruning
  - stale/orphan wait-state cleanup
  - destructive purge by run, terminal cutoff, or explicit table
  - threshold-triggered or explicit SQLite `VACUUM`
  - durable maintenance journal/state updates
  - global store metadata stamping for open time, last cleanup/vacuum timings, and latest storage footprint sample
  - auto-prune cadence support for CLI bootstrap paths via `store_maintenance_state.last_prune_finished_at_ms`
- Repository and host-service tests currently prove:
  - schema bootstrap is clean and migration is idempotent
  - mission/catalog/checkpoint/event/audit/wait-state/claim truth round-trips through SQLite
  - grouped durable commits are atomic on success/failure
  - duplicate checkpoint identity is rejected
  - hot checkpoint/event read paths use supporting indexes
  - file-backed reopen preserves active-claim truth
  - restart-time inspect/resume paths work from the final SQLite tables
  - file-backed store pragma defaults are applied on open

Known gaps:
- `runs.retention_mode` currently distinguishes active vs terminal retention (`active_full` / `terminal_tiered`) but does not yet encode richer side-effect classes
- incremental vacuum is not implemented; current strategy is full `VACUUM` gated by freelist threshold or explicit operator command
- inspection results are still JSON-shaped read models, not a typed operator bundle
