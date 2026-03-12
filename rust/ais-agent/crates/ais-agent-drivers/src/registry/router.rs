use std::collections::BTreeMap;

use ais_agent_core::{
    evidence::{EvidenceGraph, EvidenceKind},
    mission::Mission,
};

use crate::registry::{DriverCandidate, DriverCapability, DriverPathKind};

#[derive(Debug, Clone, Default)]
pub struct DriverRegistry {
    capabilities: BTreeMap<String, DriverCapability>,
}

impl DriverRegistry {
    pub fn register(&mut self, capability: DriverCapability) {
        self.capabilities
            .insert(capability.driver_id.clone(), capability);
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &DriverCapability> {
        self.capabilities.values()
    }

    pub fn route_candidates(
        &self,
        mission: &Mission,
        evidence: &EvidenceGraph,
    ) -> Vec<DriverCandidate> {
        route_driver_candidates(self.capabilities(), mission, evidence)
    }
}

pub fn route_driver_candidates<'a>(
    capabilities: impl Iterator<Item = &'a DriverCapability>,
    mission: &Mission,
    evidence: &EvidenceGraph,
) -> Vec<DriverCandidate> {
    let available_evidence_kinds: Vec<_> = evidence
        .records
        .values()
        .map(|record| record.kind.clone())
        .collect();
    let mission_goal = mission.goal.to_lowercase();

    let mut candidates = Vec::new();
    for capability in capabilities {
        if capability.path_kind == DriverPathKind::RawEnvelopeFallback
            && !mission.policy.allow_raw_envelopes
        {
            continue;
        }

        if !capability.supported_chains.is_empty()
            && !mission.allowed_chains.is_empty()
            && !capability.supported_chains.iter().any(|chain| {
                mission
                    .allowed_chains
                    .iter()
                    .any(|allowed| allowed == chain)
            })
        {
            continue;
        }

        let mut score = path_base_score(&capability.path_kind);
        let mut matched_reasons = Vec::new();

        let keyword_hits = capability
            .goal_keywords
            .iter()
            .filter(|keyword| mission_goal.contains(keyword.to_lowercase().as_str()))
            .count();
        if keyword_hits > 0 {
            score += (keyword_hits as i32) * 25;
            matched_reasons.push(format!("{keyword_hits} goal keyword matches"));
        }

        let missing_evidence_kinds: Vec<String> = capability
            .required_evidence_kinds
            .iter()
            .filter(|kind| {
                !available_evidence_kinds
                    .iter()
                    .any(|available| available == *kind)
            })
            .map(render_evidence_kind)
            .collect();

        if missing_evidence_kinds.is_empty() {
            if !capability.required_evidence_kinds.is_empty() {
                score += 20;
                matched_reasons.push("all required evidence kinds available".to_owned());
            }
        } else {
            score -= (missing_evidence_kinds.len() as i32) * 20;
        }

        if !capability.supported_chains.is_empty() && !mission.allowed_chains.is_empty() {
            score += 10;
            matched_reasons.push("chain scope matches mission".to_owned());
        }

        candidates.push(DriverCandidate {
            driver_id: capability.driver_id.clone(),
            label: capability.label.clone(),
            path_kind: capability.path_kind.clone(),
            score,
            matched_reasons,
            missing_evidence_kinds,
        });
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.driver_id.cmp(&right.driver_id))
    });
    candidates
}

fn path_base_score(path_kind: &DriverPathKind) -> i32 {
    match path_kind {
        DriverPathKind::StandardDriver => 400,
        DriverPathKind::ReflectionPath => 300,
        DriverPathKind::ApiNativePath => 250,
        DriverPathKind::RawEnvelopeFallback => 100,
    }
}

fn render_evidence_kind(kind: &EvidenceKind) -> String {
    match kind {
        EvidenceKind::Fact => "fact",
        EvidenceKind::QueryResult => "query_result",
        EvidenceKind::RouteOrQuote => "route_or_quote",
        EvidenceKind::Metadata => "metadata",
        EvidenceKind::ExternalObservation => "external_observation",
    }
    .to_owned()
}
