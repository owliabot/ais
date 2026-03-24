# ais-agent-cli

Purpose:
- provide thin local and daemon entry points for `ais-agent`
- avoid embedding runtime business logic directly in the command layer

Public API entry points:
- binary entry point:
  - `ais-agent`
- current modes:
  - `ais-agent local jsonl`
  - `ais-agent daemon http [--bind HOST:PORT]`
  - `ais-agent maintenance prune [--now-ms ...]`
  - `ais-agent maintenance vacuum [--now-ms ...]`
  - `ais-agent maintenance purge --yes <run-id|terminal-before|table> ...`
  - `ais-agent inspect jsonl --direction inbound|outbound --line '<json>'`
  - `ais-agent inspect config`
  - `ais-agent inspect jsonl-file --direction inbound|outbound --path ./capture.jsonl [--tail N]`
  - `ais-agent inspect log-file --path ./var/ais-agent.log [--tail N]`
  - `ais-agent inspect store <overview|run|events|audits|checkpoints|waits|claims|retention|storage|sql> ...`

Dependencies on workspace crates:
- `ais-agent-host`
- `ais-agent-observability-files`
- `ais-agent-store-sqlite`
- `ais-agent-transport`
- external:
  - `clap`

Current implementation status:
- thin CLI shell implemented
- command parsing uses `clap` derive types
- local mode wires stdin/stdout to the async JSONL transport
- local JSONL shell exposes a service-injected helper for transport-level regression tests
- local JSONL shell regression coverage now proves recovery-aware pause payloads, including typed `PauseBundle.required_actions[*].action_kind`, pass through unchanged
- local JSONL shell regression coverage now also preserves typed interruption/cancel projection fields and `required_actions[*].retry_intent`
- daemon mode wires HTTP serving to the transport router
- daemon HTTP mode now emits start / stop tracing lines with the bound socket address
- inspect mode decodes JSONL frames for debugging
- inspect mode now also exposes offline forensics surfaces for:
  - resolved service config
  - JSONL capture files
  - text log files
  - SQLite durable store contents
- observability surface boundaries are now explicit:
  - `ais-agent-observability-files` owns rotated text-log / JSONL-capture files
  - `ais-agent-store-sqlite` owns durable truth, audit, retention, and maintenance queries
  - CLI remains a thin parser / renderer over those two sources
- CLI now delegates file-backed observability logic to `ais-agent-observability-files`
- CLI now delegates SQLite forensics queries to `ais-agent-store-sqlite`
- local JSONL mode can now persist transport capture files under a configured directory with daily rotation and retention pruning
- process tracing can now also mirror to daily log files with retention pruning
- the CLI now also owns the first typed service/deployment config seam for:
  - transport
  - storage
  - provider endpoint wiring
  - runtime defaults
  - observability
- SQLite storage config now also includes retention/maintenance policy for:
  - checkpoint full-window pruning
  - wait-state orphan cleanup
  - destructive purge gating / confirmation
  - auto-prune cadence metadata
  - vacuum threshold metadata
- config resolution now supports:
  - built-in defaults
  - optional YAML config file
  - environment overrides
  - CLI overrides
- `daemon http --bind` now only overrides `transport.http.bind` when the flag is explicitly
  provided; otherwise the YAML/environment-resolved bind is preserved
- CLI observability now installs a process-wide `tracing_subscriber` from:
  - `observability.log_level`
  - `AIS_AGENT_LOG_LEVEL`
  - `--log-level`
- observability config now also supports:
  - `observability.file_logging.enabled|dir|retention_days`
  - `observability.jsonl_capture.enabled|dir|retention_days`
- CLI/env overrides now support:
  - `--log-dir`, `--log-retention-days`
  - `--jsonl-capture-dir`, `--jsonl-capture-retention-days`
  - `AIS_AGENT_LOG_DIR`, `AIS_AGENT_LOG_RETENTION_DAYS`
  - `AIS_AGENT_JSONL_CAPTURE_DIR`, `AIS_AGENT_JSONL_CAPTURE_RETENTION_DAYS`
- command handlers now go through a bootstrap seam instead of constructing the transport stub inline
- `in_memory` bootstrap now constructs a real runtime-backed `RuntimeHostService`
- SQLite-backed bootstrap now constructs a real archive-backed `RuntimeHostService` over:
  - `SqliteStore` mission/checkpoint/catalog/event archives
  - `SqliteStore` signer archive
  - `SqliteStore` runtime audit archive
  - `SqliteStore` claim repository
- store inspection now opens SQLite in read-only mode and supports:
  - overview summaries over `runs`
  - filtered overview slices over:
    - `status`
    - `phase`
    - `active_boundary_kind`
    - `run_id_prefix`
  - per-run mission/checkpoint/wait-state/claim aggregation
  - filtered wait-state listings by `wait_kind`
  - filtered claim listings by:
    - `status`
    - `owner_kind`
    - `host_session_id`
  - paged reads of `run_events` and `run_audits`
  - checkpoint-scoped event/audit filtering via `checkpoint_seq`
  - semantic timeline filtering via:
    - `events.event_kind`
    - `audits.audit_type`
    - `audits.recovery_disposition`
    - `checkpoints.archive_kind`
  - latest or historical `run_checkpoints` reads
  - retention and storage summaries, including growth trend deltas and recent maintenance windows
  - raw read-only SQL escape hatches for deeper forensics
- maintenance mode now executes write-side SQLite retention actions:
  - `prune` removes `terminal_intermediate` checkpoints for old terminal runs
  - `prune` clears orphaned or stale wait-state rows
  - SQLite bootstrap automatically runs `prune` when `auto_prune_cadence_minutes` is due on an existing store
  - `prune` and `purge` trigger SQLite `VACUUM` automatically when `freelist_count` reaches the configured threshold
  - `vacuum` forces a SQLite `VACUUM` pass immediately
  - `purge` supports destructive delete by `run_id`, `terminal_before_ms`, or explicit table
  - destructive purge is gated by config and `--yes`
  - successful operations append `maintenance_journal` and update `store_maintenance_state`
- default file outputs are now intended to look like:
  - `./var/log/ais-agent/ais-agent-YYYY-MM-DD.log`
  - `./var/captures/jsonl/inbound-YYYY-MM-DD.jsonl`
  - `./var/captures/jsonl/outbound-YYYY-MM-DD.jsonl`

Common store filtering examples:

```bash
ais-agent --sqlite-path ./var/ais-agent.db inspect store overview \
  --status awaiting_signer \
  --active-boundary-kind signer \
  --limit 10

ais-agent --sqlite-path ./var/ais-agent.db inspect store waits \
  --wait-kind signer \
  --limit 10

ais-agent --sqlite-path ./var/ais-agent.db inspect store claims \
  --status active \
  --host-session-id session-1 \
  --limit 10

ais-agent --sqlite-path ./var/ais-agent.db inspect store events \
  --run-id run-4 \
  --checkpoint-seq 5 \
  --limit 50

ais-agent --sqlite-path ./var/ais-agent.db inspect store events \
  --run-id run-4 \
  --event-kind awaiting_signer \
  --limit 50

ais-agent --sqlite-path ./var/ais-agent.db inspect store audits \
  --run-id run-4 \
  --audit-type recovery \
  --recovery-disposition await_signer \
  --limit 50

ais-agent --sqlite-path ./var/ais-agent.db inspect store checkpoints \
  --run-id run-4 \
  --archive-kind side_effect \
  --limit 20
```

Use `inspect store sql --query ...` when you need JSON-payload-specific forensics beyond the built-in filters.

Known gaps:
- SQLite bootstrap still uses an in-memory hot `RunRepository` and in-memory host-session store on top of the durable SQLite archives
- no capability discovery surface yet for Owliabot integration
- CLI output surfaces are still JSON-first projections rather than a richer typed operator UI model
