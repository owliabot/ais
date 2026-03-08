use serde::Serialize;
use serde_json::Value;

pub(crate) struct IntentSlotsView<'a> {
    resolved_inputs: Option<&'a serde_json::Map<String, Value>>,
}

impl<'a> IntentSlotsView<'a> {
    pub fn resolved_input(&self, slot: &str) -> Option<&'a Value> {
        self.resolved_inputs.and_then(|inputs| {
            inputs
                .get(slot)
                .or_else(|| value_at_dotted_path_object(inputs, slot))
        })
    }
}

pub(crate) struct RuntimeFactsView<'a> {
    facts: Option<&'a serde_json::Map<String, Value>>,
    meta: Option<&'a serde_json::Map<String, Value>>,
}

impl<'a> RuntimeFactsView<'a> {
    pub fn fact(&self, full_ref: &str) -> Option<&'a Value> {
        self.facts.and_then(|facts| {
            facts
                .get(full_ref)
                .or_else(|| value_at_dotted_path_object(facts, full_ref))
        })
    }

    pub fn meta(&self, full_ref: &str) -> Option<&'a Value> {
        self.meta.and_then(|meta| {
            meta.get(full_ref)
                .or_else(|| value_at_dotted_path_object(meta, full_ref))
        })
    }
}

pub(crate) struct TodoStateView<'a> {
    current_todo: Option<&'a Value>,
}

impl<'a> TodoStateView<'a> {
    #[allow(dead_code)]
    pub fn current_todo(&self) -> Option<&'a Value> {
        self.current_todo
    }

    #[allow(dead_code)]
    pub fn current_todo_id(&self) -> Option<&'a str> {
        self.current_todo
            .and_then(|todo| todo.get("id"))
            .and_then(Value::as_str)
    }

    #[allow(dead_code)]
    pub fn current_todo_status(&self) -> Option<&'a str> {
        self.current_todo
            .and_then(|todo| todo.get("status"))
            .and_then(Value::as_str)
    }

    #[allow(dead_code)]
    pub fn current_todo_blocked_reason(&self) -> Option<&'a str> {
        self.current_todo
            .and_then(|todo| todo.get("blocked_reason"))
            .and_then(Value::as_str)
    }

    pub fn current_todo_execution_scope(&self) -> Option<&'a str> {
        self.current_todo
            .and_then(|todo| todo.get("execution_scope"))
            .and_then(Value::as_str)
    }

    pub fn current_todo_title(&self) -> Option<&'a str> {
        self.current_todo
            .and_then(|todo| todo.get("title"))
            .and_then(Value::as_str)
    }

    pub fn current_todo_string_list(&self, field: &str) -> Vec<&'a str> {
        self.current_todo
            .and_then(|todo| todo.get(field))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
}

pub(crate) struct RecoveryDiagnosticsView<'a> {
    available_attempt_keys: Option<&'a Vec<Value>>,
}

impl<'a> RecoveryDiagnosticsView<'a> {
    pub fn available_attempt_keys(&self) -> Vec<&'a str> {
        self.available_attempt_keys
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
}

/// Strongly-typed representation of the agent's state summary.
///
/// Built by `projector::build_projected_summary_base` and serialized to
/// `Value` at the budget/pack boundary in `context_view.rs`.
///
/// Sub-projections that have complex or evolving schemas (`input_registry`,
/// `node_output_refs`, `input_store`) are kept as `Value` to allow
/// incremental migration.  Top-level scalars and the `input_binding`
/// contract are fully typed.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct StateSummary {
    pub completed_segments: usize,
    pub completed_nodes: usize,
    pub plan_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_reason: Option<String>,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_error: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_store: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_facts: Option<Value>,
    pub input_binding: InputBindingContract,
    pub input_registry: Value,
    pub node_output_refs: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reusable_outputs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_memory_projection: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_slots: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_context: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_view: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side_effect_lifecycle: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_diagnostics: Option<Value>,
}

impl StateSummary {
    /// Serialize to `serde_json::Value` for the budget/pack pipeline.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    // ---- Typed accessors for frequently-queried sub-paths ----

    /// `/input_store/facts` as a JSON object reference.
    pub fn input_store_facts(&self) -> Option<&serde_json::Map<String, Value>> {
        self.input_store.as_ref()?.get("facts")?.as_object()
    }

    /// `/input_store/meta` as a JSON object reference.
    pub fn input_store_meta(&self) -> Option<&serde_json::Map<String, Value>> {
        self.input_store.as_ref()?.get("meta")?.as_object()
    }

    pub fn runtime_facts_facts(&self) -> Option<&serde_json::Map<String, Value>> {
        self.runtime_facts.as_ref()?.get("facts")?.as_object()
    }

    pub fn runtime_facts_meta(&self) -> Option<&serde_json::Map<String, Value>> {
        self.runtime_facts.as_ref()?.get("meta")?.as_object()
    }

    pub fn runtime_facts_view(&self) -> RuntimeFactsView<'_> {
        RuntimeFactsView {
            facts: self.runtime_facts_facts(),
            meta: self.runtime_facts_meta(),
        }
    }

    /// `/intent_context/facts` as a JSON object reference.
    pub fn intent_context_facts(&self) -> Option<&serde_json::Map<String, Value>> {
        self.intent_context.as_ref()?.get("facts")?.as_object()
    }

    /// `/node_output_refs/known_refs` as a list of ref strings.
    pub fn node_output_refs_known_refs(&self) -> Vec<&str> {
        self.node_output_refs
            .get("known_refs")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// `/nodes` — the completed node outputs map (stored inside runtime, proxied here via
    /// the `node_output_refs` projection; for direct node data, callers still use the Value).
    pub fn intent_slots_resolved_inputs(&self) -> Option<&serde_json::Map<String, Value>> {
        self.intent_slots
            .as_ref()?
            .get("resolved_inputs")?
            .as_object()
    }

    pub fn intent_slots_view(&self) -> IntentSlotsView<'_> {
        IntentSlotsView {
            resolved_inputs: self.intent_slots_resolved_inputs(),
        }
    }

    #[allow(dead_code)]
    /// `/todo_state/current_todo` as a Value reference.
    pub fn current_todo(&self) -> Option<&Value> {
        self.todo_state_view().current_todo()
    }

    pub fn todo_state_view(&self) -> TodoStateView<'_> {
        TodoStateView {
            current_todo: self
                .todo_state
                .as_ref()
                .and_then(|todo| todo.get("current_todo")),
        }
    }

    pub fn recovery_diagnostics_view(&self) -> RecoveryDiagnosticsView<'_> {
        RecoveryDiagnosticsView {
            available_attempt_keys: self
                .recovery_diagnostics
                .as_ref()
                .and_then(|value| value.get("available_attempt_keys"))
                .and_then(Value::as_array),
        }
    }

    pub fn previous_error_autofill_attempt_keys(&self) -> Vec<&str> {
        self.previous_error
            .as_ref()
            .and_then(|value| value.pointer("/autofill_history/attempt_keys"))
            .and_then(Value::as_array)
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    pub fn allowed_recovery_attempt_keys(&self) -> Vec<&str> {
        let mut merged = self.recovery_diagnostics_view().available_attempt_keys();
        for key in self.previous_error_autofill_attempt_keys() {
            if !merged.contains(&key) {
                merged.push(key);
            }
        }
        merged
    }
}

fn value_at_dotted_path_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    dotted: &str,
) -> Option<&'a Value> {
    let mut segments = dotted.split('.').filter(|part| !part.is_empty());
    let first = segments.next()?;
    let mut current = map.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn todo_state_view_reads_current_todo_fields() {
        let summary = StateSummary {
            completed_segments: 0,
            completed_nodes: 0,
            plan_epoch: 0,
            paused_reason: None,
            done: false,
            previous_error: None,
            input_store: None,
            runtime_facts: None,
            input_binding: InputBindingContract {
                schema: "ais-agent-input-binding-contract/0.0.1",
                bindable_namespace: "inputs",
                bindable_refs_source: "state_summary.input_store",
                bindable_refs_projection: "state_summary.input_registry.known_refs",
                known_refs_only: true,
                facts_bindable: false,
            },
            input_registry: json!({"known_refs":[]}),
            node_output_refs: json!({"known_refs":[]}),
            reusable_outputs: None,
            tool_memory_projection: None,
            intent_slots: None,
            intent_context: None,
            capability_view: None,
            capability_ready: None,
            side_effect_lifecycle: None,
            todo_state: Some(json!({
                "current_todo": {
                    "id": "todo_2",
                    "status": "blocked",
                    "blocked_reason": "missing token",
                    "execution_scope": "query_only",
                    "title": "Check balances",
                    "required_facts": ["facts.balance"]
                }
            })),
            recovery_diagnostics: None,
        };

        let view = summary.todo_state_view();
        assert_eq!(view.current_todo_id(), Some("todo_2"));
        assert_eq!(view.current_todo_status(), Some("blocked"));
        assert_eq!(view.current_todo_blocked_reason(), Some("missing token"));
        assert_eq!(view.current_todo_execution_scope(), Some("query_only"));
        assert_eq!(view.current_todo_title(), Some("Check balances"));
        assert_eq!(
            view.current_todo_string_list("required_facts"),
            vec!["facts.balance"]
        );
    }

    #[test]
    fn recovery_diagnostics_view_reads_attempt_keys() {
        let summary = StateSummary {
            completed_segments: 0,
            completed_nodes: 0,
            plan_epoch: 0,
            paused_reason: None,
            done: false,
            previous_error: Some(json!({
                "autofill_history": {
                    "attempt_keys": ["history.retry"]
                }
            })),
            input_store: None,
            runtime_facts: None,
            input_binding: InputBindingContract {
                schema: "ais-agent-input-binding-contract/0.0.1",
                bindable_namespace: "inputs",
                bindable_refs_source: "state_summary.input_store",
                bindable_refs_projection: "state_summary.input_registry.known_refs",
                known_refs_only: true,
                facts_bindable: false,
            },
            input_registry: json!({"known_refs":[]}),
            node_output_refs: json!({"known_refs":[]}),
            reusable_outputs: None,
            tool_memory_projection: None,
            intent_slots: None,
            intent_context: None,
            capability_view: None,
            capability_ready: None,
            side_effect_lifecycle: None,
            todo_state: None,
            recovery_diagnostics: Some(json!({
                "available_attempt_keys": ["runtime.query.resolve", "query_ref:erc20@0.0.2/decimals"]
            })),
        };

        let view = summary.recovery_diagnostics_view();
        assert_eq!(
            view.available_attempt_keys(),
            vec!["runtime.query.resolve", "query_ref:erc20@0.0.2/decimals"]
        );
        assert_eq!(
            summary.previous_error_autofill_attempt_keys(),
            vec!["history.retry"]
        );
        assert_eq!(
            summary.allowed_recovery_attempt_keys(),
            vec![
                "runtime.query.resolve",
                "query_ref:erc20@0.0.2/decimals",
                "history.retry"
            ]
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InputBindingContract {
    pub schema: &'static str,
    pub bindable_namespace: &'static str,
    pub bindable_refs_source: &'static str,
    pub bindable_refs_projection: &'static str,
    pub known_refs_only: bool,
    pub facts_bindable: bool,
}
