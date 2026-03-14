use ais_agent_control::execution_artifact::{ComparisonOperator, PredicateSpec};
use ais_agent_expr::cel::CelEvaluator;

use crate::{
    runtime::ActiveRun,
    runtime_value_resolver::{cel_scope_with_refs, resolve_value_ref},
};

pub(crate) fn evaluate_predicate(
    runtime: &ActiveRun,
    predicate: &PredicateSpec,
) -> Result<bool, String> {
    match predicate {
        PredicateSpec::Comparison { left, op, right } => {
            let mut scope = cel_scope_with_refs(runtime)?;
            scope.insert_json("left", resolve_value_ref(runtime, left)?);
            scope.insert_json("right", resolve_value_ref(runtime, right)?);
            let expression = match op {
                ComparisonOperator::Eq => "left == right",
                ComparisonOperator::Ne => "left != right",
                ComparisonOperator::Gt => "left > right",
                ComparisonOperator::Gte => "left >= right",
                ComparisonOperator::Lt => "left < right",
                ComparisonOperator::Lte => "left <= right",
            };
            let mut evaluator = CelEvaluator::new();
            evaluator
                .evaluate_bool(expression, &scope)
                .map_err(|error| format!("execution_artifact comparison failed: {error}"))
        }
        PredicateSpec::Cel { expression } => {
            let mut evaluator = CelEvaluator::new();
            evaluator
                .evaluate_bool(expression, &cel_scope_with_refs(runtime)?)
                .map_err(|error| format!("execution_artifact CEL `{expression}` failed: {error}"))
        }
        PredicateSpec::Freshness {
            evidence_ref,
            max_age_ms,
        } => Ok(runtime
            .checkpoint
            .evidence_graph
            .records
            .get(evidence_ref)
            .and_then(|record| record.freshness.observed_at_ms)
            .is_some_and(|observed_at_ms| {
                current_time_ms().saturating_sub(observed_at_ms) <= *max_age_ms
            })),
        PredicateSpec::ReceiptStatus {
            receipt_ref,
            expected,
        } => {
            let status = runtime
                .checkpoint
                .evidence_graph
                .records
                .get(receipt_ref)
                .and_then(|record| record.payload.get("status"))
                .and_then(|value| value.as_bool());
            Ok(matches!(
                (status, expected),
                (
                    Some(true),
                    ais_agent_control::execution_artifact::ReceiptStatusExpectation::Success
                )
            ))
        }
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
