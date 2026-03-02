use super::super::context::budget_policy::ContextPressureMode;
use super::super::context::budget_policy::ToolMemoryBudgetPolicy;
use super::*;
use serde_json::json;

#[test]
fn planning_memory_scope_is_snapshot_based() {
    let mut memory = PlanningMemory::default();
    let budget = PlanningMemoryBudget::default();
    memory.ensure_scope("session-a", "snapshot-1");
    memory.insert_with_budget("k".to_string(), "v".to_string(), budget);
    assert_eq!(memory.get("k"), Some("v"));

    memory.ensure_scope("session-b", "snapshot-1");
    assert_eq!(memory.get("k"), Some("v"));

    memory.ensure_scope("session-c", "snapshot-2");
    assert_eq!(memory.get("k"), None);
}

#[test]
fn planning_memory_checkpoint_roundtrip_respects_budget() {
    let mut memory = PlanningMemory::default();
    let budget = PlanningMemoryBudget {
        max_entries: 2,
        max_entry_chars: 4,
        max_total_chars: 6,
    };
    memory.ensure_scope("s", "snap");
    memory.insert_with_budget("a".to_string(), "1111".to_string(), budget);
    memory.insert_with_budget("b".to_string(), "2222".to_string(), budget);
    memory.insert_with_budget("c".to_string(), "3333".to_string(), budget);
    let value = memory
        .checkpoint_value(budget)
        .expect("checkpoint value must exist");
    let mut restored = PlanningMemory::default();
    assert!(restored.restore_from_checkpoint(&value, budget));
    assert!(restored.get("a").is_none());
    assert!(restored.get("b").is_some() || restored.get("c").is_some());
    assert_eq!(
        serde_json::from_value::<PlanningMemorySnapshot>(value)
            .expect("snapshot")
            .snapshot_hash,
        "snap"
    );
}

#[test]
fn restore_ignores_invalid_payload() {
    let mut memory = PlanningMemory::default();
    assert!(!memory.restore_from_checkpoint(&json!({"x":1}), PlanningMemoryBudget::default()));
}

#[test]
fn tool_memory_projection_contains_recent_high_value_entries() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-1");
    memory.insert(
        "list_candidates:k0".to_string(),
        json!({
            "protocols":[
                {
                    "protocol":"erc20@0.0.2",
                    "chains":["eip155:*"],
                    "actions":[{"ref":"erc20@0.0.2/transfer"}],
                    "queries":[{"ref":"erc20@0.0.2/balance-of"}]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
            "catalog.search:k1".to_string(),
            json!({
                "query":"transfer",
                "returned_matches":2,
                "results":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action","schema_name":"erc20@0.0.2"},
                    {"ref":"evm-native-utils@0.0.1/native-transfer","kind":"action","schema_name":"evm-native-utils@0.0.1"}
                ]
            })
            .to_string(),
        );
    memory.insert(
        "get_candidate_detail:k2".to_string(),
        json!({
            "details":[
                {
                    "ref":"erc20@0.0.2/transfer",
                    "kind":"action",
                    "params":[
                        {"name":"to","required":true},
                        {"name":"amount","required":true},
                        {"name":"token","required":true}
                    ],
                    "execution_chains":["eip155:*"]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
        "guide.get:k3".to_string(),
        json!({
            "kind":"topic",
            "topic":{"topic":"cel","summary":"Use deterministic CEL only."}
        })
        .to_string(),
    );

    let projection = memory
        .tool_memory_projection(1200)
        .expect("projection should exist");
    assert_eq!(
        projection.get("schema").and_then(Value::as_str),
        Some("ais-agent-tool-memory-projection/0.0.1")
    );
    assert_eq!(
        projection
            .pointer("/recent/list_inventory/0/protocols/0/protocol")
            .and_then(Value::as_str),
        Some("erc20@0.0.2")
    );
    assert_eq!(
        projection
            .pointer("/recent/catalog_search/0/top_refs/0/ref")
            .and_then(Value::as_str),
        Some("erc20@0.0.2/transfer")
    );
    assert_eq!(
        projection
            .pointer("/recent/candidate_detail/0/signatures/0/ref")
            .and_then(Value::as_str),
        Some("erc20@0.0.2/transfer")
    );
    assert_eq!(
        projection
            .pointer("/recent/guide/topic/cel/summary")
            .and_then(Value::as_str),
        Some("Use deterministic CEL only.")
    );
}

#[test]
fn tool_memory_projection_budget_is_clamped_and_trimmed() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-2");
    for index in 0..8 {
        memory.insert(
            format!("catalog.search:k{index}"),
            json!({
                "query": format!("q{index}"),
                "returned_matches": 1,
                "results": [{"ref": format!("proto@0.0.1/action-{index}"), "kind":"action"}]
            })
            .to_string(),
        );
    }
    let projection = memory
        .tool_memory_projection(64)
        .expect("projection should exist");
    let estimated = projection
        .get("estimated_tokens")
        .and_then(Value::as_u64)
        .expect("estimated tokens");
    let budget = projection
        .get("token_budget")
        .and_then(Value::as_u64)
        .expect("budget");
    assert_eq!(
        budget,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MIN_TOKENS as u64
    );
    assert!(estimated <= budget);
}

#[test]
fn tool_memory_projection_dedupes_catalog_and_detail_refs() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-3");
    memory.insert(
        "catalog.search:k1".to_string(),
        json!({
            "query":"transfer",
            "returned_matches":2,
            "results":[
                {"ref":"erc20@0.0.2/transfer","kind":"action"},
                {"ref":"evm-native-utils@0.0.1/native-transfer","kind":"action"}
            ]
        })
        .to_string(),
    );
    memory.insert(
        "catalog.search:k2".to_string(),
        json!({
            "query":"transfer",
            "returned_matches":2,
            "results":[
                {"ref":"erc20@0.0.2/transfer","kind":"action"},
                {"ref":"erc20@0.0.2/approve","kind":"action"}
            ]
        })
        .to_string(),
    );
    memory.insert(
            "get_candidate_detail:k3".to_string(),
            json!({
                "details":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action","params":[{"name":"to","required":true}]},
                    {"ref":"erc20@0.0.2/approve","kind":"action","params":[{"name":"spender","required":true}]}
                ]
            })
            .to_string(),
        );
    memory.insert(
            "get_candidate_detail:k4".to_string(),
            json!({
                "details":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action","params":[{"name":"to","required":true}]}
                ]
            })
            .to_string(),
        );

    let projection = memory
        .tool_memory_projection(1200)
        .expect("projection should exist");
    let top_ref_a = projection
        .pointer("/recent/catalog_search/0/top_refs/0/ref")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let top_ref_b = projection
        .pointer("/recent/catalog_search/0/top_refs/1/ref")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert_ne!(top_ref_a, top_ref_b);
    let signatures = projection
        .pointer("/recent/candidate_detail")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut refs = BTreeSet::<String>::new();
    for entry in signatures {
        for signature in entry
            .get("signatures")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if let Some(reference) = signature.get("ref").and_then(Value::as_str) {
                refs.insert(reference.to_string());
            }
        }
    }
    assert!(refs.contains("erc20@0.0.2/transfer"));
    assert!(refs.contains("erc20@0.0.2/approve"));
}

#[test]
fn tool_memory_projection_prioritizes_guide_schema_topic() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-4");
    memory.insert(
        "guide.get:k1".to_string(),
        json!({
            "kind":"topic",
            "topic":{"topic":"valueref","summary":"ValueRef forms"}
        })
        .to_string(),
    );
    memory.insert(
        "guide.get:k2".to_string(),
        json!({
            "kind":"schema",
            "schema":{"id":"ais-agent-intent/0.0.1","json":{"$defs":{"x":{}}}}
        })
        .to_string(),
    );
    memory.insert(
        "guide.get:k3".to_string(),
        json!({
            "kind":"schema",
            "schema":{"id":"ais-plan-sketch/0.1.0","json":{"$defs":{"segment":{},"step":{}}}}
        })
        .to_string(),
    );
    memory.insert(
        "guide.get:k4".to_string(),
        json!({
            "kind":"topic",
            "topic":{"topic":"cel","summary":"CEL guide"}
        })
        .to_string(),
    );
    // duplicate higher-priority id should be deduped
    memory.insert(
        "guide.get:k5".to_string(),
        json!({
            "kind":"schema",
            "schema":{"id":"ais-plan-sketch/0.1.0","json":{"$defs":{"segment":{}}}}
        })
        .to_string(),
    );

    let projection = memory
        .tool_memory_projection(1200)
        .expect("projection should exist");
    assert_eq!(
        projection
            .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/defs/0")
            .and_then(Value::as_str),
        Some("segment")
    );
    assert_eq!(
        projection
            .pointer("/recent/guide/topic/cel/summary")
            .and_then(Value::as_str),
        Some("CEL guide")
    );
    let schema_keys = projection
        .pointer("/recent/guide/schema")
        .and_then(Value::as_object)
        .map(|items| items.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    assert_eq!(
        schema_keys
            .iter()
            .filter(|id| id.as_str() == "ais-plan-sketch/0.1.0")
            .count(),
        1
    );
}

#[test]
fn tool_memory_projection_guide_schema_prefers_full_over_digest() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-5");
    memory.insert(
        "guide.get:k1".to_string(),
        json!({
            "kind":"schema",
            "schema":{
                "id":"ais-plan-sketch/0.1.0",
                "mode":"digest",
                "digest":{"defs":["segment"]}
            }
        })
        .to_string(),
    );
    memory.insert(
        "guide.get:k2".to_string(),
        json!({
            "kind":"schema",
            "schema":{
                "id":"ais-plan-sketch/0.1.0",
                "mode":"full",
                "json":{"$defs":{"segment":{},"step":{}}}
            }
        })
        .to_string(),
    );

    let projection = memory
        .tool_memory_projection(1200)
        .expect("projection should exist");
    assert_eq!(
        projection
            .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/mode")
            .and_then(Value::as_str),
        Some("full")
    );
    let defs = projection
        .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/defs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(defs.iter().any(|item| item.as_str() == Some("segment")));
    assert!(defs.iter().any(|item| item.as_str() == Some("step")));
}

#[test]
fn planning_memory_pressure_prune_removes_stale_and_limits_by_pressure_mode() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-prune");
    memory.insert(
        "list_candidates:k1".to_string(),
        json!({
            "protocols":[
                {
                    "protocol":"erc20",
                    "chains":["eip155:*"],
                    "actions":[{"ref":"erc20@0.0.1/a"}],
                    "queries":[{"ref":"erc20@0.0.1/q"}]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
        "list_candidates:k2".to_string(),
        json!({
            "protocols":[
                {
                    "protocol":"erc20",
                    "chains":["eip155:*"],
                    "actions":[{"ref":"erc20@0.0.1/b"}],
                    "queries":[{"ref":"erc20@0.0.1/r"}]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
        "catalog.search:c0".to_string(),
        json!({
            "query": "transfer",
            "returned_matches": 0,
            "results": []
        })
        .to_string(),
    );
    memory.insert(
        "catalog.search:c1".to_string(),
        json!({
            "query": "transfer",
            "returned_matches": 4,
            "results": [
                {"ref":"proto@0.0.1/action-1","kind":"action"},
                {"ref":"proto@0.0.2/action-2","kind":"action"}
            ]
        })
        .to_string(),
    );
    memory.insert(
        "catalog.search:c2".to_string(),
        json!({
            "query": "transfer",
            "returned_matches": 2,
            "results": [
                {"ref":"proto@0.0.1/action-1","kind":"action"},
                {"ref":"proto@0.0.3/action-3","kind":"action"}
            ]
        })
        .to_string(),
    );
    memory.insert(
        "catalog.search:c3".to_string(),
        json!({
            "query": "swap",
            "returned_matches": 1,
            "results": [
                {"ref":"proto@0.0.1/action-s","kind":"action"}
            ]
        })
        .to_string(),
    );
    memory.insert(
        "get_candidate_detail:d1".to_string(),
        json!({
            "details":[
                {
                    "ref":"proto@0.0.1/action-1",
                    "kind":"action",
                    "params":[{"name":"owner","required":true}],
                    "execution_chains":["eip155:*"]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
        "get_candidate_detail:d2".to_string(),
        json!({
            "details":[
                {
                    "ref":"proto@0.0.1/action-1",
                    "kind":"action",
                    "params":[{"name":"owner","required":true}],
                    "execution_chains":["eip155:*"]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
        "get_candidate_detail:d3".to_string(),
        json!({
            "details":[
                {
                    "ref":"proto@0.0.2/action-2",
                    "kind":"query",
                    "params":[{"name":"to","required":true}],
                    "execution_chains":["eip155:1"]
                }
            ]
        })
        .to_string(),
    );
    memory.insert(
        "guide.get:g1".to_string(),
        json!({"kind":"topic","topic":{"topic":"cel","summary":"first"}}).to_string(),
    );
    memory.insert(
        "guide.get:g2".to_string(),
        json!({"kind":"topic","topic":{"topic":"valueref","summary":"second"}}).to_string(),
    );
    memory.insert(
        "guide.get:g3".to_string(),
        json!({
            "kind":"schema",
            "schema":{"id":"ais-plan-sketch/0.1.0","json":{"$defs":{"step":{}}}}
        })
        .to_string(),
    );
    memory.insert("guide.get:g4".to_string(), "{not_json}".to_string());

    let before = serde_json::from_value::<PlanningMemorySnapshot>(
        memory
            .checkpoint_value(PlanningMemoryBudget::default())
            .expect("snapshot should exist"),
    )
    .expect("snapshot parse")
    .tool_cache
    .len();

    let result = memory.prune_for_pressure(ToolMemoryPruneConfig {
        active_todo: true,
        phase: "projection_refresh",
        pressure_mode: ContextPressureMode::Critical,
        projection_budget_tokens: None,
    });

    let after = serde_json::from_value::<PlanningMemorySnapshot>(
        memory
            .checkpoint_value(PlanningMemoryBudget::default())
            .expect("snapshot should exist"),
    )
    .expect("snapshot parse")
    .tool_cache
    .len();

    assert!(
        result
            .removed_by_tool
            .get("catalog.search")
            .copied()
            .unwrap_or(0)
            >= 2
    );
    assert!(
        result
            .removed_by_tool
            .get("get_candidate_detail")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        result
            .removed_by_tool
            .get("guide.get")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        result
            .removed_by_tool
            .get("list_candidates")
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert!(after < before);
    assert!(result.removed_total > 0);

    let projection = memory
        .tool_memory_projection(ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MAX_TOKENS)
        .expect("projection should exist");
    let list_inventory = projection
        .pointer("/recent/list_inventory")
        .and_then(Value::as_array)
        .map(|entries| entries.len())
        .unwrap_or(0);
    let catalog_search = projection
        .pointer("/recent/catalog_search")
        .and_then(Value::as_array)
        .map(|entries| entries.len())
        .unwrap_or(0);
    let candidate_detail = projection
        .pointer("/recent/candidate_detail")
        .and_then(Value::as_array)
        .map(|entries| entries.len())
        .unwrap_or(0);
    let guide_items = projection
        .pointer("/recent/guide/schema")
        .and_then(Value::as_object)
        .map(|obj| obj.len())
        .unwrap_or(0)
        + projection
            .pointer("/recent/guide/topic")
            .and_then(Value::as_object)
            .map(|obj| obj.len())
            .unwrap_or(0);

    let keep_limits = ToolMemoryBudgetPolicy::derive_tool_memory_prune_keep_limits(
        ContextPressureMode::Critical,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS,
    );
    assert!(list_inventory <= keep_limits.max_list);
    assert!(catalog_search <= keep_limits.max_catalog);
    assert!(candidate_detail <= keep_limits.max_detail);
    assert!(guide_items <= keep_limits.max_guide);
}

#[test]
fn planning_memory_pressure_prune_noop_in_normal_mode() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-normal");
    memory.insert(
        "list_candidates:1".to_string(),
        json!({"protocols":[{"protocol":"erc20"}]}).to_string(),
    );

    let before = serde_json::from_value::<PlanningMemorySnapshot>(
        memory
            .checkpoint_value(PlanningMemoryBudget::default())
            .expect("snapshot should exist"),
    )
    .expect("snapshot parse")
    .tool_cache
    .len();
    let result = memory.prune_for_pressure(ToolMemoryPruneConfig {
        active_todo: true,
        phase: "projection_refresh",
        pressure_mode: ContextPressureMode::Normal,
        projection_budget_tokens: None,
    });
    let after = serde_json::from_value::<PlanningMemorySnapshot>(
        memory
            .checkpoint_value(PlanningMemoryBudget::default())
            .expect("snapshot should exist"),
    )
    .expect("snapshot parse")
    .tool_cache
    .len();
    assert_eq!(result.removed_total, 0);
    assert_eq!(before, after);
}

#[test]
fn planning_memory_pressure_prune_keeps_high_priority_guide_on_critical() {
    let mut memory = PlanningMemory::default();
    memory.ensure_scope("s", "snap-guide-priority");
    memory.insert(
        "guide.get:topic-valueref".to_string(),
        json!({"kind":"topic","topic":{"topic":"valueref","summary":"low-priority"}}).to_string(),
    );
    memory.insert(
        "guide.get:topic-cel".to_string(),
        json!({"kind":"topic","topic":{"topic":"cel","summary":"mid-priority"}}).to_string(),
    );
    memory.insert(
        "guide.get:schema-plan-sketch".to_string(),
        json!({
            "kind":"schema",
            "schema":{"id":"ais-plan-sketch/0.1.0","mode":"full","json":{"$defs":{"node":{}}}}
        })
        .to_string(),
    );

    let result = memory.prune_for_pressure(ToolMemoryPruneConfig {
        active_todo: true,
        phase: "projection_refresh",
        pressure_mode: ContextPressureMode::Critical,
        projection_budget_tokens: None,
    });
    assert!(result.removed_total >= 2);

    let projection = memory
        .tool_memory_projection(ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MAX_TOKENS)
        .expect("projection should exist");
    assert_eq!(
        projection
            .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/mode")
            .and_then(Value::as_str),
        Some("full")
    );
    let topic_count = projection
        .pointer("/recent/guide/topic")
        .and_then(Value::as_object)
        .map(|items| items.len())
        .unwrap_or(0);
    assert_eq!(topic_count, 0);
}
