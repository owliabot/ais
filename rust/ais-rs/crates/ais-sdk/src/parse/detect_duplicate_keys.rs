use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use std::collections::BTreeSet;

pub fn detect_yaml_duplicate_keys(input: &str) -> Vec<StructuredIssue> {
    let mut issues = Vec::new();
    let mut stack = vec![MapContext {
        indent: 0,
        keys: BTreeSet::new(),
        path: Vec::new(),
    }];
    let mut sequence_stack = Vec::<SequenceContext>::new();
    let mut last_key_path: Option<Vec<FieldPathSegment>> = None;
    let mut last_key_indent: usize = 0;

    for line in input.lines() {
        let line_no_comment = strip_inline_comment(line);
        if line_no_comment.trim().is_empty() {
            continue;
        }
        let indent = count_leading_spaces(line_no_comment);
        let trimmed = line_no_comment[indent..].trim_start();
        if trimmed == "---" || trimmed == "..." {
            continue;
        }

        while stack.len() > 1 && indent < stack.last().expect("stack not empty").indent {
            stack.pop();
        }
        while sequence_stack
            .last()
            .is_some_and(|sequence| indent < sequence.indent)
        {
            sequence_stack.pop();
        }

        if let Some(item) = trimmed.strip_prefix("- ") {
            let item_trimmed = item.trim_start();
            let seq_path = if let Some(existing) =
                sequence_stack.last().filter(|sequence| sequence.indent == indent)
            {
                existing.path.clone()
            } else {
                let path = if indent > last_key_indent {
                    last_key_path.clone().unwrap_or_default()
                } else {
                    stack.last().expect("stack not empty").path.clone()
                };
                sequence_stack.push(SequenceContext {
                    indent,
                    path: path.clone(),
                    next_index: 0,
                });
                path
            };
            let sequence = sequence_stack
                .last_mut()
                .expect("sequence stack must contain current sequence");
            let item_index = sequence.next_index;
            sequence.next_index += 1;

            while stack.len() > 1 && stack.last().expect("stack not empty").indent >= indent + 2 {
                stack.pop();
            }
            let mut item_path = seq_path;
            item_path.push(FieldPathSegment::Index(item_index));
            stack.push(MapContext {
                indent: indent + 2,
                keys: BTreeSet::new(),
                path: item_path.clone(),
            });

            let Some((key, _value)) = parse_key_value(item_trimmed) else {
                continue;
            };
            let context = stack.last_mut().expect("stack not empty");
            if !context.keys.insert(key.clone()) {
                let mut issue_path = context.path.clone();
                issue_path.push(FieldPathSegment::Key(key.clone()));
                issues.push(StructuredIssue {
                    kind: "parse_error".to_string(),
                    severity: IssueSeverity::Error,
                    node_id: None,
                    field_path: FieldPath::from_segments(issue_path),
                    message: format!("duplicate yaml key `{key}`"),
                    reference: Some("yaml.duplicate_key".to_string()),
                    related: None,
                });
            }

            let mut key_path = context.path.clone();
            key_path.push(FieldPathSegment::Key(key));
            last_key_path = Some(key_path);
            last_key_indent = indent + 2;
            continue;
        }

        let Some((key, _value)) = parse_key_value(trimmed) else {
            continue;
        };

        if indent > stack.last().expect("stack not empty").indent {
            if let Some(parent_path) = &last_key_path {
                if indent > last_key_indent {
                    stack.push(MapContext {
                        indent,
                        keys: BTreeSet::new(),
                        path: parent_path.clone(),
                    });
                }
            }
        }

        let context = stack.last_mut().expect("stack not empty");
        if !context.keys.insert(key.clone()) {
            let mut issue_path = context.path.clone();
            issue_path.push(FieldPathSegment::Key(key.clone()));
            issues.push(StructuredIssue {
                kind: "parse_error".to_string(),
                severity: IssueSeverity::Error,
                node_id: None,
                field_path: FieldPath::from_segments(issue_path),
                message: format!("duplicate yaml key `{key}`"),
                reference: Some("yaml.duplicate_key".to_string()),
                related: None,
            });
        }

        let mut key_path = context.path.clone();
        key_path.push(FieldPathSegment::Key(key));
        last_key_path = Some(key_path);
        last_key_indent = indent;
    }

    StructuredIssue::sort_stable(&mut issues);
    issues
}

#[derive(Debug, Clone)]
struct MapContext {
    indent: usize,
    keys: BTreeSet<String>,
    path: Vec<FieldPathSegment>,
}

#[derive(Debug, Clone)]
struct SequenceContext {
    indent: usize,
    path: Vec<FieldPathSegment>,
    next_index: usize,
}

fn strip_inline_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (index, character) in line.char_indices() {
        match character {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..index],
            _ => {}
        }
    }
    line
}

fn count_leading_spaces(line: &str) -> usize {
    line.chars().take_while(|character| *character == ' ').count()
}

fn parse_key_value(line: &str) -> Option<(String, Option<&str>)> {
    if line.starts_with('?') {
        return None;
    }
    let mut in_single = false;
    let mut in_double = false;
    for (index, character) in line.char_indices() {
        match character {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ':' if !in_single && !in_double => {
                let key = normalize_key(line[..index].trim());
                if key.is_empty() {
                    return None;
                }
                let value = line.get(index + 1..).map(str::trim);
                return Some((key, value));
            }
            _ => {}
        }
    }
    None
}

fn normalize_key(raw: &str) -> String {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        if (bytes[0] == b'"' && bytes[raw.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[raw.len() - 1] == b'\'')
        {
            return raw[1..raw.len() - 1].to_string();
        }
    }
    raw.to_string()
}

#[cfg(test)]
#[path = "detect_duplicate_keys_test.rs"]
mod tests;
