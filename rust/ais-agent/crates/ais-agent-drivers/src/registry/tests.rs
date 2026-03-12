use std::collections::BTreeMap;

use ais_agent_core::{
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
    },
    mission::{Mission, MissionBudget, MissionPolicy},
};
use serde_json::json;

use crate::registry::{DriverCapability, DriverPathKind, DriverRegistry};

#[test]
fn routing_prefers_keyword_and_evidence_matched_paths() {
    let mut registry = DriverRegistry::default();
    registry.register(DriverCapability {
        driver_id: "swap-standard".to_owned(),
        label: "Swap Standard Driver".to_owned(),
        path_kind: DriverPathKind::StandardDriver,
        supported_chains: vec!["eip155:1".to_owned()],
        required_evidence_kinds: vec![EvidenceKind::RouteOrQuote],
        goal_keywords: vec!["swap".to_owned()],
    });
    registry.register(DriverCapability {
        driver_id: "generic-reflect".to_owned(),
        label: "Generic Reflection".to_owned(),
        path_kind: DriverPathKind::ReflectionPath,
        supported_chains: vec!["eip155:1".to_owned()],
        required_evidence_kinds: Vec::new(),
        goal_keywords: Vec::new(),
    });

    let candidates = registry.route_candidates(&sample_mission(true), &sample_evidence_graph());

    assert_eq!(candidates[0].driver_id, "swap-standard");
    assert!(candidates[0]
        .matched_reasons
        .iter()
        .any(|reason| reason.contains("keyword")));
    assert!(candidates[0].missing_evidence_kinds.is_empty());
}

#[test]
fn routing_excludes_raw_fallback_when_mission_disallows_it() {
    let mut registry = DriverRegistry::default();
    registry.register(DriverCapability {
        driver_id: "raw-envelope".to_owned(),
        label: "Raw Envelope".to_owned(),
        path_kind: DriverPathKind::RawEnvelopeFallback,
        supported_chains: vec!["eip155:1".to_owned()],
        required_evidence_kinds: Vec::new(),
        goal_keywords: vec!["swap".to_owned()],
    });

    let candidates = registry.route_candidates(&sample_mission(false), &sample_evidence_graph());

    assert!(candidates.is_empty());
}

fn sample_mission(allow_raw_envelopes: bool) -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "swap usdc to eth".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned()],
        budget: MissionBudget::default(),
        policy: MissionPolicy {
            allow_raw_envelopes,
            ..MissionPolicy::default()
        },
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn sample_evidence_graph() -> EvidenceGraph {
    let mut records = BTreeMap::new();
    records.insert(
        "quote-1".to_owned(),
        EvidenceRecord {
            evidence_id: "quote-1".to_owned(),
            kind: EvidenceKind::RouteOrQuote,
            provenance: EvidenceProvenance {
                source: "quote-api".to_owned(),
                chain_scope: Some("eip155:1".to_owned()),
                trace_hint: None,
            },
            freshness: EvidenceFreshness::default(),
            confidence_ppm: Some(900_000),
            payload: json!({"amount_out":"1000"}),
        },
    );

    EvidenceGraph {
        records,
        ..EvidenceGraph::default()
    }
}
