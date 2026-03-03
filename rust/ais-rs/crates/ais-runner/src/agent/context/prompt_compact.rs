use serde_json::{json, Map, Value};

pub(in super::super) fn build_prompt_compact(summary: &Value) -> Value {
    let mut compact = Map::<String, Value>::new();
    compact.insert(
        "schema".to_string(),
        Value::String("ais-agent-state-summary-prompt-compact/0.0.1".to_string()),
    );
    compact.insert(
        "todo_state".to_string(),
        summary
            .pointer("/todo_state")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "input_registry".to_string(),
        summary
            .pointer("/input_registry")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "input_slots".to_string(),
        summary
            .pointer("/input_slots")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "canonical_context".to_string(),
        summary
            .pointer("/canonical_context")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "intent_slots".to_string(),
        summary
            .pointer("/intent_slots")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "intent_context".to_string(),
        summary
            .pointer("/intent_context")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "input_store".to_string(),
        summary
            .pointer("/input_store")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "node_output_refs".to_string(),
        summary
            .pointer("/node_output_refs")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "tool_memory_projection".to_string(),
        summary
            .pointer("/tool_memory_projection")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "previous_error".to_string(),
        summary
            .pointer("/previous_error")
            .cloned()
            .unwrap_or(Value::Null),
    );
    compact.insert(
        "context_budget".to_string(),
        compact_context_budget(summary),
    );
    compact.insert(
        "summary_text".to_string(),
        Value::String(build_summary_text(summary)),
    );
    Value::Object(compact)
}

fn compact_context_budget(summary: &Value) -> Value {
    let pressure_mode = summary
        .pointer("/context_budget/pressure_mode")
        .cloned()
        .unwrap_or(Value::Null);
    let overflow_reason = summary
        .pointer("/context_budget/pack_overflow_reason")
        .cloned()
        .unwrap_or(Value::Null);
    let diagnostics = summary
        .pointer("/context_budget/pack_diagnostics")
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "pressure_mode": pressure_mode,
        "pack_overflow_reason": overflow_reason,
        "pack_diagnostics": {
            "packed_blocks_total": diagnostics
                .pointer("/packed_blocks_total")
                .cloned()
                .unwrap_or(Value::Null),
            "packed_blocks_included": diagnostics
                .pointer("/packed_blocks_included")
                .cloned()
                .unwrap_or(Value::Null),
            "packed_blocks_evicted": diagnostics
                .pointer("/packed_blocks_evicted")
                .cloned()
                .unwrap_or(Value::Null),
            "compressed_blocks_total": diagnostics
                .pointer("/compressed_blocks_total")
                .cloned()
                .unwrap_or(Value::Null),
        },
    })
}

fn build_summary_text(summary: &Value) -> String {
    let todo_title = summary
        .pointer("/todo_state/current_todo/title")
        .and_then(Value::as_str)
        .or_else(|| {
            summary
                .pointer("/todo_state/current_todo/id")
                .and_then(Value::as_str)
        })
        .unwrap_or("-");
    let missing_inputs = summary
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let previous_error = summary
        .pointer("/previous_error/reason_code")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let pressure_mode = summary
        .pointer("/context_budget/pressure_mode")
        .and_then(Value::as_str)
        .unwrap_or("normal");

    format!(
        "todo={todo_title}; missing_inputs={missing_inputs}; previous_error={previous_error}; pressure_mode={pressure_mode}"
    )
}
