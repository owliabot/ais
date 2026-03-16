use std::{collections::BTreeMap, str::FromStr};

use ais_agent_control::execution_artifact::{
    EffectSpec, ExecutionArtifactLaunchSpec, ExecutionStage, ObservationSpec, ObserveStage,
    SolanaInstructionCandidate, SolanaTransactionCandidate, TransactionStage,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            observe::{
                ObserveAction, ObserveLiveBinding, ObserveSourceKind, SolanaObserveLiveBinding,
            },
            simulate::{
                SimulateAction, SimulateKind, SimulateLiveBinding, SolanaSimulateLiveBinding,
            },
            verify::{SolanaVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::solana::{
        SolanaConnectionSpec, SolanaObserveBinding, SolanaObserveRequest, SolanaSimulateBinding,
        SolanaTransactionRequest, SolanaVerifyBinding,
    },
    effect::{EffectAssertion, EffectContract, EffectContractKind},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::Value;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Signature,
};

#[derive(Debug, Clone)]
pub struct PlannedSolanaExecutionArtifact {
    pub planned_stage_graphs:
        BTreeMap<ais_agent_control::execution_artifact::ExecutionStageId, ActionGraph>,
    pub effect_contracts: BTreeMap<String, EffectContract>,
}

#[derive(Debug, Clone)]
struct PlannedExecutionEffect {
    effect_contract: EffectContract,
    pre_observation_ref: Option<String>,
    post_observation_ref: Option<String>,
    post_request: Option<SolanaObserveRequest>,
}

pub fn plan_execution_artifact(
    spec: &ExecutionArtifactLaunchSpec,
    solana_rpc_url: Option<&str>,
    chain_scope: &str,
) -> Result<PlannedSolanaExecutionArtifact, String> {
    let planned_effects = plan_solana_effects(spec)?;
    let planned_stage_graphs =
        plan_stage_graphs(spec, chain_scope, solana_rpc_url, &planned_effects)?;
    let effect_contracts = planned_effects
        .values()
        .map(|effect| {
            (
                effect.effect_contract.effect_id.clone(),
                effect.effect_contract.clone(),
            )
        })
        .collect();
    Ok(PlannedSolanaExecutionArtifact {
        planned_stage_graphs,
        effect_contracts,
    })
}

fn plan_stage_graphs(
    spec: &ExecutionArtifactLaunchSpec,
    chain_scope: &str,
    solana_rpc_url: Option<&str>,
    planned_effects: &BTreeMap<
        ais_agent_control::execution_artifact::ExecutionStageId,
        PlannedExecutionEffect,
    >,
) -> Result<BTreeMap<ais_agent_control::execution_artifact::ExecutionStageId, ActionGraph>, String>
{
    let mut planned = BTreeMap::new();
    for stage in &spec.stages {
        match stage {
            ExecutionStage::Transaction(stage) => {
                let candidate = spec
                    .transaction_candidate(stage.candidate_ref.as_str())
                    .ok_or_else(|| {
                        format!(
                            "execution_artifact stage `{}` references unknown candidate `{}`",
                            stage.stage_id, stage.candidate_ref
                        )
                    })?;
                let graph = plan_single_solana_transaction_stage(
                    spec.protocol_package_id.as_str(),
                    spec.action_key.as_str(),
                    stage,
                    candidate,
                    &spec.preconditions,
                    &spec.postconditions,
                    spec.actor
                        .as_ref()
                        .and_then(|actor| actor.sender_address_hint.as_deref()),
                    solana_rpc_url,
                    chain_scope,
                    planned_effects.get(&stage.stage_id),
                )?;
                planned.insert(stage.stage_id.clone(), graph);
            }
            ExecutionStage::Observe(stage) => {
                let graph = plan_single_solana_observe_stage(
                    spec.protocol_package_id.as_str(),
                    spec.action_key.as_str(),
                    stage,
                    spec.observations.as_slice(),
                    solana_rpc_url,
                    chain_scope,
                )?;
                planned.insert(stage.stage_id.clone(), graph);
            }
            ExecutionStage::Branch(_) | ExecutionStage::Continuation(_) => {}
        }
    }
    Ok(planned)
}

fn plan_single_solana_observe_stage(
    protocol_package_id: &str,
    action_key: &str,
    stage: &ObserveStage,
    observations: &[ObservationSpec],
    solana_rpc_url: Option<&str>,
    chain_scope: &str,
) -> Result<ActionGraph, String> {
    let observation = observations
        .iter()
        .find(|observation| observation.observation_id == stage.observation_ref)
        .ok_or_else(|| {
            format!(
                "execution_artifact stage `{}` references unknown observation `{}`",
                stage.stage_id, stage.observation_ref
            )
        })?;
    let live_connection = solana_rpc_url.map(|rpc_url| SolanaConnectionSpec {
        rpc_url: rpc_url.to_owned(),
        ws_url: None,
    });
    let graph_id = format!("artifact.{protocol_package_id}.{action_key}");
    let node_prefix = format!("artifact.{}", stage.stage_id);
    let observe_node = observation_node(
        format!("{node_prefix}.observe"),
        observation,
        chain_scope,
        live_connection,
    )?;

    Ok(ActionGraph {
        graph_id: Some(graph_id),
        roots: vec![observe_node.node_id.clone()],
        terminals: vec![observe_node.node_id.clone()],
        nodes: BTreeMap::from([(observe_node.node_id.clone(), observe_node)]),
    })
}

fn plan_single_solana_transaction_stage(
    protocol_package_id: &str,
    action_key: &str,
    stage: &TransactionStage,
    candidate: &ais_agent_control::execution_artifact::ExecutionTransactionCandidate,
    preconditions: &[ObservationSpec],
    postconditions: &[ObservationSpec],
    sender_address_hint: Option<&str>,
    solana_rpc_url: Option<&str>,
    chain_scope: &str,
    expected_effect: Option<&PlannedExecutionEffect>,
) -> Result<ActionGraph, String> {
    let sender = sender_address_hint
        .map(|value| parse_pubkey(value, "execution_artifact.actor.sender_address_hint"))
        .transpose()?;
    let live_connection = solana_rpc_url.map(|rpc_url| SolanaConnectionSpec {
        rpc_url: rpc_url.to_owned(),
        ws_url: None,
    });
    let graph_id = format!("artifact.{protocol_package_id}.{action_key}");
    let node_prefix = format!("artifact.{}", stage.stage_id);

    let Some(candidate) = candidate.as_solana_transaction() else {
        return Err(
            "execution_artifact current solana planner requires solana_transaction candidates"
                .to_owned(),
        );
    };

    let request = solana_transaction_request(sender, candidate)?;
    let actuator_hint = format!(
        "execute solana transaction {} on {chain_scope}",
        candidate.candidate_id
    );

    single_transaction_graph(
        graph_id,
        node_prefix,
        chain_scope,
        live_connection,
        request,
        actuator_hint,
        preconditions,
        postconditions,
        expected_effect,
    )
}

fn single_transaction_graph(
    graph_id: String,
    node_prefix: String,
    chain_scope: &str,
    live_connection: Option<SolanaConnectionSpec>,
    request: SolanaTransactionRequest,
    actuator_hint: String,
    preconditions: &[ObservationSpec],
    postconditions: &[ObservationSpec],
    expected_effect: Option<&PlannedExecutionEffect>,
) -> Result<ActionGraph, String> {
    let pre_observations = preconditions
        .iter()
        .map(|spec| {
            observation_node(
                format!("{node_prefix}.pre_observe.{}", spec.observation_id),
                spec,
                chain_scope,
                live_connection.clone(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let post_observations = postconditions
        .iter()
        .map(|spec| {
            let mut node = observation_node(
                format!("{node_prefix}.post_observe.{}", spec.observation_id),
                spec,
                chain_scope,
                live_connection.clone(),
            )?;
            node.depends_on = vec![format!("{node_prefix}.verify")];
            Ok::<ActionNode, String>(node)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let simulate_id = format!("{node_prefix}.simulate");
    let actuate_id = format!("{node_prefix}.actuate");
    let verify_id = format!("{node_prefix}.verify");
    let effect_ref = expected_effect.map(|effect| effect.effect_contract.effect_id.clone());

    let mut nodes = BTreeMap::new();
    for node in &pre_observations {
        nodes.insert(node.node_id.clone(), node.clone());
    }
    nodes.insert(
        simulate_id.clone(),
        ActionNode {
            node_id: simulate_id.clone(),
            kind: ActionNodeKind::Simulate,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: pre_observations
                .iter()
                .map(|node| node.node_id.clone())
                .collect(),
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Simulate(SimulateAction {
                simulate_kind: SimulateKind::Call,
                simulator_hint: format!("simulate {node_prefix}"),
                live: live_connection.clone().map(|connection| {
                    SimulateLiveBinding::Solana(SolanaSimulateLiveBinding {
                        connection: Some(connection),
                        binding: SolanaSimulateBinding::SimulateTransaction,
                        request: request.clone(),
                    })
                }),
            }),
            implementation_hint: Some("execution_artifact".to_owned()),
            expected_effect_ref: None,
        },
    );
    nodes.insert(
        actuate_id.clone(),
        ActionNode {
            node_id: actuate_id.clone(),
            kind: ActionNodeKind::Actuate,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec![simulate_id.clone()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Actuate(ActuateAction {
                mode: ActuateMode::DriverCall,
                actuator_hint,
                chain: Some(chain_scope.to_owned()),
                envelope_ref: None,
                requires_effect_contract: expected_effect.is_some(),
                live: None,
            }),
            implementation_hint: Some("execution_artifact".to_owned()),
            expected_effect_ref: effect_ref.clone(),
        },
    );
    for node in &post_observations {
        nodes.insert(node.node_id.clone(), node.clone());
    }
    nodes.insert(
        verify_id.clone(),
        ActionNode {
            node_id: verify_id.clone(),
            kind: ActionNodeKind::Verify,
            origin: ActionOrigin::DriverFragment,
            status: ActionNodeStatus::Pending,
            depends_on: vec![actuate_id.clone()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Verify(VerifyAction {
                verify_kind: if expected_effect.is_some() {
                    VerifyKind::EffectContract
                } else {
                    VerifyKind::ReceiptObserved
                },
                verifier_hint: if expected_effect.is_some() {
                    format!("verify expected effect for {node_prefix}")
                } else {
                    format!("verify {node_prefix}")
                },
                pre_observation_ref: expected_effect
                    .and_then(|effect| effect.pre_observation_ref.clone()),
                post_observation_ref: expected_effect
                    .and_then(|effect| effect.post_observation_ref.clone()),
                live: live_connection.map(|connection| {
                    VerifyLiveBinding::Solana(SolanaVerifyLiveBinding {
                        connection: Some(connection),
                        binding: if expected_effect.is_some() {
                            SolanaVerifyBinding::EffectContractFromSignatureStatus
                        } else {
                            SolanaVerifyBinding::SignatureStatus
                        },
                        post_request: expected_effect
                            .and_then(|effect| effect.post_request.clone()),
                    })
                }),
            }),
            implementation_hint: Some("execution_artifact".to_owned()),
            expected_effect_ref: effect_ref,
        },
    );

    Ok(ActionGraph {
        graph_id: Some(graph_id),
        roots: if pre_observations.is_empty() {
            vec![simulate_id]
        } else {
            pre_observations
                .iter()
                .map(|node| node.node_id.clone())
                .collect()
        },
        terminals: if post_observations.is_empty() {
            vec![verify_id]
        } else {
            post_observations
                .iter()
                .map(|node| node.node_id.clone())
                .collect()
        },
        nodes,
    })
}

fn plan_solana_effects(
    spec: &ExecutionArtifactLaunchSpec,
) -> Result<
    BTreeMap<ais_agent_control::execution_artifact::ExecutionStageId, PlannedExecutionEffect>,
    String,
> {
    let preconditions = spec
        .preconditions
        .iter()
        .map(|observation| (observation.observation_id.as_str(), observation))
        .collect::<BTreeMap<_, _>>();
    let postconditions = spec
        .postconditions
        .iter()
        .map(|observation| (observation.observation_id.as_str(), observation))
        .collect::<BTreeMap<_, _>>();
    let mut planned = BTreeMap::new();

    for effect in &spec.expected_effects {
        let pre_observation_ref = optional_effect_string_param(effect, "pre_observation_id")?;
        let post_observation_ref = optional_effect_string_param(effect, "post_observation_id")?;
        let post_request = post_observation_ref
            .as_ref()
            .map(|reference| {
                postconditions
                    .get(reference.as_str())
                    .ok_or_else(|| {
                        format!(
                            "execution_artifact expected effect `{}` references unknown post observation `{reference}`",
                            effect.effect_id
                        )
                    })
                    .and_then(|spec| parse_solana_observation_spec(spec).map(|(_, request)| request))
            })
            .transpose()?;

        if let Some(reference) = pre_observation_ref.as_ref() {
            if !preconditions.contains_key(reference.as_str()) {
                return Err(format!(
                    "execution_artifact expected effect `{}` references unknown pre observation `{reference}`",
                    effect.effect_id
                ));
            }
        }

        let stage_id = effect.stage_id.clone();
        if planned.contains_key(&stage_id) {
            return Err(format!(
                "execution_artifact currently supports at most one expected effect per stage; `{stage_id}` has multiple effect specs"
            ));
        }

        planned.insert(
            stage_id,
            PlannedExecutionEffect {
                effect_contract: effect_spec_to_contract(effect)?,
                pre_observation_ref,
                post_observation_ref,
                post_request,
            },
        );
    }

    Ok(planned)
}

fn effect_spec_to_contract(effect: &EffectSpec) -> Result<EffectContract, String> {
    Ok(EffectContract {
        effect_id: effect.effect_id.clone(),
        kind: effect_contract_kind(effect)?,
        assertions: effect_assertions(effect)?,
        tolerance_hint: optional_effect_string_param(effect, "tolerance_hint")?,
    })
}

fn effect_contract_kind(effect: &EffectSpec) -> Result<EffectContractKind, String> {
    match effect.kind.as_str() {
        "asset_delta" => Ok(EffectContractKind::AssetDelta),
        "state_transition" => Ok(EffectContractKind::StateTransition),
        "external_job_outcome" => Ok(EffectContractKind::ExternalJobOutcome),
        other => Err(format!(
            "execution_artifact expected effect `{}` uses unsupported kind `{other}`",
            effect.effect_id
        )),
    }
}

fn effect_assertions(effect: &EffectSpec) -> Result<Vec<EffectAssertion>, String> {
    let Some(assertions) = effect.params.get("assertions").and_then(Value::as_array) else {
        return Err(format!(
            "execution_artifact expected effect `{}` requires params.assertions",
            effect.effect_id
        ));
    };

    assertions
        .iter()
        .enumerate()
        .map(|(index, assertion)| {
            let expression = assertion
                .get("expression")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "execution_artifact expected effect `{}` assertions[{index}] requires non-empty `expression`",
                        effect.effect_id
                    )
                })?;
            let description = assertion
                .get("description")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    format!(
                        "execution_artifact expected effect `{}` assertions[{index}] requires non-empty `description`",
                        effect.effect_id
                    )
                })?;
            Ok(EffectAssertion {
                expression: expression.to_owned(),
                description: description.to_owned(),
            })
        })
        .collect()
}

fn optional_effect_string_param(effect: &EffectSpec, key: &str) -> Result<Option<String>, String> {
    let Some(value) = effect.params.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "execution_artifact expected effect `{}` requires non-empty string param `{key}`",
                effect.effect_id
            )
        })?;
    Ok(Some(value.to_owned()))
}

fn observation_node(
    node_id: String,
    spec: &ObservationSpec,
    chain_scope: &str,
    live_connection: Option<SolanaConnectionSpec>,
) -> Result<ActionNode, String> {
    let (binding, request) = parse_solana_observation_spec(spec)?;
    Ok(ActionNode {
        node_id,
        kind: ActionNodeKind::Observe,
        origin: ActionOrigin::DriverFragment,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Observe(ObserveAction {
            source_kind: ObserveSourceKind::ChainRead,
            source_hint: format!("observe {} on {chain_scope}", spec.observation_id),
            output_key: Some(spec.observation_id.clone()),
            live: Some(ObserveLiveBinding::Solana(SolanaObserveLiveBinding {
                connection: live_connection,
                binding,
                request,
            })),
        }),
        implementation_hint: Some("execution_artifact".to_owned()),
        expected_effect_ref: None,
    })
}

fn parse_solana_observation_spec(
    spec: &ObservationSpec,
) -> Result<(SolanaObserveBinding, SolanaObserveRequest), String> {
    match spec.kind.as_str() {
        "solana.slot" => Ok((SolanaObserveBinding::Slot, SolanaObserveRequest::Slot)),
        "solana.account_lamports" => {
            let address = pubkey_param(spec, "address")?;
            Ok((
                SolanaObserveBinding::AccountLamports,
                SolanaObserveRequest::AccountLamports { address },
            ))
        }
        "solana.spl_token_balance" => {
            let token_account = pubkey_param(spec, "token_account")?;
            Ok((
                SolanaObserveBinding::SplTokenBalance,
                SolanaObserveRequest::SplTokenBalance { token_account },
            ))
        }
        "solana.account_data" => {
            let address = pubkey_param(spec, "address")?;
            Ok((
                SolanaObserveBinding::AccountData,
                SolanaObserveRequest::AccountData { address },
            ))
        }
        "solana.signature_status" => {
            let signature = signature_param(spec, "signature")?;
            Ok((
                SolanaObserveBinding::SignatureStatus,
                SolanaObserveRequest::SignatureStatus { signature },
            ))
        }
        other => Err(format!(
            "execution_artifact observation `{}` uses unsupported kind `{other}`",
            spec.observation_id
        )),
    }
}

fn solana_transaction_request(
    payer: Option<Pubkey>,
    candidate: &SolanaTransactionCandidate,
) -> Result<SolanaTransactionRequest, String> {
    let instructions = candidate
        .instructions
        .iter()
        .enumerate()
        .map(|(index, instruction)| {
            instruction_candidate_to_instruction(candidate, index, instruction)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SolanaTransactionRequest::Legacy {
        recent_blockhash: None,
        payer,
        instructions,
    })
}

fn instruction_candidate_to_instruction(
    candidate: &SolanaTransactionCandidate,
    index: usize,
    instruction: &SolanaInstructionCandidate,
) -> Result<Instruction, String> {
    let program_id = parse_pubkey(
        instruction.program_id.as_str(),
        &format!(
            "execution_artifact.transactions.{}.instructions[{index}].program_id",
            candidate.candidate_id
        ),
    )?;
    let accounts = instruction
        .accounts
        .iter()
        .enumerate()
        .map(|(account_index, account)| {
            let pubkey = parse_pubkey(
                account.address.as_str(),
                &format!(
                    "execution_artifact.transactions.{}.instructions[{index}].accounts[{account_index}].address",
                    candidate.candidate_id
                ),
            )?;
            Ok::<AccountMeta, String>(if account.is_writable {
                AccountMeta::new(pubkey, account.is_signer)
            } else {
                AccountMeta::new_readonly(pubkey, account.is_signer)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let data = instruction
        .data_base64
        .as_deref()
        .map(|value| {
            BASE64.decode(value).map_err(|error| {
                format!(
                    "invalid execution_artifact transaction `{}` instruction[{index}] data_base64: {error}",
                    candidate.candidate_id
                )
            })
        })
        .transpose()?
        .unwrap_or_default();

    Ok(Instruction {
        program_id,
        accounts,
        data,
    })
}

fn string_param<'a>(spec: &'a ObservationSpec, key: &str) -> Result<&'a str, String> {
    spec.params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "execution_artifact observation `{}` requires string param `{key}`",
                spec.observation_id
            )
        })
}

fn pubkey_param(spec: &ObservationSpec, key: &str) -> Result<Pubkey, String> {
    parse_pubkey(
        string_param(spec, key)?,
        &format!(
            "execution_artifact.observations.{}.{}",
            spec.observation_id, key
        ),
    )
}

fn signature_param(spec: &ObservationSpec, key: &str) -> Result<Signature, String> {
    Signature::from_str(string_param(spec, key)?).map_err(|error| {
        format!(
            "invalid execution_artifact observation `{}` signature param `{key}`: {error}",
            spec.observation_id
        )
    })
}

fn parse_pubkey(value: &str, field: &str) -> Result<Pubkey, String> {
    Pubkey::from_str(value).map_err(|error| format!("invalid {field} pubkey `{value}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn planner_builds_solana_observe_stage_graph() {
        let graph = plan_single_solana_observe_stage(
            "owliabot.solana",
            "read_slot",
            &ObserveStage {
                stage_id: "stage.observe".into(),
                observation_ref: "obs.slot".to_owned(),
                exports: Vec::new(),
                next_stage_id: None,
            },
            &[ObservationSpec {
                observation_id: "obs.slot".to_owned(),
                kind: "solana.slot".to_owned(),
                params: BTreeMap::new(),
            }],
            Some("http://127.0.0.1:8899"),
            "solana:mainnet",
        )
        .expect("planned graph");

        assert_eq!(
            graph.roots,
            vec!["artifact.stage.observe.observe".to_owned()]
        );
        let observe = graph
            .nodes
            .get("artifact.stage.observe.observe")
            .expect("observe node");
        let ActionPayload::Observe(payload) = &observe.payload else {
            panic!("expected observe payload");
        };
        match payload.live.as_ref() {
            Some(ObserveLiveBinding::Solana(live)) => {
                assert_eq!(live.binding, SolanaObserveBinding::Slot);
            }
            other => panic!("unexpected live binding: {other:?}"),
        }
    }

    #[test]
    fn planner_builds_solana_transaction_verify_effect_contract() {
        let graph = plan_single_solana_transaction_stage(
            "owliabot.solana",
            "transfer",
            &TransactionStage {
                stage_id: "stage.transfer".into(),
                candidate_ref: "tx.transfer".into(),
                exports: Vec::new(),
                next_stage_id: None,
            },
            &ais_agent_control::execution_artifact::ExecutionTransactionCandidate::SolanaTransaction(
                SolanaTransactionCandidate {
                    candidate_id: "tx.transfer".into(),
                    instructions: vec![SolanaInstructionCandidate {
                        program_id: "11111111111111111111111111111111".to_owned(),
                        accounts: vec![ais_agent_control::execution_artifact::SolanaInstructionAccount {
                            address: "11111111111111111111111111111111".to_owned(),
                            is_signer: true,
                            is_writable: true,
                        }],
                        data_base64: Some("AQID".to_owned()),
                    }],
                },
            ),
            &[],
            &[ObservationSpec {
                observation_id: "state.post.signature".to_owned(),
                kind: "solana.signature_status".to_owned(),
                params: BTreeMap::from([(
                    "signature".to_owned(),
                    json!(Signature::new_unique().to_string()),
                )]),
            }],
            Some("11111111111111111111111111111111"),
            Some("http://127.0.0.1:8899"),
            "solana:mainnet",
            Some(&PlannedExecutionEffect {
                effect_contract: EffectContract {
                    effect_id: "effect.transfer".to_owned(),
                    kind: EffectContractKind::AssetDelta,
                    assertions: vec![EffectAssertion {
                        expression: "receipt.status == true".to_owned(),
                        description: "signature must confirm".to_owned(),
                    }],
                    tolerance_hint: None,
                },
                pre_observation_ref: None,
                post_observation_ref: Some("state.post.signature".to_owned()),
                post_request: Some(SolanaObserveRequest::SignatureStatus {
                    signature: Signature::new_unique(),
                }),
            }),
        )
        .expect("planned graph");

        let verify = graph
            .nodes
            .get("artifact.stage.transfer.verify")
            .expect("verify node");
        let ActionPayload::Verify(payload) = &verify.payload else {
            panic!("expected verify payload");
        };
        assert_eq!(payload.verify_kind, VerifyKind::EffectContract);
        match payload.live.as_ref() {
            Some(VerifyLiveBinding::Solana(live)) => {
                assert_eq!(
                    live.binding,
                    SolanaVerifyBinding::EffectContractFromSignatureStatus
                );
                assert!(live.post_request.is_some());
            }
            other => panic!("unexpected live binding: {other:?}"),
        }
    }
}
