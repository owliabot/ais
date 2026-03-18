use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use ais_agent_transport::jsonl::{JsonlInboundFrame, JsonlOutboundFrame};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileInspectDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Serialize)]
pub struct TailTextLine {
    pub line_number: usize,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct TailJsonlLine {
    pub line_number: usize,
    pub frame: Value,
}

#[derive(Debug, Serialize)]
pub struct LogTailOutput {
    pub path: String,
    pub total_lines: usize,
    pub selected_lines: usize,
    pub lines: Vec<TailTextLine>,
}

#[derive(Debug, Serialize)]
pub struct JsonlTailOutput {
    pub path: String,
    pub direction: FileInspectDirection,
    pub total_lines: usize,
    pub selected_lines: usize,
    pub frames: Vec<TailJsonlLine>,
}

#[derive(Debug, Error)]
pub enum FileInspectError {
    #[error("capture file not found: {0}")]
    NotFound(String),
    #[error("failed to read capture file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSONL frame on line {line}: {message}")]
    InvalidJsonlFrame { line: usize, message: String },
}

pub fn inspect_log_file(path: &Path, tail: usize) -> Result<LogTailOutput, FileInspectError> {
    let (total_lines, lines) = read_tail_lines(path, tail)?;
    Ok(LogTailOutput {
        path: path.display().to_string(),
        total_lines,
        selected_lines: lines.len(),
        lines: lines
            .into_iter()
            .map(|(line_number, text)| TailTextLine { line_number, text })
            .collect(),
    })
}

pub fn inspect_jsonl_file(
    direction: FileInspectDirection,
    path: &Path,
    tail: usize,
) -> Result<JsonlTailOutput, FileInspectError> {
    let (total_lines, lines) = read_tail_lines(path, tail)?;
    let frames = lines
        .into_iter()
        .map(|(line_number, text)| {
            let frame = match direction {
                FileInspectDirection::Inbound => {
                    serde_json::to_value(serde_json::from_str::<JsonlInboundFrame>(&text).map_err(
                        |error| FileInspectError::InvalidJsonlFrame {
                            line: line_number,
                            message: error.to_string(),
                        },
                    )?)
                    .expect("inbound frame serializes")
                }
                FileInspectDirection::Outbound => serde_json::to_value(
                    serde_json::from_str::<JsonlOutboundFrame>(&text).map_err(|error| {
                        FileInspectError::InvalidJsonlFrame {
                            line: line_number,
                            message: error.to_string(),
                        }
                    })?,
                )
                .expect("outbound frame serializes"),
            };
            Ok(TailJsonlLine { line_number, frame })
        })
        .collect::<Result<Vec<_>, FileInspectError>>()?;

    Ok(JsonlTailOutput {
        path: path.display().to_string(),
        direction,
        total_lines,
        selected_lines: frames.len(),
        frames,
    })
}

fn read_tail_lines(
    path: &Path,
    tail: usize,
) -> Result<(usize, Vec<(usize, String)>), FileInspectError> {
    if !path.exists() {
        return Err(FileInspectError::NotFound(path.display().to_string()));
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let keep = tail.max(1);
    let mut lines = VecDeque::with_capacity(keep);
    let mut total_lines = 0usize;

    for line in reader.lines() {
        total_lines += 1;
        let text = line?;
        if lines.len() == keep {
            lines.pop_front();
        }
        lines.push_back((total_lines, text));
    }

    Ok((total_lines, lines.into_iter().collect()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{inspect_jsonl_file, inspect_log_file, FileInspectDirection};

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("ais-agent-observability-files-{name}-{unique}.tmp"))
    }

    #[test]
    fn tails_plaintext_log_files() {
        let path = temp_path("log");
        fs::write(&path, "line-1\nline-2\nline-3\n").expect("write log");

        let output = inspect_log_file(&path, 2).expect("read log");

        assert_eq!(output.total_lines, 3);
        assert_eq!(output.selected_lines, 2);
        assert_eq!(output.lines[0].line_number, 2);
        assert_eq!(output.lines[0].text, "line-2");
        assert_eq!(output.lines[1].text, "line-3");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn tails_and_decodes_jsonl_capture_files() {
        let path = temp_path("jsonl");
        fs::write(
            &path,
            concat!(
                "{\"type\":\"poll_events\",\"query\":{\"run_id\":\"run-1\"}}\n",
                "{\"type\":\"poll_events\",\"query\":{\"run_id\":\"run-2\",\"after_event_seq\":3}}\n"
            ),
        )
        .expect("write capture");

        let output =
            inspect_jsonl_file(FileInspectDirection::Inbound, &path, 1).expect("read capture");

        assert_eq!(output.total_lines, 2);
        assert_eq!(output.selected_lines, 1);
        assert_eq!(output.frames[0].line_number, 2);
        assert_eq!(output.frames[0].frame["type"], "poll_events");
        assert_eq!(output.frames[0].frame["query"]["run_id"], "run-2");

        let _ = fs::remove_file(path);
    }
}
