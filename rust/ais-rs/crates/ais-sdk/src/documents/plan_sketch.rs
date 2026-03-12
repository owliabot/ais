use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchDocument {
    pub schema: String,
    pub intent: String,
    pub pack_snapshot: PlanSketchPackSnapshot,
    pub catalog_snapshot: PlanSketchCatalogSnapshot,
    #[serde(default)]
    pub chain_scope: Vec<String>,
    #[serde(default)]
    pub session: Option<PlanSketchSession>,
    pub segments: Vec<PlanSketchSegment>,
    #[serde(default)]
    pub meta: Option<PlanSketchMeta>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchPackSnapshot {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchCatalogSnapshot {
    pub schema: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchSession {
    pub session_id: String,
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchMeta {
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchSegment {
    pub segment_id: String,
    pub cursor_in: String,
    pub cursor_out: String,
    pub done: bool,
    #[serde(default)]
    pub summary: Option<String>,
    pub steps: Vec<PlanSketchStep>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchStep {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub chain: Option<String>,
    #[serde(default)]
    pub candidate_ref: Option<String>,
    pub inputs: Map<String, Value>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub stores: BTreeMap<String, String>,
    #[serde(default)]
    pub when: Option<PlanSketchWhen>,
    #[serde(default)]
    pub until: Option<Value>,
    #[serde(default)]
    pub retry: Option<PlanSketchRetry>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub constraint_templates: Vec<PlanSketchConstraintTemplateRef>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchWhen {
    pub cel: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchRetry {
    pub interval_ms: u64,
    #[serde(default)]
    pub max_attempts: Option<u64>,
    #[serde(default)]
    pub backoff: Option<PlanSketchRetryBackoff>,
    #[serde(default)]
    pub extensions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanSketchRetryBackoff {
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSketchConstraintTemplateRef {
    pub name: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}
