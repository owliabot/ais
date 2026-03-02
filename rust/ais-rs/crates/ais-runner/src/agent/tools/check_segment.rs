use serde_json::{json, Value};

use super::super::intent_segmented::PlannerRoundPhase;

pub(crate) fn requires_successful_check_before_finalize(
    phase: PlannerRoundPhase,
    has_segment_check_context: bool,
) -> bool {
    has_segment_check_context
        && matches!(
            phase,
            PlannerRoundPhase::ProposeSegment | PlannerRoundPhase::ReviseSegment
        )
}

pub(crate) fn missing_pre_finalize_check_payload(finalize_tool: &str) -> Value {
    json!({
        "error": {
            "code": "missing_pre_finalize_check_segment",
            "message": format!("call plan.check_segment and wait for ok=true before `{finalize_tool}`"),
            "required_tool": "plan.check_segment",
            "required_ok": true,
            "blocked_finalize": finalize_tool
        }
    })
}

pub(crate) fn pre_finalize_segment_mismatch_payload(
    finalize_tool: &str,
    checked_signature: Option<&str>,
    finalized_signature: Option<&str>,
) -> Value {
    json!({
        "error": {
            "code": "pre_finalize_segment_mismatch",
            "message": format!("segment changed after plan.check_segment; re-run plan.check_segment on the current segment before `{finalize_tool}`"),
            "required_tool": "plan.check_segment",
            "required_ok": true,
            "blocked_finalize": finalize_tool,
            "checked_segment_signature": checked_signature,
            "finalized_segment_signature": finalized_signature,
        }
    })
}

pub(crate) fn repeated_plan_check_failure_payload(
    content: &str,
    streak: u64,
    threshold: u64,
    finalize_tool: &str,
) -> Value {
    let fallback_reason_code = if finalize_tool == "plan.propose_segment" {
        "check_segment_repeated_failure_propose"
    } else {
        "check_segment_repeated_failure_revise"
    };
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return json!({
            "error": {
                "code": "repeated_plan_check_failure",
                "message": "plan.check_segment returned repeated malformed payload; aborting revise loop for deterministic repair",
                "reason_code": fallback_reason_code,
                "streak": streak,
                "threshold": threshold,
            }
        });
    };
    let mut issue_keys = plan_check_issue_summaries(&value)
        .into_iter()
        .map(|summary| format!("{}#{}", summary.reason_code, summary.step_id))
        .collect::<Vec<_>>();
    issue_keys.sort();
    issue_keys.dedup();
    let top_level_reason_code = value
        .pointer("/error/reason_code")
        .and_then(Value::as_str)
        .or_else(|| value.get("reason_code").and_then(Value::as_str))
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_string);
    let reason_code = top_level_reason_code.unwrap_or_else(|| {
        if issue_keys.is_empty() {
            fallback_reason_code.to_string()
        } else {
            format!("{fallback_reason_code}:{}", issue_keys.join("|"))
        }
    });
    let issue_summaries = plan_check_issue_summaries(&value);
    let mut issue_reason_codes = issue_summaries
        .iter()
        .map(|issue| issue.reason_code.clone())
        .collect::<Vec<_>>();
    issue_reason_codes.sort();
    issue_reason_codes.dedup();
    let mut step_ids = issue_summaries
        .iter()
        .map(|issue| issue.step_id.clone())
        .filter(|step_id| !step_id.is_empty())
        .collect::<Vec<_>>();
    step_ids.sort();
    step_ids.dedup();
    let mut suggested_fixes = vec![
        "Use candidate refs returned by discovery tools and avoid synthetic refs for assert/branch."
            .to_string(),
        format!("Call plan.check_segment again and finalize `{finalize_tool}` only when ok=true."),
    ];
    if issue_reason_codes
        .iter()
        .any(|code| code == "missing_action_gate_dep")
    {
        suggested_fixes.insert(
            1,
            "For each write action issue, add depends_on with the relevant gate step id."
                .to_string(),
        );
    }
    if issue_reason_codes
        .iter()
        .any(|code| code == "missing_gate_query_dep" || code == "missing_query_assert_branch_chain")
    {
        suggested_fixes.insert(
            2,
            "For each reported gate step, add depends_on to one or more query step ids."
                .to_string(),
        );
    }
    json!({
        "error": {
            "code": "repeated_plan_check_failure",
            "message": "plan.check_segment returned the same failure repeatedly; aborting revise loop for deterministic repair",
            "reason_code": reason_code,
            "streak": streak,
            "threshold": threshold,
            "issue_reason_codes": issue_reason_codes,
            "step_ids": step_ids,
            "issues": issue_summaries.iter().map(|issue| issue.as_json()).collect::<Vec<_>>(),
            "suggested_fixes": suggested_fixes,
        }
    })
}

pub(crate) fn plan_check_has_control_step_candidate_not_found(content: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return false;
    };
    let issues = value
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    issues.iter().any(|issue| {
        let reference = issue
            .get("reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = issue
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        reference == "candidate_not_found"
            && message.contains("candidate not found for control step")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanCheckIssueSummary {
    reason_code: String,
    step_id: String,
    message: String,
    reference: String,
    path: String,
    suggested_ref: Option<String>,
    candidates: Vec<String>,
}

impl PlanCheckIssueSummary {
    fn as_json(&self) -> Value {
        let mut value = json!({
            "reason_code": self.reason_code,
            "step_id": self.step_id,
            "message": self.message,
            "reference": self.reference,
            "path": self.path,
        });
        if let Some(object) = value.as_object_mut() {
            if let Some(suggested_ref) = self.suggested_ref.clone() {
                object.insert("suggested_ref".to_string(), Value::String(suggested_ref));
            }
            if !self.candidates.is_empty() {
                object.insert(
                    "candidates".to_string(),
                    Value::Array(
                        self.candidates
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect::<Vec<_>>(),
                    ),
                );
            }
        }
        value
    }
}

fn plan_check_issue_summaries(value: &Value) -> Vec<PlanCheckIssueSummary> {
    let issues = value
        .get("issues")
        .and_then(Value::as_array)
        .or_else(|| value.pointer("/error/issues").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    issues
        .iter()
        .map(|item| PlanCheckIssueSummary {
            reason_code: item
                .get("gate_reason_code")
                .and_then(Value::as_str)
                .or_else(|| item.get("reason_code").and_then(Value::as_str))
                .unwrap_or("unknown")
                .to_string(),
            step_id: item
                .get("step_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: item
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            reference: item
                .get("reference")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            path: item
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            suggested_ref: item
                .get("suggested_ref")
                .and_then(Value::as_str)
                .map(str::to_string),
            candidates: item
                .get("candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|items| items.iter())
                .filter_map(Value::as_str)
                .take(3)
                .map(str::to_string)
                .collect::<Vec<_>>(),
        })
        .collect::<Vec<_>>()
}
