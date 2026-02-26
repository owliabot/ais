use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct JsonBudgetOptions {
    pub max_depth: usize,
    pub max_object_entries: usize,
    pub max_array_items: usize,
    pub max_string_chars: usize,
}

impl Default for JsonBudgetOptions {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_object_entries: 24,
            max_array_items: 24,
            max_string_chars: 240,
        }
    }
}

pub fn compact_json_for_llm(value: &Value) -> Value {
    compact_json_with_options(value, &JsonBudgetOptions::default())
}

pub fn compact_json_with_options(value: &Value, options: &JsonBudgetOptions) -> Value {
    compact_inner(value, options, 0)
}

fn compact_inner(value: &Value, options: &JsonBudgetOptions, depth: usize) -> Value {
    if depth >= options.max_depth {
        return Value::String("[TRUNCATED_DEPTH]".to_string());
    }
    match value {
        Value::Object(object) => {
            let mut out = serde_json::Map::new();
            for (index, (key, item)) in object.iter().enumerate() {
                if index >= options.max_object_entries {
                    out.insert(
                        "_truncated_object_entries".to_string(),
                        Value::String((object.len() - options.max_object_entries).to_string()),
                    );
                    break;
                }
                out.insert(key.clone(), compact_inner(item, options, depth + 1));
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let mut out = Vec::<Value>::new();
            for (index, item) in items.iter().enumerate() {
                if index >= options.max_array_items {
                    out.push(Value::String(format!(
                        "[TRUNCATED_ARRAY_ITEMS:{}]",
                        items.len() - options.max_array_items
                    )));
                    break;
                }
                out.push(compact_inner(item, options, depth + 1));
            }
            Value::Array(out)
        }
        Value::String(text) => {
            if text.chars().count() <= options.max_string_chars {
                return Value::String(text.clone());
            }
            let mut clipped = text
                .chars()
                .take(options.max_string_chars)
                .collect::<String>();
            clipped.push_str("...");
            Value::String(clipped)
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compact_json_limits_depth_arrays_and_strings() {
        let input = json!({
            "a":[1,2,3,4,5],
            "b":{"c":{"d":{"e":{"f":"abcdefghijklmnopqrstuvwxyz"}}}},
            "text":"abcdefghijklmnopqrstuvwxyz",
        });
        let out = compact_json_with_options(
            &input,
            &JsonBudgetOptions {
                max_depth: 4,
                max_object_entries: 10,
                max_array_items: 2,
                max_string_chars: 5,
            },
        );
        assert_eq!(
            out.pointer("/a/2"),
            Some(&json!("[TRUNCATED_ARRAY_ITEMS:3]"))
        );
        assert_eq!(out.pointer("/b/c/d/e"), Some(&json!("[TRUNCATED_DEPTH]")));
        assert_eq!(out.pointer("/text"), Some(&json!("abcde...")));
    }
}
