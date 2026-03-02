use super::budget::compact_json_for_llm;
use crate::error::RunnerError;
use ais_engine::{EngineEventRecord, EngineEventType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannerOutputPattern {
    needle: &'static str,
    sub_reason_code: PlannerSubReasonCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlannerReasonCode {
    PlannerInvalidToolOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlannerSubReasonCode {
    InvalidToolArgs,
    SegmentNotJson,
    NoToolCalls,
    MissingCandidateRef,
    SegmentShapeInvalid,
    MissingSegment,
    MissingError,
    InvalidStatus,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExecutionReasonCode {
    ExecutionPaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExecutionSubReasonCode {
    MissingPausedReason,
    ExecutorError,
    AssertFailed,
    ConditionFailed,
    NonRetryablePause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompileSubReasonCode {
    UnknownInputRef,
    CandidateNotFound,
    MissingRequiredInput,
    ShapeOrContract,
    CompileError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ReasonCode {
    Known(KnownReasonCode),
    Raw(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KnownReasonCode {
    CompileError,
    MissingRequiredInput,
}

impl ReasonCode {
    fn from_raw(raw: &str) -> Self {
        serde_json::from_value::<Self>(Value::String(raw.to_string()))
            .unwrap_or_else(|_| Self::Raw(raw.to_string()))
    }

    fn compile_default() -> Self {
        Self::Known(KnownReasonCode::CompileError)
    }

    fn is_missing_required_input(&self) -> bool {
        matches!(self, Self::Known(KnownReasonCode::MissingRequiredInput))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExecutionPausePrefixPattern {
    prefix: &'static str,
    sub_reason_code: ExecutionSubReasonCode,
}

const RETRYABLE_PLANNER_OUTPUT_PATTERNS: &[PlannerOutputPattern] = &[
    PlannerOutputPattern {
        needle: "invalid plan.propose_segment args",
        sub_reason_code: PlannerSubReasonCode::InvalidToolArgs,
    },
    PlannerOutputPattern {
        needle: "invalid plan.revise_segment args",
        sub_reason_code: PlannerSubReasonCode::InvalidToolArgs,
    },
    PlannerOutputPattern {
        needle: "proposed segment draft `segment` must be a JSON object (stringified JSON is not allowed)",
        sub_reason_code: PlannerSubReasonCode::SegmentNotJson,
    },
    PlannerOutputPattern {
        needle: "proposed segment draft `segment` string must be valid JSON object text",
        sub_reason_code: PlannerSubReasonCode::SegmentNotJson,
    },
    PlannerOutputPattern {
        needle: "proposed segment draft `segment` must decode to a JSON object",
        sub_reason_code: PlannerSubReasonCode::SegmentNotJson,
    },
    PlannerOutputPattern {
        needle: "segmented planner provider returned no tool calls",
        sub_reason_code: PlannerSubReasonCode::NoToolCalls,
    },
    PlannerOutputPattern {
        needle: "steps missing required `candidate_ref`",
        sub_reason_code: PlannerSubReasonCode::MissingCandidateRef,
    },
    PlannerOutputPattern {
        needle: "proposed segment draft `segment` must be a valid PlanSketchSegment",
        sub_reason_code: PlannerSubReasonCode::SegmentShapeInvalid,
    },
    PlannerOutputPattern {
        needle: "proposed segment draft requires `segment`",
        sub_reason_code: PlannerSubReasonCode::MissingSegment,
    },
    PlannerOutputPattern {
        needle: "unavailable segment draft requires `error`",
        sub_reason_code: PlannerSubReasonCode::MissingError,
    },
    PlannerOutputPattern {
        needle: "invalid segment draft requires `error`",
        sub_reason_code: PlannerSubReasonCode::MissingError,
    },
    PlannerOutputPattern {
        needle: "invalid segment draft status",
        sub_reason_code: PlannerSubReasonCode::InvalidStatus,
    },
];

const RETRYABLE_EXECUTION_PAUSE_PREFIXES: &[ExecutionPausePrefixPattern] = &[
    ExecutionPausePrefixPattern {
        prefix: "executor_error:",
        sub_reason_code: ExecutionSubReasonCode::ExecutorError,
    },
    ExecutionPausePrefixPattern {
        prefix: "assert_failed:",
        sub_reason_code: ExecutionSubReasonCode::AssertFailed,
    },
    ExecutionPausePrefixPattern {
        prefix: "condition_failed:",
        sub_reason_code: ExecutionSubReasonCode::ConditionFailed,
    },
];

pub(super) fn should_attempt_intent_repair(paused_reason: Option<&str>) -> bool {
    classify_execution_pause(paused_reason).retryable
}

pub(super) fn should_retry_segmented_planner_output(error: &RunnerError) -> bool {
    classify_planner_output_error(error).is_some()
}

pub(super) fn segmented_planner_output_error_payload(
    error: &RunnerError,
    expected_finalize_tool: &str,
    round: u8,
    retry: u8,
    last_failed_finalize: Option<Value>,
) -> Value {
    let message = error.to_string();
    let sub_reason_code = classify_planner_output_error(error)
        .map(|pattern| pattern.sub_reason_code)
        .unwrap_or(PlannerSubReasonCode::Unknown);
    compact_json_for_llm(&json!({
        "phase": "planning",
        "reason_code": PlannerReasonCode::PlannerInvalidToolOutput,
        "sub_reason_code": sub_reason_code,
        "phase_reason_code": format!("planning.{}", code_as_str(&sub_reason_code)),
        "message": message,
        "expected_finalize_tool": expected_finalize_tool,
        "repair_order": ["shape", "ref", "slot", "semantic"],
        "hint": planner_hint_for_sub_reason(sub_reason_code),
        "round": round,
        "retry": retry,
        "last_failed_finalize": last_failed_finalize,
    }))
}

pub(super) fn intent_execution_error_payload(
    paused_reason: Option<&str>,
    events: &[EngineEventRecord],
    round: u8,
) -> Value {
    let classification = classify_execution_pause(paused_reason);
    let last_error = events
        .iter()
        .rev()
        .find(|record| record.event.event_type == EngineEventType::Error)
        .map(|record| json!({"node_id": record.event.node_id, "data": record.event.data}));
    compact_json_for_llm(&json!({
        "phase": "execution",
        "reason_code": ExecutionReasonCode::ExecutionPaused,
        "sub_reason_code": classification.sub_reason_code,
        "retryable": classification.retryable,
        "paused_reason": paused_reason,
        "round": round,
        "last_error": last_error,
    }))
}

pub(super) fn compile_error_state_payload(error_payload: &Value, round: u8) -> Value {
    let reason_code = error_payload
        .get("reason_code")
        .and_then(Value::as_str)
        .map(ReasonCode::from_raw)
        .unwrap_or_else(ReasonCode::compile_default);
    let message = error_payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("segment compile failed");
    let issues = error_payload
        .get("issues")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let sub_reason_code = classify_compile_sub_reason_code(&reason_code, &issues);
    compact_json_for_llm(&json!({
        "phase": "compile",
        "reason_code": reason_code,
        "sub_reason_code": sub_reason_code,
        "phase_reason_code": format!("compile.{}", code_as_str(&sub_reason_code)),
        "message": message,
        "issues": issues,
        "repair_order": ["shape", "ref", "slot", "semantic"],
        "round": round,
    }))
}

pub(super) fn grounding_phase_error_payload(
    reason_code: &str,
    message: Option<&str>,
    issues: &[Value],
    questions: &[Value],
    round: u8,
) -> Value {
    let base_reason_code = ReasonCode::from_raw(reason_code);
    compact_json_for_llm(&json!({
        "phase": "grounding",
        "reason_code": format!("grounding.{}", code_as_str(&base_reason_code)),
        "base_reason_code": base_reason_code,
        "message": message,
        "issues": issues,
        "questions": questions,
        "round": round,
    }))
}

pub(super) fn todo_phase_error_payload(
    reason_code: &str,
    message: Option<&str>,
    issues: &[Value],
    questions: &[Value],
    round: u8,
) -> Value {
    let base_reason_code = ReasonCode::from_raw(reason_code);
    compact_json_for_llm(&json!({
        "phase": "todo_planning",
        "reason_code": format!("todo.{}", code_as_str(&base_reason_code)),
        "base_reason_code": base_reason_code,
        "message": message,
        "issues": issues,
        "questions": questions,
        "round": round,
    }))
}

fn classify_compile_sub_reason_code(
    reason_code: &ReasonCode,
    issues: &Value,
) -> CompileSubReasonCode {
    let issue_items = issues.as_array().cloned().unwrap_or_default();
    if issue_items
        .iter()
        .any(|item| item.get("reference").and_then(Value::as_str) == Some("unknown_input_ref"))
    {
        return CompileSubReasonCode::UnknownInputRef;
    }
    if issue_items.iter().any(|item| {
        item.get("reference")
            .and_then(Value::as_str)
            .is_some_and(|reference| reference.contains("candidate_not_found"))
    }) {
        return CompileSubReasonCode::CandidateNotFound;
    }
    if issue_items.iter().any(|item| {
        item.get("reference")
            .and_then(Value::as_str)
            .is_some_and(|reference| reference.contains("missing_required_input"))
    }) || reason_code.is_missing_required_input()
    {
        return CompileSubReasonCode::MissingRequiredInput;
    }
    if issue_items.iter().any(|item| {
        item.get("reference")
            .and_then(Value::as_str)
            .is_some_and(|reference| {
                reference.contains("input_type_mismatch")
                    || reference.contains("constraint_violation")
                    || reference.contains("write_gate_missing")
            })
    }) {
        return CompileSubReasonCode::ShapeOrContract;
    }
    CompileSubReasonCode::CompileError
}

fn classify_planner_output_error(error: &RunnerError) -> Option<PlannerOutputPattern> {
    let RunnerError::Llm(message) = error else {
        return None;
    };
    RETRYABLE_PLANNER_OUTPUT_PATTERNS
        .iter()
        .find(|pattern| message.contains(pattern.needle))
        .copied()
}

fn planner_hint_for_sub_reason(sub_reason_code: PlannerSubReasonCode) -> Value {
    match sub_reason_code {
        PlannerSubReasonCode::MissingSegment => json!({
            "summary": "status=proposed must include segment",
            "expected": {"status":"proposed","done":false,"segment":{"segment_id":"...","cursor_in":"...","cursor_out":"...","done":false,"steps":[{"id":"...","kind":"query","candidate_ref":"...","inputs":{}}]}}
        }),
        PlannerSubReasonCode::MissingError => json!({
            "summary": "status=invalid|unavailable must include error.reason_code",
            "expected": {"status":"invalid","done":false,"error":{"reason_code":"...","message":"..."}}
        }),
        PlannerSubReasonCode::InvalidToolArgs => json!({
            "summary": "finalize tool arguments did not match schema",
            "note": "Keep fields minimal; avoid extra keys; ensure types match (strings for ids/cursors; boolean for done)."
        }),
        PlannerSubReasonCode::SegmentNotJson => json!({
            "summary": "segment was a string but not valid JSON",
            "note": "Return segment as an object, or a JSON string that decodes to an object."
        }),
        PlannerSubReasonCode::NoToolCalls => json!({
            "summary": "model response did not include tool calls",
            "note": "Always emit exactly one final tool call in each round."
        }),
        PlannerSubReasonCode::SegmentShapeInvalid => json!({
            "summary": "segment JSON did not match required fields",
            "required_segment_fields": ["segment_id","cursor_in","cursor_out","done","steps"],
            "step_forbidden_fields": ["if_true","if_false","then","else","children","steps_if_true","steps_if_false"],
            "branch_fix": "Do not nest branch trees. Keep steps flat and use when.cel + depends_on to model path selection."
        }),
        PlannerSubReasonCode::MissingCandidateRef => json!({
            "summary": "one or more steps are missing candidate_ref",
            "required_step_fields": ["id","kind","inputs"],
            "rules": [
                "query/action steps require candidate_ref.",
                "assert/branch are built-in control steps and may omit candidate_ref.",
                "when candidate_ref is present, it must come from discovered candidates (catalog.search/get_candidate_detail).",
                "Do not invent refs; reuse known candidate refs and keep semantics unchanged while fixing shape."
            ]
        }),
        PlannerSubReasonCode::InvalidStatus => json!({
            "summary": "status must be one of: proposed|invalid|unavailable",
            "allowed": ["proposed","invalid","unavailable"]
        }),
        _ => json!({
            "rules": [
                "Return exactly one finalize tool call and make it the last tool call in the round.",
                "If status=proposed: include a non-empty segment with steps[].",
                "If status=invalid|unavailable: include error.reason_code and omit segment.",
                "Do not add unknown fields; follow the tool input schema exactly."
            ]
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExecutionPauseClassification {
    sub_reason_code: ExecutionSubReasonCode,
    retryable: bool,
}

fn classify_execution_pause(paused_reason: Option<&str>) -> ExecutionPauseClassification {
    let Some(reason) = paused_reason else {
        return ExecutionPauseClassification {
            sub_reason_code: ExecutionSubReasonCode::MissingPausedReason,
            retryable: false,
        };
    };
    for pattern in RETRYABLE_EXECUTION_PAUSE_PREFIXES {
        if reason.starts_with(pattern.prefix) {
            return ExecutionPauseClassification {
                sub_reason_code: pattern.sub_reason_code,
                retryable: true,
            };
        }
    }
    ExecutionPauseClassification {
        sub_reason_code: ExecutionSubReasonCode::NonRetryablePause,
        retryable: false,
    }
}

fn code_as_str<T: Serialize>(code: &T) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_string())
}
