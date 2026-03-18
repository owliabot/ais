use std::path::Path;

use ais_agent_observability_files::{
    inspect_jsonl_file as inspect_jsonl_file_impl, inspect_log_file as inspect_log_file_impl,
    FileInspectDirection,
};

use crate::cli::args::JsonlDirection;

pub fn inspect_log_file(path: &Path, tail: usize) -> Result<(), Box<dyn std::error::Error>> {
    let output = inspect_log_file_impl(path, tail)?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub fn inspect_jsonl_file(
    direction: JsonlDirection,
    path: &Path,
    tail: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = inspect_jsonl_file_impl(map_direction(direction), path, tail)?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn map_direction(direction: JsonlDirection) -> FileInspectDirection {
    match direction {
        JsonlDirection::Inbound => FileInspectDirection::Inbound,
        JsonlDirection::Outbound => FileInspectDirection::Outbound,
    }
}

#[cfg(test)]
mod tests {
    use ais_agent_observability_files::FileInspectDirection;

    use crate::cli::args::JsonlDirection;

    #[test]
    fn maps_cli_jsonl_direction_to_file_inspection_direction() {
        assert_eq!(
            super::map_direction(JsonlDirection::Inbound),
            FileInspectDirection::Inbound
        );
        assert_eq!(
            super::map_direction(JsonlDirection::Outbound),
            FileInspectDirection::Outbound
        );
    }
}
