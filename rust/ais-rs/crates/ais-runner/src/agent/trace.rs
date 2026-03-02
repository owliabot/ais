pub(super) fn emit(enabled: bool, phase: &str, event: &str, fields: &[(&str, String)]) {
    if !enabled {
        return;
    }

    let mut line = format!("[agent.trace] phase={phase} event={event}");
    for (key, value) in fields {
        if value.trim().is_empty() {
            continue;
        }
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(compact_value(value).as_str());
    }
    eprintln!("{line}");
}

fn compact_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "tests/trace.rs"]
mod tests;
