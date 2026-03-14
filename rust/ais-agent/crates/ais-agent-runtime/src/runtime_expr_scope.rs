use serde_json::{Map, Value};

use crate::runtime::ActiveRun;

pub(crate) fn artifact_refs_value(runtime: &ActiveRun) -> Result<Value, String> {
    let Some(snapshot) = runtime.checkpoint.execution_artifact.as_ref() else {
        return Err("execution_artifact runtime state is not present".to_owned());
    };

    let mut refs = Map::new();
    refs.insert(
        "evidence".to_owned(),
        merged_evidence_value(
            &snapshot.launch_spec.evidence,
            &runtime.checkpoint.evidence_graph,
        ),
    );
    refs.insert(
        "exports".to_owned(),
        dotted_map_value(
            snapshot
                .exported_outputs
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone())),
        ),
    );
    refs.insert(
        "outputs".to_owned(),
        dotted_map_value(
            snapshot
                .exported_outputs
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone())),
        ),
    );
    refs.insert(
        "receipts".to_owned(),
        receipt_alias_value(&runtime.checkpoint.evidence_graph),
    );
    refs.insert(
        "artifact".to_owned(),
        serde_json::json!({
            "protocol_package_id": snapshot.launch_spec.protocol_package_id,
            "action_key": snapshot.launch_spec.action_key,
            "active_stage_id": snapshot.active_stage_id.as_ref().map(|value| value.as_str()),
            "awaiting_continuation": snapshot.awaiting_continuation.is_some(),
            "metadata": snapshot.launch_spec.metadata,
        }),
    );

    Ok(Value::Object(refs))
}

fn merged_evidence_value(
    launch_evidence: &Value,
    evidence_graph: &ais_agent_core::evidence::EvidenceGraph,
) -> Value {
    let mut root = match launch_evidence {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        other => {
            let mut map = Map::new();
            map.insert("launch".to_owned(), other.clone());
            map
        }
    };

    for record in evidence_graph.records.values() {
        insert_dotted_value(
            &mut root,
            record.evidence_id.as_str(),
            record.payload.clone(),
        );
    }

    Value::Object(root)
}

fn receipt_alias_value(evidence_graph: &ais_agent_core::evidence::EvidenceGraph) -> Value {
    let mut root = Map::new();
    for record in evidence_graph.records.values() {
        let Some(node_id) = record.provenance.trace_hint.as_deref() else {
            continue;
        };
        let Some(stage_id) = node_id
            .strip_prefix("artifact.")
            .and_then(|value| value.strip_suffix(".verify"))
        else {
            continue;
        };
        if !record.evidence_id.starts_with("receipt.") {
            continue;
        }
        insert_dotted_value(&mut root, stage_id, record.payload.clone());
    }
    Value::Object(root)
}

fn dotted_map_value<'a>(values: impl Iterator<Item = (&'a str, Value)>) -> Value {
    let mut root = Map::new();
    for (key, value) in values {
        insert_dotted_value(&mut root, key, value);
    }
    Value::Object(root)
}

fn insert_dotted_value(root: &mut Map<String, Value>, dotted_key: &str, value: Value) {
    let mut segments = dotted_key
        .split('.')
        .filter(|segment| !segment.is_empty())
        .peekable();
    if segments.peek().is_none() {
        return;
    }

    let mut cursor = root;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            cursor.insert(segment.to_owned(), value);
            return;
        }

        let entry = cursor
            .entry(segment.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cursor = entry.as_object_mut().expect("normalized object value");
    }
}
