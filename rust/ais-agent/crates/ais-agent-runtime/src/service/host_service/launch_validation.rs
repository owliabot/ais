use std::collections::BTreeMap;

use ais_agent_control::execution_artifact::{
    BranchStage, BranchTarget, EffectSpec, EvmTransactionCandidate, ExecutionArtifactLaunchSpec,
    ExecutionChainFamily, ExecutionStage, ExecutionTransactionCandidate, ObservationSpec,
    ObserveStage, PredicateSpec, SolanaTransactionCandidate, TransactionStage, ValueRef,
};
use ais_agent_control::launch_spec::{
    LaunchSpecSubmission, PrebuiltFragmentLaunchSpec, ReflectionRequestLaunchSpec,
};
use ais_agent_core::{action::ActionGraph, effect::EffectContract, evidence::EvidenceGraph};
use ais_agent_expr::cel::{CelExpressionKind, CelTypeChecker};
use serde_json::Value as JsonValue;

use super::launch_binding;

#[derive(Debug)]
pub(crate) struct ValidatedPrebuiltFragment {
    pub action_graph: Option<ActionGraph>,
    pub evidence_graph: Option<EvidenceGraph>,
    pub effect_contracts: Option<BTreeMap<String, EffectContract>>,
}

pub(crate) fn validate_launch_spec_submission(
    launch_spec: &LaunchSpecSubmission,
) -> Result<Option<ValidatedPrebuiltFragment>, String> {
    match launch_spec {
        LaunchSpecSubmission::PrebuiltFragment(spec) => Ok(Some(validate_prebuilt_fragment(spec)?)),
        LaunchSpecSubmission::ReflectionRequest(spec) => Err(format!(
            "reflection_request launch specs are not implemented yet: {}",
            summarize_reflection_request(spec)
        )),
        LaunchSpecSubmission::ExecutionArtifact(spec) => {
            validate_execution_artifact(spec)?;
            Ok(None)
        }
    }
}

fn validate_prebuilt_fragment(
    spec: &PrebuiltFragmentLaunchSpec,
) -> Result<ValidatedPrebuiltFragment, String> {
    let validated = launch_binding::parse_prebuilt_fragment(spec)?;

    if let Some(effect_contracts) = &validated.effect_contracts {
        for (effect_ref, contract) in effect_contracts {
            if contract.effect_id != *effect_ref {
                return Err(format!(
                    "prebuilt_fragment.effect_contracts key `{effect_ref}` does not match contract.effect_id `{}`",
                    contract.effect_id
                ));
            }
        }
    }

    Ok(validated)
}

fn validate_execution_artifact(spec: &ExecutionArtifactLaunchSpec) -> Result<(), String> {
    if spec.protocol_package_id.trim().is_empty() {
        return Err("execution_artifact.protocol_package_id must not be empty".to_owned());
    }

    if spec.action_key.trim().is_empty() {
        return Err("execution_artifact.action_key must not be empty".to_owned());
    }

    if spec.entry_stage_id.trim().is_empty() {
        return Err("execution_artifact.entry_stage_id must not be empty".to_owned());
    }

    if spec
        .allowed_chains
        .iter()
        .any(|chain| chain.trim().is_empty())
    {
        return Err("execution_artifact.allowed_chains entries must not be blank".to_owned());
    }

    if spec.allowed_chains.len() != 1 {
        return Err(
            "execution_artifact.allowed_chains must contain exactly one active chain scope"
                .to_owned(),
        );
    }

    if spec.stages.is_empty() {
        return Err("execution_artifact.stages must not be empty".to_owned());
    }

    if spec.transactions.is_empty() && !spec.stages.iter().any(|stage| stage.as_observe().is_some())
    {
        return Err(
            "execution_artifact must provide at least one transaction or observe stage".to_owned(),
        );
    }

    let candidate_ids: Vec<&str> = spec
        .transactions
        .iter()
        .map(|candidate| candidate_id(spec.chain_family, candidate))
        .collect::<Result<Vec<_>, _>>()?;
    ensure_unique("execution_artifact.transactions", &candidate_ids)?;

    let stage_ids: Vec<&str> = spec
        .stages
        .iter()
        .map(stage_id)
        .collect::<Result<Vec<_>, _>>()?;
    ensure_unique("execution_artifact.stages", &stage_ids)?;

    if !stage_ids
        .iter()
        .any(|stage_id| stage_id == &spec.entry_stage_id.as_str())
    {
        return Err(format!(
            "execution_artifact.entry_stage_id references unknown stage `{}`",
            spec.entry_stage_id
        ));
    }

    let exported_output_keys = collect_export_keys(spec)?;
    let observation_ids = validate_observations(spec)?;
    validate_effects(spec, &stage_ids)?;

    for stage in &spec.stages {
        validate_stage(
            stage,
            &candidate_ids,
            &observation_ids,
            &stage_ids,
            &exported_output_keys,
            "execution_artifact.stages",
        )?;
    }

    validate_semantic_contract(spec, &candidate_ids)?;

    Ok(())
}

fn validate_semantic_contract(
    spec: &ExecutionArtifactLaunchSpec,
    candidate_ids: &[&str],
) -> Result<(), String> {
    if !spec.semantic_contract_active() {
        return Ok(());
    }

    let Some(risk_class) = spec.risk_class.as_deref() else {
        return Err(
            "execution_artifact.risk_class is required when semantic artifact fields are present"
                .to_owned(),
        );
    };
    if risk_class.trim().is_empty() {
        return Err(
            "execution_artifact.risk_class must not be blank when present".to_owned(),
        );
    }

    if spec.decoded_intent.is_none() {
        return Err(
            "execution_artifact.decoded_intent is required when semantic artifact fields are present"
                .to_owned(),
        );
    }

    if spec.candidate_envelopes.is_empty() {
        return Err(
            "execution_artifact.candidate_envelopes must include at least one envelope when semantic artifact fields are present"
                .to_owned(),
        );
    }

    if spec.validation_plan.is_none() {
        return Err(
            "execution_artifact.validation_plan is required when semantic artifact fields are present"
                .to_owned(),
        );
    }

    for (index, envelope) in spec.candidate_envelopes.iter().enumerate() {
        validate_candidate_envelope(envelope, index, candidate_ids)?;
    }

    Ok(())
}

fn validate_candidate_envelope(
    envelope: &JsonValue,
    index: usize,
    candidate_ids: &[&str],
) -> Result<(), String> {
    let Some(candidate_ref) = envelope
        .as_object()
        .ok_or_else(|| {
            format!(
                "execution_artifact.candidate_envelopes[{index}] must be a JSON object"
            )
        })?
        .get("candidate_ref")
    else {
        return Ok(());
    };

    let candidate_ref = candidate_ref.as_str().ok_or_else(|| {
        format!(
            "execution_artifact.candidate_envelopes[{index}].candidate_ref must be a string"
        )
    })?;
    if candidate_ref.trim().is_empty() {
        return Err(format!(
            "execution_artifact.candidate_envelopes[{index}].candidate_ref must not be blank"
        ));
    }
    if !candidate_ids.iter().any(|known| known == &candidate_ref) {
        return Err(format!(
            "execution_artifact.candidate_envelopes[{index}].candidate_ref references unknown candidate `{candidate_ref}`"
        ));
    }

    Ok(())
}

fn validate_observations(spec: &ExecutionArtifactLaunchSpec) -> Result<Vec<&str>, String> {
    let observation_ids = spec
        .observations
        .iter()
        .chain(spec.preconditions.iter())
        .chain(spec.postconditions.iter())
        .map(|spec| validate_observation_spec(spec))
        .collect::<Result<Vec<_>, _>>()?;
    ensure_unique("execution_artifact.observations", &observation_ids)?;
    Ok(observation_ids)
}

fn validate_effects(spec: &ExecutionArtifactLaunchSpec, stage_ids: &[&str]) -> Result<(), String> {
    let effect_ids = spec
        .expected_effects
        .iter()
        .map(|effect| validate_effect_spec(effect, spec, stage_ids))
        .collect::<Result<Vec<_>, _>>()?;
    ensure_unique("execution_artifact.expected_effects", &effect_ids)?;

    let mut stage_effects = BTreeMap::<&str, usize>::new();
    for effect in &spec.expected_effects {
        *stage_effects.entry(effect.stage_id.as_str()).or_default() += 1;
    }
    if let Some((stage_id, _)) = stage_effects.into_iter().find(|(_, count)| *count > 1) {
        return Err(format!(
            "execution_artifact.expected_effects currently supports at most one effect per transaction stage; stage `{stage_id}` has multiple entries"
        ));
    }

    Ok(())
}

fn summarize_reflection_request(spec: &ReflectionRequestLaunchSpec) -> String {
    if spec.request.is_null() {
        "empty request".to_owned()
    } else {
        "request payload provided".to_owned()
    }
}

fn candidate_id(
    chain_family: ExecutionChainFamily,
    candidate: &ExecutionTransactionCandidate,
) -> Result<&str, String> {
    match (chain_family, candidate) {
        (ExecutionChainFamily::Evm, ExecutionTransactionCandidate::EvmTransaction(candidate)) => {
            validate_evm_transaction_candidate(candidate)?;
            Ok(candidate.candidate_id.as_str())
        }
        (
            ExecutionChainFamily::Solana,
            ExecutionTransactionCandidate::SolanaTransaction(candidate),
        ) => {
            validate_solana_transaction_candidate(candidate)?;
            Ok(candidate.candidate_id.as_str())
        }
        (ExecutionChainFamily::Evm, ExecutionTransactionCandidate::SolanaTransaction(_)) => Err(
            "execution_artifact.transactions contains solana_transaction under evm chain_family"
                .to_owned(),
        ),
        (ExecutionChainFamily::Solana, ExecutionTransactionCandidate::EvmTransaction(_)) => Err(
            "execution_artifact.transactions contains evm_transaction under solana chain_family"
                .to_owned(),
        ),
    }
}

fn validate_evm_transaction_candidate(candidate: &EvmTransactionCandidate) -> Result<(), String> {
    if candidate.candidate_id.trim().is_empty() {
        return Err("execution_artifact.transactions candidate_id must not be empty".to_owned());
    }
    if candidate.to.trim().is_empty() {
        return Err("execution_artifact.transactions to must not be empty".to_owned());
    }
    if matches!(candidate.value.as_deref(), Some(value) if value.trim().is_empty()) {
        return Err(
            "execution_artifact.transactions value must not be blank when present".to_owned(),
        );
    }
    if matches!(candidate.calldata.as_deref(), Some(value) if value.trim().is_empty()) {
        return Err(
            "execution_artifact.transactions calldata must not be blank when present".to_owned(),
        );
    }
    if candidate.value.is_none() && candidate.calldata.is_none() {
        return Err(
            "execution_artifact.transactions must provide value or calldata for each candidate"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_solana_transaction_candidate(
    candidate: &SolanaTransactionCandidate,
) -> Result<(), String> {
    if candidate.candidate_id.trim().is_empty() {
        return Err("execution_artifact.transactions candidate_id must not be empty".to_owned());
    }
    if candidate.instructions.is_empty() {
        return Err(
            "execution_artifact.transactions solana_transaction.instructions must not be empty"
                .to_owned(),
        );
    }
    for instruction in &candidate.instructions {
        if instruction.program_id.trim().is_empty() {
            return Err(
                "execution_artifact.transactions solana_transaction.program_id must not be empty"
                    .to_owned(),
            );
        }
        if matches!(instruction.data_base64.as_deref(), Some(value) if value.trim().is_empty()) {
            return Err(
                "execution_artifact.transactions solana_transaction.data_base64 must not be blank when present"
                    .to_owned(),
            );
        }
        for account in &instruction.accounts {
            if account.address.trim().is_empty() {
                return Err(
                    "execution_artifact.transactions solana_transaction.accounts.address must not be empty"
                        .to_owned(),
                );
            }
        }
    }
    Ok(())
}

fn stage_id(stage: &ExecutionStage) -> Result<&str, String> {
    let stage_id = stage.stage_id().as_str();

    if stage_id.trim().is_empty() {
        Err("execution_artifact.stages stage_id must not be empty".to_owned())
    } else {
        Ok(stage_id)
    }
}

fn collect_export_keys(spec: &ExecutionArtifactLaunchSpec) -> Result<Vec<&str>, String> {
    let export_keys = spec
        .stages
        .iter()
        .flat_map(|stage| match stage {
            ExecutionStage::Transaction(stage) => stage.exports.iter().collect::<Vec<_>>(),
            ExecutionStage::Observe(stage) => stage.exports.iter().collect::<Vec<_>>(),
            ExecutionStage::Branch(_) | ExecutionStage::Continuation(_) => Vec::new(),
        })
        .map(|export| {
            if export.output_key.trim().is_empty() {
                Err("execution_artifact.stages.exports output_key must not be empty".to_owned())
            } else {
                Ok(export.output_key.as_str())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    ensure_unique("execution_artifact.stages.exports", &export_keys)?;
    Ok(export_keys)
}

fn validate_stage(
    stage: &ExecutionStage,
    candidate_ids: &[&str],
    observation_ids: &[&str],
    stage_ids: &[&str],
    export_keys: &[&str],
    field_path: &str,
) -> Result<(), String> {
    match stage {
        ExecutionStage::Transaction(stage) => {
            validate_transaction_stage(stage, candidate_ids, stage_ids, field_path)
        }
        ExecutionStage::Observe(stage) => {
            validate_observe_stage(stage, observation_ids, stage_ids, field_path)
        }
        ExecutionStage::Branch(stage) => validate_branch_stage(stage, stage_ids, field_path),
        ExecutionStage::Continuation(stage) => {
            validate_continuation_stage(stage, stage_ids, export_keys, field_path)
        }
    }
}

fn validate_transaction_stage(
    stage: &TransactionStage,
    candidate_ids: &[&str],
    stage_ids: &[&str],
    field_path: &str,
) -> Result<(), String> {
    let stage_path = format!("{field_path}[{}]", stage.stage_id);
    if !candidate_ids
        .iter()
        .any(|candidate_id| candidate_id == &stage.candidate_ref.as_str())
    {
        return Err(format!(
            "{stage_path}.candidate_ref references unknown candidate `{}`",
            stage.candidate_ref
        ));
    }
    validate_next_stage_ref(stage.next_stage_id.as_deref(), stage_ids, &stage_path)?;

    for export in &stage.exports {
        if export.output_key.trim().is_empty() {
            return Err(format!("{stage_path}.exports output_key must not be empty"));
        }
        validate_value_ref(
            &export.source,
            &format!("{stage_path}.exports[{}].source", export.output_key),
        )?;
    }

    Ok(())
}

fn validate_branch_stage(
    stage: &BranchStage,
    stage_ids: &[&str],
    field_path: &str,
) -> Result<(), String> {
    let stage_path = format!("{field_path}[{}]", stage.stage_id);
    validate_predicate_spec(&stage.predicate, &format!("{stage_path}.predicate"))?;
    validate_branch_target(&stage.if_true, stage_ids, &format!("{stage_path}.if_true"))?;
    validate_branch_target(
        &stage.if_false,
        stage_ids,
        &format!("{stage_path}.if_false"),
    )?;
    Ok(())
}

fn validate_observe_stage(
    stage: &ObserveStage,
    observation_ids: &[&str],
    stage_ids: &[&str],
    field_path: &str,
) -> Result<(), String> {
    let stage_path = format!("{field_path}[{}]", stage.stage_id);
    if !observation_ids
        .iter()
        .any(|observation_id| observation_id == &stage.observation_ref.as_str())
    {
        return Err(format!(
            "{stage_path}.observation_ref references unknown observation `{}`",
            stage.observation_ref
        ));
    }
    validate_next_stage_ref(stage.next_stage_id.as_deref(), stage_ids, &stage_path)?;

    for export in &stage.exports {
        if export.output_key.trim().is_empty() {
            return Err(format!("{stage_path}.exports output_key must not be empty"));
        }
        validate_value_ref(
            &export.source,
            &format!("{stage_path}.exports[{}].source", export.output_key),
        )?;
    }

    Ok(())
}

fn validate_continuation_stage(
    stage: &ais_agent_control::execution_artifact::ContinuationStage,
    stage_ids: &[&str],
    export_keys: &[&str],
    field_path: &str,
) -> Result<(), String> {
    let stage_path = format!("{field_path}[{}]", stage.stage_id);
    if stage.package_entry.trim().is_empty() {
        return Err(format!("{stage_path}.package_entry must not be empty"));
    }

    for required_output in &stage.required_outputs {
        if required_output.trim().is_empty() {
            return Err(format!(
                "{stage_path}.required_outputs entries must not be empty"
            ));
        }
        if !export_keys
            .iter()
            .any(|export_key| export_key == &required_output.as_str())
        {
            return Err(format!(
                "{stage_path}.required_outputs references unknown output `{required_output}`"
            ));
        }
    }

    validate_next_stage_ref(stage.next_stage_id.as_deref(), stage_ids, &stage_path)?;
    Ok(())
}

fn validate_branch_target(
    target: &BranchTarget,
    stage_ids: &[&str],
    field_path: &str,
) -> Result<(), String> {
    match target {
        BranchTarget::GotoStage { stage_id } => {
            if !stage_ids.iter().any(|known| known == &stage_id.as_str()) {
                return Err(format!(
                    "{field_path} references unknown stage_id `{stage_id}`"
                ));
            }
        }
        BranchTarget::Assert {
            failure_code,
            message,
        } => {
            if failure_code.trim().is_empty() {
                return Err(format!("{field_path}.failure_code must not be empty"));
            }
            if message.trim().is_empty() {
                return Err(format!("{field_path}.message must not be empty"));
            }
        }
    }

    Ok(())
}

fn validate_next_stage_ref(
    next_stage_id: Option<&str>,
    stage_ids: &[&str],
    field_path: &str,
) -> Result<(), String> {
    let Some(next_stage_id) = next_stage_id else {
        return Ok(());
    };
    if next_stage_id.trim().is_empty() {
        return Err(format!(
            "{field_path}.next_stage_id must not be blank when present"
        ));
    }
    if !stage_ids.iter().any(|stage_id| stage_id == &next_stage_id) {
        return Err(format!(
            "{field_path}.next_stage_id references unknown stage `{next_stage_id}`"
        ));
    }
    Ok(())
}

fn validate_predicate_spec(predicate: &PredicateSpec, field_path: &str) -> Result<(), String> {
    match predicate {
        PredicateSpec::Comparison { left, right, .. } => {
            validate_value_ref(left, &format!("{field_path}.left"))?;
            validate_value_ref(right, &format!("{field_path}.right"))?;
        }
        PredicateSpec::Cel { expression } => {
            validate_cel_expression(
                expression,
                CelExpressionKind::BoundaryPredicate,
                &format!("{field_path}.expression"),
            )?;
        }
        PredicateSpec::Freshness { evidence_ref, .. } => {
            if evidence_ref.trim().is_empty() {
                return Err(format!("{field_path}.evidence_ref must not be empty"));
            }
        }
        PredicateSpec::ReceiptStatus { receipt_ref, .. } => {
            if receipt_ref.trim().is_empty() {
                return Err(format!("{field_path}.receipt_ref must not be empty"));
            }
        }
    }

    Ok(())
}

fn validate_value_ref(value_ref: &ValueRef, field_path: &str) -> Result<(), String> {
    match value_ref {
        ValueRef::Literal { .. } => Ok(()),
        ValueRef::Ref { reference } => {
            if reference.trim().is_empty() {
                Err(format!("{field_path}.ref must not be empty"))
            } else {
                Ok(())
            }
        }
        ValueRef::Cel { expression } => validate_cel_expression(
            expression,
            CelExpressionKind::Derivation,
            &format!("{field_path}.expression"),
        ),
    }
}

fn validate_observation_spec(spec: &ObservationSpec) -> Result<&str, String> {
    if spec.observation_id.trim().is_empty() {
        return Err("execution_artifact.observations observation_id must not be empty".to_owned());
    }
    if spec.kind.trim().is_empty() {
        return Err(format!(
            "execution_artifact.observations[{}].kind must not be empty",
            spec.observation_id
        ));
    }
    Ok(spec.observation_id.as_str())
}

fn validate_effect_spec<'a>(
    spec: &'a EffectSpec,
    artifact: &ExecutionArtifactLaunchSpec,
    stage_ids: &[&str],
) -> Result<&'a str, String> {
    if spec.effect_id.trim().is_empty() {
        return Err("execution_artifact.expected_effects effect_id must not be empty".to_owned());
    }
    if spec.stage_id.trim().is_empty() {
        return Err(format!(
            "execution_artifact.expected_effects[{}].stage_id must not be empty",
            spec.effect_id
        ));
    }
    if spec.kind.trim().is_empty() {
        return Err(format!(
            "execution_artifact.expected_effects[{}].kind must not be empty",
            spec.effect_id
        ));
    }
    if !stage_ids
        .iter()
        .any(|stage_id| stage_id == &spec.stage_id.as_str())
    {
        return Err(format!(
            "execution_artifact.expected_effects[{}].stage_id references unknown stage `{}`",
            spec.effect_id, spec.stage_id
        ));
    }
    if artifact
        .stage(spec.stage_id.as_str())
        .and_then(ExecutionStage::as_transaction)
        .is_none()
    {
        return Err(format!(
            "execution_artifact.expected_effects[{}].stage_id must reference a transaction stage",
            spec.effect_id
        ));
    }
    if !matches!(
        spec.kind.as_str(),
        "asset_delta" | "state_transition" | "external_job_outcome"
    ) {
        return Err(format!(
            "execution_artifact.expected_effects[{}].kind `{}` is unsupported",
            spec.effect_id, spec.kind
        ));
    }

    let assertions = spec
        .params
        .get("assertions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            format!(
                "execution_artifact.expected_effects[{}].params.assertions must be a non-empty array",
                spec.effect_id
            )
        })?;
    if assertions.is_empty() {
        return Err(format!(
            "execution_artifact.expected_effects[{}].params.assertions must not be empty",
            spec.effect_id
        ));
    }
    for (index, assertion) in assertions.iter().enumerate() {
        let expression = assertion
            .get("expression")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "execution_artifact.expected_effects[{}].params.assertions[{index}].expression must be a non-empty string",
                    spec.effect_id
                )
            })?;
        let description = assertion
            .get("description")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "execution_artifact.expected_effects[{}].params.assertions[{index}].description must be a non-empty string",
                    spec.effect_id
                )
            })?;
        let _ = description;
        validate_cel_expression(
            expression,
            CelExpressionKind::EffectPredicate,
            &format!(
                "execution_artifact.expected_effects[{}].params.assertions[{index}].expression",
                spec.effect_id
            ),
        )?;
    }

    validate_effect_observation_ref(
        spec,
        artifact.preconditions.as_slice(),
        "pre_observation_id",
        "execution_artifact.preconditions",
    )?;
    validate_effect_observation_ref(
        spec,
        artifact.postconditions.as_slice(),
        "post_observation_id",
        "execution_artifact.postconditions",
    )?;

    if let Some(tolerance_hint) = spec.params.get("tolerance_hint") {
        if !tolerance_hint.is_string() {
            return Err(format!(
                "execution_artifact.expected_effects[{}].params.tolerance_hint must be a string when present",
                spec.effect_id
            ));
        }
    }

    Ok(spec.effect_id.as_str())
}

fn validate_effect_observation_ref(
    effect: &EffectSpec,
    observations: &[ObservationSpec],
    key: &str,
    field_path: &str,
) -> Result<(), String> {
    let Some(reference) = effect.params.get(key) else {
        return Ok(());
    };
    let reference = reference
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "execution_artifact.expected_effects[{}].params.{key} must be a non-empty string",
                effect.effect_id
            )
        })?;
    if observations
        .iter()
        .all(|observation| observation.observation_id != reference)
    {
        return Err(format!(
            "execution_artifact.expected_effects[{}].params.{key} references unknown observation `{reference}` in {field_path}",
            effect.effect_id
        ));
    }
    Ok(())
}

fn validate_cel_expression(
    expression: &str,
    kind: CelExpressionKind,
    field_path: &str,
) -> Result<(), String> {
    if expression.trim().is_empty() {
        return Err(format!("{field_path} must not be empty"));
    }

    CelTypeChecker
        .validate(kind, expression)
        .map_err(|error| format!("{field_path} is not valid CEL: {error}"))
}

fn ensure_unique(field_path: &str, values: &[&str]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        if !seen.insert(*value) {
            return Err(format!(
                "{field_path} contains duplicate identifier `{value}`"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use ais_agent_control::execution_artifact::{
        BranchStage, BranchTarget, ComparisonOperator, ContinuationStage, EvmTransactionCandidate,
        ExecutionChainFamily, ObservationSpec, ObserveStage, OutputExportSpec, TransactionStage,
    };

    fn sample_execution_artifact() -> ExecutionArtifactLaunchSpec {
        ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "swap".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["8453".to_owned()],
            entry_stage_id: "stage.allowance".into(),
            actor: None,
            transactions: vec![
                ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                    candidate_id: "tx.approve".into(),
                    to: "0x2222222222222222222222222222222222222222".to_owned(),
                    value: None,
                    calldata: Some("0x095ea7b3".to_owned()),
                }),
                ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                    candidate_id: "tx.swap".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0xdeadbeef".to_owned()),
                }),
            ],
            stages: vec![
                ExecutionStage::Branch(BranchStage {
                    stage_id: "stage.allowance".into(),
                    predicate: PredicateSpec::Comparison {
                        left: ValueRef::Ref {
                            reference: "refs.allowance.current_atomic".to_owned(),
                        },
                        op: ComparisonOperator::Lt,
                        right: ValueRef::Cel {
                            expression: "mul_div(refs.swap.amount_in_atomic, 1, 1)".to_owned(),
                        },
                    },
                    if_true: BranchTarget::GotoStage {
                        stage_id: "stage.approve".into(),
                    },
                    if_false: BranchTarget::GotoStage {
                        stage_id: "stage.swap".into(),
                    },
                }),
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.approve".into(),
                    candidate_ref: "tx.approve".into(),
                    exports: Vec::new(),
                    next_stage_id: Some("stage.swap".into()),
                }),
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.swap".into(),
                    candidate_ref: "tx.swap".into(),
                    exports: vec![OutputExportSpec {
                        output_key: "swap.received_atomic".into(),
                        source: ValueRef::Ref {
                            reference: "refs.post.received_atomic".to_owned(),
                        },
                    }],
                    next_stage_id: Some("stage.continue_aave".into()),
                }),
                ExecutionStage::Continuation(ContinuationStage {
                    stage_id: "stage.continue_aave".into(),
                    required_outputs: vec!["swap.received_atomic".into()],
                    package_entry: "build_aave_supply_from_swap_output".into(),
                    next_stage_id: None,
                }),
            ],
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            risk_class: None,
            risk_tags: Vec::new(),
            decoded_intent: None,
            candidate_envelopes: Vec::new(),
            decode_spec: None,
            validation_plan: None,
            evidence: json!({ "quote": { "quotedAtMs": 1 } }),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn execution_artifact_validation_accepts_minimal_valid_contract() {
        validate_execution_artifact(&sample_execution_artifact()).expect("valid artifact");
    }

    fn sample_observe_only_artifact() -> ExecutionArtifactLaunchSpec {
        ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "quote_exact_in_single".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["eip155:1".to_owned()],
            entry_stage_id: "stage.quote".into(),
            actor: None,
            transactions: Vec::new(),
            stages: vec![ExecutionStage::Observe(ObserveStage {
                stage_id: "stage.quote".into(),
                observation_ref: "query.quote".to_owned(),
                exports: vec![OutputExportSpec {
                    output_key: "quote.amount_out_atomic".into(),
                    source: ValueRef::Ref {
                        reference: "refs.evidence.query.quote.amount_out_atomic".to_owned(),
                    },
                }],
                next_stage_id: None,
            })],
            observations: vec![ObservationSpec {
                observation_id: "query.quote".to_owned(),
                kind: "evm.contract_state_read".to_owned(),
                params: BTreeMap::from([
                    (
                        "to".to_owned(),
                        json!("0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6"),
                    ),
                    (
                        "data".to_owned(),
                        json!("0xf7729d43000000000000000000000000"),
                    ),
                ]),
            }],
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            risk_class: None,
            risk_tags: Vec::new(),
            decoded_intent: None,
            candidate_envelopes: Vec::new(),
            decode_spec: None,
            validation_plan: None,
            evidence: json!({}),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn execution_artifact_validation_accepts_observe_only_contract() {
        validate_execution_artifact(&sample_observe_only_artifact())
            .expect("valid observe-only artifact");
    }

    #[test]
    fn execution_artifact_validation_rejects_missing_chain_scope() {
        let mut artifact = sample_execution_artifact();
        artifact.allowed_chains.clear();

        let error = validate_execution_artifact(&artifact).expect_err("missing chain scope");
        assert!(
            error.contains("allowed_chains must contain exactly one active chain scope"),
            "{error}"
        );
    }

    #[test]
    fn execution_artifact_validation_rejects_multiple_chain_scopes() {
        let mut artifact = sample_execution_artifact();
        artifact.allowed_chains.push("eip155:1".to_owned());

        let error = validate_execution_artifact(&artifact).expect_err("multiple chain scopes");
        assert!(
            error.contains("allowed_chains must contain exactly one active chain scope"),
            "{error}"
        );
    }

    #[test]
    fn execution_artifact_validation_rejects_observe_stage_unknown_observation_ref() {
        let mut artifact = sample_observe_only_artifact();
        let ExecutionStage::Observe(stage) = &mut artifact.stages[0] else {
            panic!("expected observe stage");
        };
        stage.observation_ref = "query.missing".to_owned();

        let error = validate_execution_artifact(&artifact).expect_err("unknown observation ref");
        assert!(error.contains("unknown observation"), "{error}");
    }

    #[test]
    fn execution_artifact_validation_rejects_unknown_transaction_reference() {
        let mut artifact = sample_execution_artifact();
        let ExecutionStage::Transaction(stage) = &mut artifact.stages[1] else {
            panic!("expected transaction stage");
        };
        stage.candidate_ref = "missing".into();

        let error = validate_execution_artifact(&artifact).expect_err("invalid transaction ref");
        assert!(error.contains("unknown candidate"));
    }

    #[test]
    fn execution_artifact_validation_rejects_unknown_stage_reference() {
        let mut artifact = sample_execution_artifact();
        let ExecutionStage::Branch(stage) = &mut artifact.stages[0] else {
            panic!("expected branch stage");
        };
        stage.if_false = BranchTarget::GotoStage {
            stage_id: "missing".into(),
        };

        let error = validate_execution_artifact(&artifact).expect_err("invalid stage ref");
        assert!(error.contains("unknown stage_id"));
    }

    #[test]
    fn execution_artifact_validation_rejects_empty_assert_message() {
        let mut artifact = sample_execution_artifact();
        let ExecutionStage::Branch(stage) = &mut artifact.stages[0] else {
            panic!("expected branch stage");
        };
        stage.if_false = BranchTarget::Assert {
            failure_code: "stale_quote".to_owned(),
            message: " ".to_owned(),
        };

        let error = validate_execution_artifact(&artifact).expect_err("empty assert message");
        assert!(error.contains("message must not be empty"));
    }

    #[test]
    fn execution_artifact_validation_rejects_blank_ref_value_ref() {
        let mut artifact = sample_execution_artifact();
        let ExecutionStage::Transaction(stage) = &mut artifact.stages[2] else {
            panic!("expected transaction stage");
        };
        stage.exports[0].source = ValueRef::Ref {
            reference: " ".to_owned(),
        };

        let error = validate_execution_artifact(&artifact).expect_err("blank ref");
        assert!(error.contains("source.ref must not be empty"));
    }

    #[test]
    fn execution_artifact_validation_rejects_invalid_cel_predicate() {
        let mut artifact = sample_execution_artifact();
        let ExecutionStage::Branch(stage) = &mut artifact.stages[0] else {
            panic!("expected branch stage");
        };
        stage.predicate = PredicateSpec::Cel {
            expression: "refs.allowance <".to_owned(),
        };

        let error = validate_execution_artifact(&artifact).expect_err("invalid CEL");
        assert!(
            error.contains("predicate.expression is not valid CEL"),
            "{error}"
        );
    }

    #[test]
    fn execution_artifact_validation_rejects_unknown_required_output() {
        let mut artifact = sample_execution_artifact();
        let ExecutionStage::Continuation(stage) = &mut artifact.stages[3] else {
            panic!("expected continuation stage");
        };
        stage.required_outputs = vec!["swap.missing_output".into()];

        let error = validate_execution_artifact(&artifact).expect_err("unknown required output");
        assert!(error.contains("unknown output"), "{error}");
    }

    #[test]
    fn execution_artifact_validation_rejects_duplicate_observation_ids() {
        let mut artifact = sample_execution_artifact();
        artifact.preconditions = vec![ObservationSpec {
            observation_id: "state.balance".to_owned(),
            kind: "evm.native_balance".to_owned(),
            params: BTreeMap::from([(
                "address".to_owned(),
                json!("0x1111111111111111111111111111111111111111"),
            )]),
        }];
        artifact.postconditions = vec![ObservationSpec {
            observation_id: "state.balance".to_owned(),
            kind: "evm.native_balance".to_owned(),
            params: BTreeMap::from([(
                "address".to_owned(),
                json!("0x2222222222222222222222222222222222222222"),
            )]),
        }];

        let error = validate_execution_artifact(&artifact).expect_err("duplicate observation id");
        assert!(
            error.contains("duplicate identifier `state.balance`"),
            "{error}"
        );
    }

    #[test]
    fn execution_artifact_validation_rejects_blank_observation_kind() {
        let mut artifact = sample_execution_artifact();
        artifact.postconditions = vec![ObservationSpec {
            observation_id: "state.post.balance".to_owned(),
            kind: " ".to_owned(),
            params: BTreeMap::new(),
        }];

        let error = validate_execution_artifact(&artifact).expect_err("blank observation kind");
        assert!(error.contains("kind must not be empty"), "{error}");
    }

    #[test]
    fn execution_artifact_validation_accepts_expected_effect_for_transaction_stage() {
        let mut artifact = sample_execution_artifact();
        artifact.preconditions = vec![ObservationSpec {
            observation_id: "state.pre.received_balance".to_owned(),
            kind: "evm.erc20_balance_of".to_owned(),
            params: BTreeMap::from([
                (
                    "token".to_owned(),
                    json!("0x1111111111111111111111111111111111111111"),
                ),
                (
                    "owner".to_owned(),
                    json!("0x2222222222222222222222222222222222222222"),
                ),
            ]),
        }];
        artifact.postconditions = vec![ObservationSpec {
            observation_id: "state.post.received_balance".to_owned(),
            kind: "evm.erc20_balance_of".to_owned(),
            params: BTreeMap::from([
                (
                    "token".to_owned(),
                    json!("0x1111111111111111111111111111111111111111"),
                ),
                (
                    "owner".to_owned(),
                    json!("0x2222222222222222222222222222222222222222"),
                ),
            ]),
        }];
        artifact.expected_effects = vec![EffectSpec {
            effect_id: "effect.swap".to_owned(),
            stage_id: "stage.swap".into(),
            kind: "asset_delta".to_owned(),
            params: BTreeMap::from([
                (
                    "assertions".to_owned(),
                    json!([{
                        "expression": "receipt.status == true && post.decoded_u256 != pre.decoded_u256",
                        "description": "swap must change recipient balance"
                    }]),
                ),
                (
                    "pre_observation_id".to_owned(),
                    json!("state.pre.received_balance"),
                ),
                (
                    "post_observation_id".to_owned(),
                    json!("state.post.received_balance"),
                ),
            ]),
        }];

        validate_execution_artifact(&artifact).expect("valid artifact");
    }

    #[test]
    fn execution_artifact_validation_rejects_expected_effect_for_non_transaction_stage() {
        let mut artifact = sample_execution_artifact();
        artifact.expected_effects = vec![EffectSpec {
            effect_id: "effect.allowance".to_owned(),
            stage_id: "stage.allowance".into(),
            kind: "asset_delta".to_owned(),
            params: BTreeMap::from([(
                "assertions".to_owned(),
                json!([{
                    "expression": "receipt.status == true",
                    "description": "branch is not a transaction stage"
                }]),
            )]),
        }];

        let error = validate_execution_artifact(&artifact).expect_err("invalid effect stage");
        assert!(
            error.contains("must reference a transaction stage"),
            "{error}"
        );
    }

    #[test]
    fn execution_artifact_validation_rejects_invalid_expected_effect_assertion() {
        let mut artifact = sample_execution_artifact();
        artifact.expected_effects = vec![EffectSpec {
            effect_id: "effect.swap".to_owned(),
            stage_id: "stage.swap".into(),
            kind: "asset_delta".to_owned(),
            params: BTreeMap::from([(
                "assertions".to_owned(),
                json!([{
                    "expression": "receipt.status ==",
                    "description": "bad CEL"
                }]),
            )]),
        }];

        let error = validate_execution_artifact(&artifact).expect_err("invalid effect CEL");
        assert!(error.contains("is not valid CEL"), "{error}");
    }

    #[test]
    fn execution_artifact_validation_accepts_complete_semantic_contract() {
        let mut artifact = sample_execution_artifact();
        artifact.risk_class = Some("bounded_swap".to_owned());
        artifact.risk_tags = vec!["external_quote".to_owned()];
        artifact.decoded_intent = Some(json!({
            "kind": "swap_exact_in",
            "token_in": "0x2222222222222222222222222222222222222222",
            "token_out": "0x1111111111111111111111111111111111111111",
        }));
        artifact.candidate_envelopes = vec![json!({
            "candidate_ref": "tx.swap",
            "kind": "evm_transaction",
        })];
        artifact.decode_spec = Some(json!({
            "kind": "abi",
            "entrypoint": "swapExactTokensForTokens",
        }));
        artifact.validation_plan = Some(json!({
            "checks": ["target", "selector", "intent"],
        }));

        validate_execution_artifact(&artifact).expect("valid semantic artifact");
    }

    #[test]
    fn execution_artifact_validation_rejects_semantic_contract_missing_risk_class() {
        let mut artifact = sample_execution_artifact();
        artifact.decoded_intent = Some(json!({ "kind": "swap_exact_in" }));
        artifact.candidate_envelopes = vec![json!({ "candidate_ref": "tx.swap" })];
        artifact.validation_plan = Some(json!({ "checks": [] }));

        let error =
            validate_execution_artifact(&artifact).expect_err("semantic artifact missing risk");
        assert!(error.contains("risk_class is required"), "{error}");
    }

    #[test]
    fn execution_artifact_validation_rejects_semantic_contract_without_candidate_envelopes() {
        let mut artifact = sample_execution_artifact();
        artifact.risk_class = Some("bounded_swap".to_owned());
        artifact.decoded_intent = Some(json!({ "kind": "swap_exact_in" }));
        artifact.validation_plan = Some(json!({ "checks": [] }));

        let error = validate_execution_artifact(&artifact)
            .expect_err("semantic artifact without candidate envelopes");
        assert!(error.contains("candidate_envelopes must include at least one envelope"), "{error}");
    }

    #[test]
    fn execution_artifact_validation_rejects_semantic_contract_with_unknown_candidate_ref() {
        let mut artifact = sample_execution_artifact();
        artifact.risk_class = Some("bounded_swap".to_owned());
        artifact.decoded_intent = Some(json!({ "kind": "swap_exact_in" }));
        artifact.candidate_envelopes = vec![json!({ "candidate_ref": "tx.missing" })];
        artifact.validation_plan = Some(json!({ "checks": [] }));

        let error =
            validate_execution_artifact(&artifact).expect_err("unknown semantic candidate ref");
        assert!(error.contains("references unknown candidate `tx.missing`"), "{error}");
    }
}
