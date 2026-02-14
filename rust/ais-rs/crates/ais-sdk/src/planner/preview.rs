use super::readiness::{
    get_node_readiness, get_node_readiness_async, NodeReadinessResult, NodeRunState,
};
use crate::documents::PlanDocument;
use crate::resolver::{DetectResolver, ResolverContext, ValueRefEvalOptions};
use ais_core::{stable_hash_hex, FieldPath, FieldPathSegment, IssueSeverity, StableJsonOptions, StructuredIssue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DRY_RUN_REPORT_SCHEMA_0_0_1: &str = "ais-dry-run-report/0.0.1";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunSummary {
    pub total_nodes: usize,
    pub ready_nodes: usize,
    pub blocked_nodes: usize,
    pub skipped_nodes: usize,
    pub estimated_confirmation_points: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunNodeReport {
    pub node_id: String,
    #[serde(default)]
    pub chain: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub execution_type: Option<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub risk_flags: Vec<String>,
    pub readiness: NodeReadinessResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunJsonReport {
    pub schema: String,
    pub summary: DryRunSummary,
    pub plan_hash: String,
    pub report_hash: String,
    pub nodes: Vec<DryRunNodeReport>,
    pub issues: Vec<StructuredIssue>,
}

pub fn dry_run_json(
    plan: &PlanDocument,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> DryRunJsonReport {
    let mut issues = Vec::<StructuredIssue>::new();
    let mut nodes = Vec::<DryRunNodeReport>::new();

    for (index, node) in plan.nodes.iter().enumerate() {
        match build_node_report_sync(node, index, context, options, &mut issues) {
            Some(report) => nodes.push(report),
            None => continue,
        }
    }

    let summary = summarize_reports(&nodes);
    let plan_hash = stable_hash_for_value(
        &serde_json::to_value(plan).unwrap_or(Value::Null),
        "dry_run.plan_hash_error",
        &mut issues,
    );
    StructuredIssue::sort_stable(&mut issues);

    let mut report = DryRunJsonReport {
        schema: DRY_RUN_REPORT_SCHEMA_0_0_1.to_string(),
        summary,
        plan_hash,
        report_hash: String::new(),
        nodes,
        issues,
    };
    report.report_hash = stable_hash_for_report(&report);
    report
}

pub async fn dry_run_json_async(
    plan: &PlanDocument,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    detect_resolver: Option<&dyn DetectResolver>,
) -> DryRunJsonReport {
    let mut issues = Vec::<StructuredIssue>::new();
    let mut nodes = Vec::<DryRunNodeReport>::new();

    for (index, node) in plan.nodes.iter().enumerate() {
        match build_node_report_async(node, index, context, options, detect_resolver, &mut issues).await {
            Some(report) => nodes.push(report),
            None => continue,
        }
    }

    let summary = summarize_reports(&nodes);
    let plan_hash = stable_hash_for_value(
        &serde_json::to_value(plan).unwrap_or(Value::Null),
        "dry_run.plan_hash_error",
        &mut issues,
    );
    StructuredIssue::sort_stable(&mut issues);

    let mut report = DryRunJsonReport {
        schema: DRY_RUN_REPORT_SCHEMA_0_0_1.to_string(),
        summary,
        plan_hash,
        report_hash: String::new(),
        nodes,
        issues,
    };
    report.report_hash = stable_hash_for_report(&report);
    report
}

pub fn dry_run_text(
    plan: &PlanDocument,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
) -> String {
    let report = dry_run_json(plan, context, options);
    render_dry_run_text(&report)
}

pub async fn dry_run_text_async(
    plan: &PlanDocument,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    detect_resolver: Option<&dyn DetectResolver>,
) -> String {
    let report = dry_run_json_async(plan, context, options, detect_resolver).await;
    render_dry_run_text(&report)
}

pub fn render_dry_run_text(report: &DryRunJsonReport) -> String {
    let mut lines = Vec::<String>::new();
    lines.push("AIS dry-run".to_string());
    lines.push(format!(
        "summary: total={} ready={} blocked={} skipped={} confirmations={}",
        report.summary.total_nodes,
        report.summary.ready_nodes,
        report.summary.blocked_nodes,
        report.summary.skipped_nodes,
        report.summary.estimated_confirmation_points
    ));
    lines.push(format!(
        "hashes: plan={} report={}",
        report.plan_hash, report.report_hash
    ));
    lines.push("nodes:".to_string());

    for node in &report.nodes {
        let state = node_state_label(node.readiness.state);
        let chain = node.chain.as_deref().unwrap_or("-");
        let kind = node.kind.as_deref().unwrap_or("-");
        let execution = node.execution_type.as_deref().unwrap_or("-");
        let writes = if node.writes.is_empty() {
            "-".to_string()
        } else {
            node.writes.join(",")
        };
        let risks = if node.risk_flags.is_empty() {
            "-".to_string()
        } else {
            node.risk_flags.join(",")
        };
        lines.push(format!(
            "- [{}] id={} chain={} kind={} exec={} writes={} risks={}",
            state, node.node_id, chain, kind, execution, writes, risks
        ));
        if !node.readiness.missing_refs.is_empty() {
            lines.push(format!(
                "  missing_refs={}",
                node.readiness.missing_refs.join(",")
            ));
        }
        if node.readiness.needs_detect {
            lines.push("  needs_detect=true".to_string());
        }
        if !node.readiness.errors.is_empty() {
            lines.push(format!("  errors={}", node.readiness.errors.join(" | ")));
        }
    }

    lines.push(format!("issues: {}", report.issues.len()));
    for issue in &report.issues {
        let node_id = issue.node_id.as_deref().unwrap_or("-");
        let reference = issue.reference.as_deref().unwrap_or("-");
        lines.push(format!(
            "- [{}] node={} ref={} {}",
            issue_severity_label(issue.severity),
            node_id,
            reference,
            issue.message
        ));
    }
    lines.join("\n")
}

fn build_node_report_sync(
    node: &Value,
    node_index: usize,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    issues: &mut Vec<StructuredIssue>,
) -> Option<DryRunNodeReport> {
    let node_object = match node.as_object() {
        Some(object) => object,
        None => {
            issues.push(issue(
                "dry_run_error",
                path_with_nodes_index(node_index),
                "plan node must be an object",
                "dry_run.node.invalid",
                None,
            ));
            return None;
        }
    };

    let node_id = node_object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if node_id.is_empty() {
        issues.push(issue(
            "dry_run_error",
            path_with_nodes_key(node_index, "id"),
            "plan node must include non-empty `id`",
            "dry_run.node.id_required",
            None,
        ));
        return None;
    }

    let readiness = get_node_readiness(node, context, options);
    push_readiness_issues(issues, node_index, &node_id, &readiness);

    Some(DryRunNodeReport {
        node_id,
        chain: node_object.get("chain").and_then(Value::as_str).map(str::to_string),
        kind: node_object.get("kind").and_then(Value::as_str).map(str::to_string),
        execution_type: node_object
            .get("execution")
            .and_then(Value::as_object)
            .and_then(|execution| execution.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string),
        writes: extract_writes(node_object.get("writes")),
        risk_flags: collect_risk_flags(node_object, &readiness),
        readiness,
    })
}

async fn build_node_report_async(
    node: &Value,
    node_index: usize,
    context: &ResolverContext,
    options: &ValueRefEvalOptions,
    detect_resolver: Option<&dyn DetectResolver>,
    issues: &mut Vec<StructuredIssue>,
) -> Option<DryRunNodeReport> {
    let node_object = match node.as_object() {
        Some(object) => object,
        None => {
            issues.push(issue(
                "dry_run_error",
                path_with_nodes_index(node_index),
                "plan node must be an object",
                "dry_run.node.invalid",
                None,
            ));
            return None;
        }
    };

    let node_id = node_object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if node_id.is_empty() {
        issues.push(issue(
            "dry_run_error",
            path_with_nodes_key(node_index, "id"),
            "plan node must include non-empty `id`",
            "dry_run.node.id_required",
            None,
        ));
        return None;
    }

    let readiness = get_node_readiness_async(node, context, options, detect_resolver).await;
    push_readiness_issues(issues, node_index, &node_id, &readiness);

    Some(DryRunNodeReport {
        node_id,
        chain: node_object.get("chain").and_then(Value::as_str).map(str::to_string),
        kind: node_object.get("kind").and_then(Value::as_str).map(str::to_string),
        execution_type: node_object
            .get("execution")
            .and_then(Value::as_object)
            .and_then(|execution| execution.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string),
        writes: extract_writes(node_object.get("writes")),
        risk_flags: collect_risk_flags(node_object, &readiness),
        readiness,
    })
}

fn summarize_reports(nodes: &[DryRunNodeReport]) -> DryRunSummary {
    let mut summary = DryRunSummary {
        total_nodes: nodes.len(),
        ..DryRunSummary::default()
    };

    for node in nodes {
        match node.readiness.state {
            NodeRunState::Ready => summary.ready_nodes += 1,
            NodeRunState::Blocked => summary.blocked_nodes += 1,
            NodeRunState::Skipped => summary.skipped_nodes += 1,
        }
        if !node.writes.is_empty() {
            summary.estimated_confirmation_points += 1;
        }
    }
    summary
}

fn push_readiness_issues(
    issues: &mut Vec<StructuredIssue>,
    node_index: usize,
    node_id: &str,
    readiness: &NodeReadinessResult,
) {
    if !readiness.missing_refs.is_empty() {
        issues.push(issue(
            "readiness_blocked",
            path_with_nodes_key(node_index, "execution"),
            &format!("missing refs: {}", readiness.missing_refs.join(",")),
            "dry_run.readiness_missing_refs",
            Some(node_id),
        ));
    }
    if readiness.needs_detect {
        issues.push(issue(
            "readiness_blocked",
            path_with_nodes_key(node_index, "execution"),
            "detect decision required before execution",
            "dry_run.readiness_needs_detect",
            Some(node_id),
        ));
    }
    for error in &readiness.errors {
        issues.push(issue(
            "readiness_error",
            path_with_nodes_key(node_index, "execution"),
            error,
            "dry_run.readiness_error",
            Some(node_id),
        ));
    }
}

fn extract_writes(raw_writes: Option<&Value>) -> Vec<String> {
    raw_writes
        .and_then(Value::as_array)
        .map(|items| {
            let mut writes = items
                .iter()
                .filter_map(|item| item.as_object())
                .filter_map(|item| item.get("path"))
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            writes.sort();
            writes.dedup();
            writes
        })
        .unwrap_or_default()
}

fn collect_risk_flags(
    node_object: &serde_json::Map<String, Value>,
    readiness: &NodeReadinessResult,
) -> Vec<String> {
    let mut flags = Vec::<String>::new();
    if node_object.contains_key("assert") {
        flags.push("assert".to_string());
    }
    if node_object.contains_key("until") {
        flags.push("polling".to_string());
    }
    if node_object
        .get("execution")
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind == "evm_call" || kind == "evm_multicall" || kind == "solana_instruction")
    {
        flags.push("write_execution".to_string());
    }
    if readiness.needs_detect {
        flags.push("needs_detect".to_string());
    }
    if !readiness.missing_refs.is_empty() {
        flags.push("missing_refs".to_string());
    }
    flags.sort();
    flags.dedup();
    flags
}

fn stable_hash_for_report(report: &DryRunJsonReport) -> String {
    let mut options = StableJsonOptions::default();
    options.ignore_object_keys.insert("report_hash".to_string());
    let value = serde_json::to_value(report).unwrap_or(Value::Null);
    stable_hash_hex(&value, &options).unwrap_or_else(|_| "unavailable".to_string())
}

fn stable_hash_for_value(
    value: &Value,
    reference: &str,
    issues: &mut Vec<StructuredIssue>,
) -> String {
    match stable_hash_hex(value, &StableJsonOptions::default()) {
        Ok(hash) => hash,
        Err(error) => {
            issues.push(StructuredIssue {
                kind: "dry_run_error".to_string(),
                severity: IssueSeverity::Error,
                node_id: None,
                field_path: FieldPath::root(),
                message: format!("stable hash failed: {error}"),
                reference: Some(reference.to_string()),
                related: None,
            });
            "unavailable".to_string()
        }
    }
}

fn path_with_nodes_index(index: usize) -> Vec<FieldPathSegment> {
    vec![
        FieldPathSegment::Key("nodes".to_string()),
        FieldPathSegment::Index(index),
    ]
}

fn path_with_nodes_key(index: usize, key: &str) -> Vec<FieldPathSegment> {
    vec![
        FieldPathSegment::Key("nodes".to_string()),
        FieldPathSegment::Index(index),
        FieldPathSegment::Key(key.to_string()),
    ]
}

fn issue(
    kind: &str,
    path: Vec<FieldPathSegment>,
    message: &str,
    reference: &str,
    node_id: Option<&str>,
) -> StructuredIssue {
    StructuredIssue {
        kind: kind.to_string(),
        severity: IssueSeverity::Error,
        node_id: node_id.map(str::to_string),
        field_path: FieldPath::from_segments(path),
        message: message.to_string(),
        reference: Some(reference.to_string()),
        related: None,
    }
}

fn node_state_label(state: NodeRunState) -> &'static str {
    match state {
        NodeRunState::Ready => "ready",
        NodeRunState::Blocked => "blocked",
        NodeRunState::Skipped => "skipped",
    }
}

fn issue_severity_label(severity: IssueSeverity) -> &'static str {
    match severity {
        IssueSeverity::Error => "error",
        IssueSeverity::Warning => "warning",
        IssueSeverity::Info => "info",
    }
}

#[cfg(test)]
#[path = "preview_test.rs"]
mod tests;
