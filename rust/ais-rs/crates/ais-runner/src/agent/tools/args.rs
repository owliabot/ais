use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct CandidateDetailArgs {
    pub(crate) refs: Vec<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(crate) struct ListCandidatesFilterArgs {
    #[serde(default)]
    pub(crate) chain: Option<String>,
    #[serde(default)]
    pub(crate) protocol: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct GuideGetArgs {
    #[serde(default)]
    pub(crate) schema: Option<String>,
    #[serde(default)]
    pub(crate) topic: Option<String>,
    #[serde(default)]
    pub(crate) full: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ResolveMissingFactsArgs {
    #[serde(default)]
    pub(crate) missing_refs: Vec<String>,
    #[serde(default)]
    pub(crate) limit_per_ref: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CheckSegmentArgs {
    pub(crate) segment: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeginLimits {
    pub(crate) max_rounds: u8,
    pub(crate) max_segments: u8,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeginToolArgs {
    pub(crate) session_id: Value,
    pub(crate) snapshot_hash: Value,
    pub(crate) cursor: Value,
    pub(crate) limits: BeginLimits,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct CatalogDiscoverArgs {
    #[serde(default)]
    pub(crate) chain: Option<String>,
    #[serde(default)]
    pub(crate) protocol: Option<String>,
    #[serde(default)]
    pub(crate) query: Option<String>,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) min_risk_level: Option<u8>,
    #[serde(default)]
    pub(crate) max_risk_level: Option<u8>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AbortIntentEvidence {
    #[serde(default)]
    pub(crate) attempted_recovery: Vec<String>,
    #[serde(default)]
    pub(crate) invalid_fields: Vec<String>,
    #[serde(default)]
    pub(crate) missing_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AbortIntentArgs {
    pub(crate) reason_code: String,
    pub(crate) summary: String,
    pub(crate) evidence: AbortIntentEvidence,
    #[serde(default)]
    pub(crate) user_fix_hint: Option<String>,
}
