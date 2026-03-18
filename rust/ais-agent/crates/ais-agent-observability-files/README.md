# ais-agent-observability-files

Purpose:
- own file-backed observability helpers for `rust/ais-agent`
- keep log rotation, JSONL capture persistence, and offline file inspection out of `ais-agent-cli`

Public API entry points:
- `DailyFileSink`
- `JsonlCaptureFiles`
- `inspect_log_file(...)`
- `inspect_jsonl_file(...)`
- `FileInspectDirection`

Dependencies on workspace crates:
- `ais-agent-transport`

Current implementation status:
- daily file sink implemented for append-only file outputs
- file retention currently prunes files older than the configured day window
- JSONL capture helper now persists separate inbound/outbound files
- offline inspection helpers now tail:
  - plaintext log files
  - JSONL capture files decoded into typed transport frames

Known gaps:
- retention is based on UTC calendar dates encoded in filenames, not mtime
- HTTP daemon capture helpers are not wired yet; current JSONL capture integration is for local JSONL mode
