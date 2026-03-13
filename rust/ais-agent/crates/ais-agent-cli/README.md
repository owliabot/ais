# ais-agent-cli

Purpose:
- provide thin local and daemon entry points for the greenfield `ais-agent`
- avoid embedding runtime business logic directly in the command layer

Public API entry points:
- binary entry point:
  - `ais-agent`
- current modes:
  - `ais-agent local jsonl`
  - `ais-agent daemon http [--bind HOST:PORT]`
  - `ais-agent inspect jsonl --direction inbound|outbound --line '<json>'`

Dependencies on workspace crates:
- `ais-agent-host`
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
- inspect mode decodes JSONL frames for debugging
- the CLI now also owns the first typed service/deployment config seam for:
  - transport
  - storage
  - provider endpoint wiring
  - runtime defaults
  - observability
- config resolution now supports:
  - built-in defaults
  - optional YAML config file
  - environment overrides
  - CLI overrides
- command handlers now go through a bootstrap seam instead of constructing the transport stub inline
- `in_memory` bootstrap now constructs a real runtime-backed `RuntimeHostService`
- SQLite-backed bootstrap now constructs a real archive-backed `RuntimeHostService` over:
  - `SqliteStore` mission/checkpoint/catalog/event archives
  - `SqliteStore` signer archive
  - `SqliteStore` runtime audit archive
  - `SqliteStore` claim repository

Known gaps:
- SQLite bootstrap still uses an in-memory hot `RunRepository` and in-memory host-session store on top of the durable SQLite archives
- no capability discovery surface yet for Owliabot integration
