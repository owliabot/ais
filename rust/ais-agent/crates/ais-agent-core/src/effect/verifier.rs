use serde::{Deserialize, Serialize};
use serde_json::Value;

use ais_agent_expr::cel::{CelEvaluationError, CelEvaluator, CelScope};

use crate::effect::{EffectContract, EffectDelta, EffectDeltaStatus};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EffectObservationBundle {
    pub pre: Option<Value>,
    pub post: Option<Value>,
    pub receipt: Option<Value>,
    pub expected: Option<Value>,
    pub context: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectVerificationResult {
    #[serde(default)]
    pub deltas: Vec<EffectDelta>,
    pub final_status: EffectDeltaStatus,
    pub final_summary: String,
}

pub fn verify_effect_contract(
    contract: &EffectContract,
    observations: &EffectObservationBundle,
) -> EffectVerificationResult {
    let mut evaluator = CelEvaluator::new();
    let mut deltas = Vec::with_capacity(contract.assertions.len());

    for assertion in &contract.assertions {
        let mut scope = CelScope::new();
        if let Some(pre) = observations.pre.clone() {
            scope.insert_json("pre", pre);
        }
        if let Some(post) = observations.post.clone() {
            scope.insert_json("post", post);
        }
        if let Some(receipt) = observations.receipt.clone() {
            scope.insert_json("receipt", receipt);
        }
        if let Some(expected) = observations.expected.clone() {
            scope.insert_json("expected", expected);
        }
        if let Some(context) = observations.context.clone() {
            scope.insert_json("ctx", context);
        }

        deltas.push(
            match evaluator.evaluate_bool(assertion.expression.as_str(), &scope) {
                Ok(true) => EffectDelta {
                    effect_id: contract.effect_id.clone(),
                    assertion_description: Some(assertion.description.clone()),
                    status: EffectDeltaStatus::Satisfied,
                    summary: format!("assertion satisfied: {}", assertion.description),
                    missing_bindings: Vec::new(),
                },
                Ok(false) => EffectDelta {
                    effect_id: contract.effect_id.clone(),
                    assertion_description: Some(assertion.description.clone()),
                    status: EffectDeltaStatus::Violated,
                    summary: format!("assertion violated: {}", assertion.description),
                    missing_bindings: Vec::new(),
                },
                Err(error) => match classify_eval_error(error) {
                    VerificationEvalClassification::MissingBindings(bindings) => EffectDelta {
                        effect_id: contract.effect_id.clone(),
                        assertion_description: Some(assertion.description.clone()),
                        status: EffectDeltaStatus::UnknownDueToMissingObservation,
                        summary: format!(
                            "missing observations prevented verification: {}",
                            assertion.description
                        ),
                        missing_bindings: bindings,
                    },
                    VerificationEvalClassification::HardFailure(message) => EffectDelta {
                        effect_id: contract.effect_id.clone(),
                        assertion_description: Some(assertion.description.clone()),
                        status: EffectDeltaStatus::Violated,
                        summary: format!("verification error: {message}"),
                        missing_bindings: Vec::new(),
                    },
                },
            },
        );
    }

    let final_status = summarize_status(&deltas);
    let final_summary = match final_status {
        EffectDeltaStatus::Satisfied => "all effect assertions satisfied".to_owned(),
        EffectDeltaStatus::Violated => "one or more effect assertions were violated".to_owned(),
        EffectDeltaStatus::UnknownDueToMissingObservation => {
            "effect verification incomplete due to missing observations".to_owned()
        }
        EffectDeltaStatus::Pending => "effect verification pending".to_owned(),
    };

    EffectVerificationResult {
        deltas,
        final_status,
        final_summary,
    }
}

fn summarize_status(deltas: &[EffectDelta]) -> EffectDeltaStatus {
    if deltas.is_empty() {
        return EffectDeltaStatus::Pending;
    }

    if deltas
        .iter()
        .any(|delta| delta.status == EffectDeltaStatus::Violated)
    {
        return EffectDeltaStatus::Violated;
    }

    if deltas
        .iter()
        .any(|delta| delta.status == EffectDeltaStatus::UnknownDueToMissingObservation)
    {
        return EffectDeltaStatus::UnknownDueToMissingObservation;
    }

    if deltas
        .iter()
        .all(|delta| delta.status == EffectDeltaStatus::Satisfied)
    {
        return EffectDeltaStatus::Satisfied;
    }

    EffectDeltaStatus::Pending
}

enum VerificationEvalClassification {
    MissingBindings(Vec<String>),
    HardFailure(String),
}

fn classify_eval_error(error: CelEvaluationError) -> VerificationEvalClassification {
    match error {
        CelEvaluationError::Eval(runtime) => {
            let rendered = runtime.to_string();
            if let Some(name) = rendered.strip_prefix("undefined identifier: ") {
                return VerificationEvalClassification::MissingBindings(vec![name.to_owned()]);
            }
            if rendered.contains("invalid member access:") {
                return VerificationEvalClassification::MissingBindings(vec![rendered]);
            }
            VerificationEvalClassification::HardFailure(rendered)
        }
        CelEvaluationError::ExpectedBool => VerificationEvalClassification::HardFailure(
            "effect assertion did not evaluate to bool".to_owned(),
        ),
    }
}
