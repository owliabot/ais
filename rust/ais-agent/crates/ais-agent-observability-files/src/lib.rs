//! File-backed observability helpers for `ais-agent`.

mod inspect_files;
mod retained_files;

pub use inspect_files::{
    inspect_jsonl_file, inspect_log_file, FileInspectDirection, JsonlTailOutput, LogTailOutput,
};
pub use retained_files::{DailyFileSink, JsonlCaptureFiles};
