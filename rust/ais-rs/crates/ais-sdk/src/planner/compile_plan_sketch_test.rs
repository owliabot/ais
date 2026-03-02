use super::{compile_plan_sketch, CompilePlanSketchOptions, CompilePlanSketchResult};
use crate::catalog::ExecutableCandidates;
use crate::documents::PlanSketchDocument;
use crate::resolver::ResolverContext;
use ais_core::{stable_hash_hex, StableJsonOptions};
use serde_json::{json, Value};

#[test]
fn compiles_plan_sketch_into_deterministic_plan_nodes() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"check and transfer",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":false,
        "steps":[
          {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}},
          {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","depends_on":["q1"],"inputs":{"amount":"100"}}
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    assert_eq!(plan.schema, "ais-plan/0.0.3");
    assert_eq!(plan.nodes.len(), 2);
    let ids = plan
        .nodes
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|node| node.get("id"))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["s1__q1", "s1__a1"]);
}

#[test]
fn compile_plan_sketch_canary_snapshot_hash_is_stable() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"canary deterministic compile",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[
        {
          "segment_id":"s1",
          "cursor_in":"c0",
          "cursor_out":"c1",
          "done":false,
          "steps":[
            {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}}
          ]
        },
        {
          "segment_id":"s2",
          "cursor_in":"c1",
          "cursor_out":"c2",
          "done":true,
          "steps":[
            {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{"amount":"100"}}
          ]
        }
      ]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };

    let plan_value = serde_json::to_value(&plan).expect("plan json");
    let plan_hash = stable_hash_hex(&plan_value, &StableJsonOptions::default()).expect("hash");
    assert_eq!(
        plan_hash, "c8816a4373f189607217f3ba51cd4f1891a9256c74177f1aa4194e7bc4988ad2",
        "plan canary hash changed; inspect compiler mapping changes before updating this snapshot",
    );
}

#[test]
fn compile_plan_sketch_reports_candidate_not_found() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"bad candidate",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{"segment_id":"s1","cursor_in":"c0","cursor_out":"c1","done":false,"steps":[{"id":"x","kind":"query","candidate_ref":"demo@0.0.2/missing","inputs":{}}]}]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("candidate_not_found")));
        }
    }
}

#[test]
fn compile_plan_sketch_reports_execution_type_not_allowlisted() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"bad execution type",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{"segment_id":"s1","cursor_in":"c0","cursor_out":"c1","done":false,"steps":[{"id":"a","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{"amount":"1"}}]}]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_read")),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("execution_type_not_allowed")));
        }
    }
}

#[test]
fn compile_plan_sketch_normalizes_asset_address_inputs() {
    let mut context = ResolverContext::new();
    context.register_protocol(asset_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"erc20 balance",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:31338"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"asset-demo@0.0.2/balance-of",
            "inputs":{
              "owner":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
              "token":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"}
            }
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&asset_candidates()),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/bindings/params/token/object/address/lit"),
        Some(&json!("0x8464135c8F25Da09e49BC8782676a84730C318bC"))
    );
    assert_eq!(
        node.pointer("/bindings/params/token/object/chain_id/lit"),
        Some(&json!("eip155:31338"))
    );
}

#[test]
fn compile_plan_sketch_normalizes_asset_chain_ref_to_chain_id() {
    let mut context = ResolverContext::new();
    context.register_protocol(asset_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"erc20 balance",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:31338"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"asset-demo@0.0.2/balance-of",
            "inputs":{
              "owner":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
              "token":{"object":{
                "address":{"lit":"0x8464135c8F25Da09e49BC8782676a84730C318bC"},
                "chain_ref":{"lit":"eip155:31338"}
              }}
            }
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&asset_candidates()),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/bindings/params/token/object/chain_id/lit"),
        Some(&json!("eip155:31338"))
    );
    assert!(node
        .pointer("/bindings/params/token/object/chain_ref")
        .is_none());
}

#[test]
fn compile_plan_sketch_copies_constraint_templates_into_policy_extensions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"swap with template constraints",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"a1",
            "kind":"action",
            "candidate_ref":"demo@0.0.2/swap",
            "inputs":{"amount":"100"},
            "constraint_templates":[
              {"name":"max_spend","params":{"amount_atomic":"100"}},
              {"name":"disallow_unlimited_approval"}
            ]
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/extensions/policy/constraint_templates/0/name"),
        Some(&json!("max_spend"))
    );
    assert_eq!(
        node.pointer("/extensions/policy/constraint_templates/0/params/amount_atomic"),
        Some(&json!("100"))
    );
    assert_eq!(
        node.pointer("/extensions/policy/constraint_templates/1/name"),
        Some(&json!("disallow_unlimited_approval"))
    );
}

#[test]
fn compile_plan_sketch_copies_step_stores_into_plan_sketch_extensions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"store query outputs",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{"owner":"0xabc"},
            "stores":{"balance":"inputs.balance","decimals":"token.decimals"}
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/extensions/plan_sketch/stores/balance"),
        Some(&json!("inputs.balance"))
    );
    assert_eq!(
        node.pointer("/extensions/plan_sketch/stores/decimals"),
        Some(&json!("token.decimals"))
    );
}

#[test]
fn compile_plan_sketch_copies_segment_todo_id_into_plan_sketch_extensions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"copy todo id",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "extensions":{"todo_id":"todo_1"},
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{"owner":"0xabc"}
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/extensions/plan_sketch/todo_id"),
        Some(&json!("todo_1"))
    );
}

#[test]
fn compile_plan_sketch_copies_candidate_risk_metadata_into_extensions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"copy candidate risk metadata",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{"owner":"0xabc"}
          },
          {
            "id":"a1",
            "kind":"action",
            "candidate_ref":"demo@0.0.2/swap",
            "depends_on":["q1"],
            "inputs":{"amount":"100"}
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let mut candidates = demo_candidates("evm_read", "evm_call");
    candidates.queries[0]
        .as_object_mut()
        .expect("query card object")
        .insert("risk_level".to_string(), json!(2));
    candidates.queries[0]
        .as_object_mut()
        .expect("query card object")
        .insert("risk_tags".to_string(), json!(["readonly", "pricing"]));
    candidates.actions[0]
        .as_object_mut()
        .expect("action card object")
        .insert("risk_level".to_string(), json!(4));
    candidates.actions[0]
        .as_object_mut()
        .expect("action card object")
        .insert("risk_tags".to_string(), json!(["slippage"]));

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&candidates),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };

    let query_node = plan.nodes.first().expect("query node");
    let action_node = plan.nodes.get(1).expect("action node");
    assert_eq!(
        query_node.pointer("/extensions/risk_level"),
        Some(&json!(2))
    );
    assert_eq!(
        query_node.pointer("/extensions/risk_tags"),
        Some(&json!(["readonly", "pricing"]))
    );
    assert_eq!(
        action_node.pointer("/extensions/risk_level"),
        Some(&json!(4))
    );
    assert_eq!(
        action_node.pointer("/extensions/risk_tags"),
        Some(&json!(["slippage"]))
    );
}

#[test]
fn compile_plan_sketch_keeps_risk_extensions_absent_when_candidate_has_no_risk_metadata() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"risk extensions default compatibility",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"a1",
            "kind":"action",
            "candidate_ref":"demo@0.0.2/swap",
            "inputs":{"amount":"100"}
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };

    let node = plan.nodes.first().expect("node");
    assert!(node.pointer("/extensions/risk_level").is_none());
    assert!(node.pointer("/extensions/risk_tags").is_none());
}

#[test]
fn compile_plan_sketch_accepts_assert_and_branch_step_kinds() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"control-kind passthrough",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {"id":"st_assert","kind":"assert","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}},
          {"id":"st_branch","kind":"branch","candidate_ref":"demo@0.0.2/swap","depends_on":["st_assert"],"inputs":{"amount":"1"}}
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };

    assert_eq!(plan.nodes.len(), 0);
}

#[test]
fn compile_plan_sketch_control_kind_without_discovered_candidate_is_noop() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"control-kind missing candidate",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{"segment_id":"s1","cursor_in":"c0","cursor_out":"c1","done":false,"steps":[{"id":"x","kind":"assert","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}}]}]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        None,
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => {
            panic!("must compile as control no-op: {issues:?}")
        }
    };
    assert!(plan.nodes.is_empty());
}

#[test]
fn compile_plan_sketch_maps_runtime_controls_until_retry_timeout() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"runtime controls",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{"owner":"0xabc"},
            "until":{"cel":"nodes.q1.outputs.value != null"},
            "retry":{"interval_ms":1000,"max_attempts":3,"backoff":"fixed"},
            "timeout_ms":30000
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };

    let node = plan.nodes.first().expect("node");
    assert_eq!(
        node.pointer("/until/cel").and_then(Value::as_str),
        Some("nodes.s1__q1.outputs.value != null")
    );
    assert_eq!(
        node.pointer("/retry/interval_ms").and_then(Value::as_u64),
        Some(1000)
    );
    assert_eq!(
        node.pointer("/retry/max_attempts").and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        node.pointer("/retry/backoff").and_then(Value::as_str),
        Some("fixed")
    );
    assert_eq!(
        node.pointer("/timeout_ms").and_then(Value::as_u64),
        Some(30000)
    );
}

#[test]
fn compile_plan_sketch_rewrites_node_refs_in_bindings_and_conditions() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"rewrite node refs",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"seg-2",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {"id":"q_token","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}},
          {
            "id":"a-transfer",
            "kind":"action",
            "candidate_ref":"demo@0.0.2/swap",
            "depends_on":["q_token"],
            "when":{"cel":"nodes.q_token.outputs.value > 0 || nodes[\"q_token\"].outputs.value > 0"},
            "inputs":{
              "amount":"100",
              "guard":{"ref":"nodes.q_token.outputs.value"}
            }
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    let plan = match result {
        CompilePlanSketchResult::Ok { plan } => plan,
        CompilePlanSketchResult::Err { issues } => panic!("must compile: {issues:?}"),
    };
    assert_eq!(
        plan.nodes
            .get(1)
            .and_then(|node| node.pointer("/condition/cel"))
            .and_then(Value::as_str),
        Some(
            "nodes.seg_2__q_token.outputs.value > 0 || nodes[\"seg_2__q_token\"].outputs.value > 0"
        )
    );
    assert_eq!(
        plan.nodes
            .get(1)
            .and_then(|node| node.pointer("/bindings/params/guard/ref"))
            .and_then(Value::as_str),
        Some("nodes.seg_2__q_token.outputs.value")
    );
}

#[test]
fn compile_plan_sketch_rejects_non_local_node_ref_forms() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"reject non-local refs",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}},
          {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{"amount":"1"},"when":{"cel":"nodes[\"seg-x/q_other\"].outputs.value > 0"}}
        ]
      }]
    }))
    .expect("valid sketch");
    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("non_local_node_ref")));
        }
    }
}

#[test]
fn compile_plan_sketch_rejects_unknown_node_ref() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());
    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"reject unknown refs",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {"id":"q1","kind":"query","candidate_ref":"demo@0.0.2/quote","inputs":{"owner":"0xabc"}},
          {"id":"a1","kind":"action","candidate_ref":"demo@0.0.2/swap","inputs":{"amount":"1"},"when":{"cel":"nodes.q_missing.outputs.value > 0"}}
        ]
      }]
    }))
    .expect("valid sketch");
    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("unknown_node_ref")));
        }
    }
}

#[test]
fn compile_plan_sketch_rejects_invalid_until_valueref_shape() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"invalid until",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{"owner":"0xabc"},
            "until":{"unsupported":"shape"}
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues.iter().any(|issue| {
                issue.reference.as_deref() == Some("input_type_mismatch")
                    && issue.field_path.to_string().ends_with(".until")
            }));
        }
    }
}

#[test]
fn compile_plan_sketch_rejects_invalid_retry_and_timeout_values() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"invalid retry timeout",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{"owner":"0xabc"},
            "retry":{"interval_ms":0,"max_attempts":0,"backoff":"fixed"},
            "timeout_ms":0
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues.iter().any(|issue| {
                issue.reference.as_deref() == Some("input_type_mismatch")
                    && issue.field_path.to_string().ends_with(".retry")
            }));
            assert!(issues.iter().any(|issue| {
                issue.reference.as_deref() == Some("input_type_mismatch")
                    && issue.field_path.to_string().ends_with(".timeout_ms")
            }));
        }
    }
}

#[test]
fn compile_plan_sketch_rejects_invalid_asset_literal_shape() {
    let mut context = ResolverContext::new();
    context.register_protocol(asset_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"erc20 invalid asset",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:31338"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"asset-demo@0.0.2/balance-of",
            "inputs":{
              "owner":{"lit":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"},
              "token":{"lit":123}
            }
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&asset_candidates()),
        &CompilePlanSketchOptions::default(),
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            assert!(issues
                .iter()
                .any(|issue| issue.reference.as_deref() == Some("input_type_mismatch")));
        }
    }
}

#[test]
fn compile_plan_sketch_reports_unknown_input_ref_with_suggestion() {
    let mut context = ResolverContext::new();
    context.register_protocol(demo_protocol());

    let sketch: PlanSketchDocument = serde_json::from_value(json!({
      "schema":"ais-plan-sketch/0.1.0",
      "intent":"unknown input ref",
      "pack_snapshot":{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "catalog_snapshot":{"schema":"ais-catalog/0.0.1","hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
      "chain_scope":["eip155:1"],
      "segments":[{
        "segment_id":"s1",
        "cursor_in":"c0",
        "cursor_out":"c1",
        "done":true,
        "steps":[
          {
            "id":"q1",
            "kind":"query",
            "candidate_ref":"demo@0.0.2/quote",
            "inputs":{
              "owner":{"ref":"inputs.ownre"}
            }
          }
        ]
      }]
    }))
    .expect("valid sketch");

    let result = compile_plan_sketch(
        &sketch,
        &context,
        Some(&demo_candidates("evm_read", "evm_call")),
        &CompilePlanSketchOptions {
            default_chain: None,
            known_input_refs: vec!["inputs.owner".to_string(), "inputs.amount".to_string()],
        },
    );
    match result {
        CompilePlanSketchResult::Ok { .. } => panic!("must fail"),
        CompilePlanSketchResult::Err { issues } => {
            let issue = issues
                .iter()
                .find(|item| item.reference.as_deref() == Some("unknown_input_ref"))
                .expect("unknown_input_ref issue");
            assert!(issue.message.contains("inputs.ownre"));
            assert!(issue.message.contains("suggested_ref=inputs.owner"));
        }
    }
}

fn demo_protocol() -> crate::documents::ProtocolDocument {
    serde_json::from_value(json!({
      "schema":"ais/0.0.2",
      "meta":{"protocol":"demo","version":"0.0.2"},
      "deployments":[{"chain":"eip155:1","contracts":{}}],
      "actions":{
        "swap":{
          "description":"swap action",
          "params":[{"name":"amount","required":true,"type":"uint256"}],
          "execution":{"eip155:*":{"type":"evm_call","to":{"lit":"0x1"},"abi":{"type":"function","name":"swap","inputs":[],"outputs":[]},"args":{}}}
        }
      },
      "queries":{
        "quote":{
          "description":"quote query",
          "params":[{"name":"owner","required":true,"type":"address"}],
          "execution":{"eip155:*":{"type":"evm_read","to":{"lit":"0x1"},"abi":{"type":"function","name":"quote","inputs":[],"outputs":[]},"args":{}}}
        }
      }
    }))
    .expect("protocol")
}

fn demo_candidates(query_exec_type: &str, action_exec_type: &str) -> ExecutableCandidates {
    ExecutableCandidates {
        schema: "ais-executable-candidates/0.0.1".to_string(),
        created_at: None,
        hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        catalog_schema: "ais-catalog/0.0.1".to_string(),
        catalog_hash: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            .to_string(),
        pack: None,
        chain_scope: Some(vec!["eip155:1".to_string()]),
        actions: vec![json!({
          "ref":"demo@0.0.2/swap",
          "execution_types":[action_exec_type],
          "execution_chains":["eip155:*"]
        })],
        queries: vec![json!({
          "ref":"demo@0.0.2/quote",
          "execution_types":[query_exec_type],
          "execution_chains":["eip155:*"]
        })],
        execution_plugins: vec![],
    }
}

fn asset_protocol() -> crate::documents::ProtocolDocument {
    serde_json::from_value(json!({
      "schema":"ais/0.0.2",
      "meta":{"protocol":"asset-demo","version":"0.0.2"},
      "deployments":[{"chain":"eip155:31338","contracts":{}}],
      "actions":{},
      "queries":{
        "balance-of":{
          "description":"balance query",
          "params":[
            {"name":"token","required":true,"type":"asset"},
            {"name":"owner","required":true,"type":"address"}
          ],
          "execution":{
            "eip155:*":{
              "type":"evm_read",
              "to":{"ref":"params.token.address"},
              "abi":{"type":"function","name":"balanceOf","inputs":[],"outputs":[]},
              "args":{"owner":{"ref":"params.owner"}}
            }
          }
        }
      }
    }))
    .expect("protocol")
}

fn asset_candidates() -> ExecutableCandidates {
    ExecutableCandidates {
        schema: "ais-executable-candidates/0.0.1".to_string(),
        created_at: None,
        hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        catalog_schema: "ais-catalog/0.0.1".to_string(),
        catalog_hash: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            .to_string(),
        pack: None,
        chain_scope: Some(vec!["eip155:31338".to_string()]),
        actions: vec![],
        queries: vec![json!({
          "ref":"asset-demo@0.0.2/balance-of",
          "execution_types":["evm_read"],
          "execution_chains":["eip155:*"]
        })],
        execution_plugins: vec![],
    }
}
