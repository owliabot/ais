use std::{collections::BTreeMap, str::FromStr};

use ais_agent_control::execution_artifact::{
    EffectSpec, EvmTransactionCandidate, ExecutionArtifactLaunchSpec, ExecutionStage,
    ObservationSpec, ObserveStage, TransactionStage,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateLiveBinding, ActuateMode, EvmActuateLiveBinding},
            observe::{
                EvmObserveLiveBinding, ObserveAction, ObserveLiveBinding, ObserveSourceKind,
            },
            simulate::{EvmSimulateLiveBinding, SimulateAction, SimulateKind, SimulateLiveBinding},
            verify::{EvmVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{
        EvmActuateBinding, EvmCallRequest, EvmObserveBinding, EvmObserveRequest,
        EvmSimulateBinding, EvmVerifyBinding,
    },
    effect::{EffectAssertion, EffectContract, EffectContractKind},
};
use alloy::primitives::{Address, Bytes, U256};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct PlannedEvmExecutionArtifact {
    pub planned_stage_graphs:
        BTreeMap<ais_agent_control::execution_artifact::ExecutionStageId, ActionGraph>,
    pub effect_contracts: BTreeMap<String, EffectContract>,
}

#[derive(Debug, Clone)]
struct PlannedExecutionEffect {
    effect_contract: EffectContract,
    pre_observation_ref: Option<String>,
    post_observation_ref: Option<String>,
    post_request: Option<EvmObserveRequest>,
}

pub fn plan_execution_artifact(
    spec: &ExecutionArtifactLaunchSpec,
    evm_rpc_url: Option<&str>,
    chain_scope: &str,
) -> Result<PlannedEvmExecutionArtifact, String> {
    let planned_effects = plan_evm_effects(spec)?;
    let planned_stage_graphs = plan_stage_graphs(spec, chain_scope, evm_rpc_url, &planned_effects)?;
    let effect_contracts = planned_effects
        .values()
        .map(|effect| {
            (
                effect.effect_contract.effect_id.clone(),
                effect.effect_contract.clone(),
            )
        })
        .collect();
    Ok(PlannedEvmExecutionArtifact {
        planned_stage_graphs,
        effect_contracts,
    })
}

fn plan_stage_graphs(
    spec: &ExecutionArtifactLaunchSpec,
    chain_scope: &str,
    evm_rpc_url: Option<&str>,
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
                let graph = plan_single_evm_transaction_stage(
                    spec.protocol_package_id.as_str(),
                    spec.action_key.as_str(),
                    stage,
                    candidate,
                    &spec.preconditions,
                    &spec.postconditions,
                    spec.actor
                        .as_ref()
                        .and_then(|actor| actor.sender_address_hint.as_deref()),
                    evm_rpc_url,
                    chain_scope,
                    planned_effects.get(&stage.stage_id),
                )?;
                planned.insert(stage.stage_id.clone(), graph);
            }
            ExecutionStage::Observe(stage) => {
                let graph = plan_single_evm_observe_stage(
                    spec.protocol_package_id.as_str(),
                    spec.action_key.as_str(),
                    stage,
                    spec.observations.as_slice(),
                    evm_rpc_url,
                    chain_scope,
                )?;
                planned.insert(stage.stage_id.clone(), graph);
            }
            ExecutionStage::Branch(_) | ExecutionStage::Continuation(_) => {}
        }
    }
    Ok(planned)
}

fn plan_single_evm_observe_stage(
    protocol_package_id: &str,
    action_key: &str,
    stage: &ObserveStage,
    observations: &[ObservationSpec],
    evm_rpc_url: Option<&str>,
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
    let live_connection =
        evm_rpc_url.map(|rpc_url| ais_agent_core::binding::evm::EvmConnectionSpec {
            rpc_url: rpc_url.to_owned(),
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

fn plan_single_evm_transaction_stage(
    protocol_package_id: &str,
    action_key: &str,
    stage: &TransactionStage,
    candidate: &ais_agent_control::execution_artifact::ExecutionTransactionCandidate,
    preconditions: &[ObservationSpec],
    postconditions: &[ObservationSpec],
    sender_address_hint: Option<&str>,
    evm_rpc_url: Option<&str>,
    chain_scope: &str,
    expected_effect: Option<&PlannedExecutionEffect>,
) -> Result<ActionGraph, String> {
    let sender = sender_address_hint
        .map(|value| parse_address(value, "execution_artifact.actor.sender_address_hint"))
        .transpose()?;
    let live_connection =
        evm_rpc_url.map(|rpc_url| ais_agent_core::binding::evm::EvmConnectionSpec {
            rpc_url: rpc_url.to_owned(),
        });
    let graph_id = format!("artifact.{protocol_package_id}.{action_key}");
    let node_prefix = format!("artifact.{}", stage.stage_id);

    let Some(candidate) = candidate.as_evm_transaction() else {
        return Err(
            "execution_artifact current evm planner requires evm_transaction candidates".to_owned(),
        );
    };

    let call = evm_call_request(sender, candidate)?;
    let actuator_hint = if candidate.calldata.is_some() {
        format!(
            "execute contract call {} on {chain_scope}",
            candidate.candidate_id
        )
    } else {
        format!(
            "execute native transfer {} on {chain_scope}",
            candidate.candidate_id
        )
    };

    single_call_graph(
        graph_id,
        node_prefix,
        chain_scope,
        live_connection,
        call,
        actuator_hint,
        preconditions,
        postconditions,
        expected_effect,
    )
}

fn single_call_graph(
    graph_id: String,
    node_prefix: String,
    chain_scope: &str,
    live_connection: Option<ais_agent_core::binding::evm::EvmConnectionSpec>,
    request: EvmCallRequest,
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
                    SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                        connection: Some(connection),
                        binding: EvmSimulateBinding::EthCall,
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
                live: live_connection.clone().map(|connection| {
                    ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                        connection: Some(connection),
                        binding: EvmActuateBinding::BroadcastRawTransaction,
                    })
                }),
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
                    VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                        connection: Some(connection),
                        binding: if let Some(effect) = expected_effect {
                            if effect.post_request.is_some() {
                                EvmVerifyBinding::EffectContractFromReceiptAndPostState
                            } else {
                                EvmVerifyBinding::EffectContractFromReceipt
                            }
                        } else {
                            EvmVerifyBinding::ReceiptStatus
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

fn plan_evm_effects(
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
                    .and_then(|spec| parse_evm_observation_spec(spec).map(|(_, request)| request))
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
    live_connection: Option<ais_agent_core::binding::evm::EvmConnectionSpec>,
) -> Result<ActionNode, String> {
    let (binding, request) = parse_evm_observation_spec(spec)?;
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
            live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                connection: live_connection,
                binding,
                request,
            })),
        }),
        implementation_hint: Some("execution_artifact".to_owned()),
        expected_effect_ref: None,
    })
}

fn parse_evm_observation_spec(
    spec: &ObservationSpec,
) -> Result<(EvmObserveBinding, EvmObserveRequest), String> {
    match spec.kind.as_str() {
        "evm.block_number" => Ok((
            EvmObserveBinding::BlockNumber,
            EvmObserveRequest::BlockNumber,
        )),
        "evm.native_balance" => {
            let address = address_param(spec, "address")?;
            Ok((
                EvmObserveBinding::NativeBalance,
                EvmObserveRequest::NativeBalance { address },
            ))
        }
        "evm.storage_slot" => {
            let address = address_param(spec, "address")?;
            let slot = u256_param(spec, "slot")?;
            Ok((
                EvmObserveBinding::StorageSlot,
                EvmObserveRequest::StorageSlot { address, slot },
            ))
        }
        "evm.erc20_balance_of" => {
            let token = address_param(spec, "token")?;
            let owner = address_param(spec, "owner")?;
            Ok((
                EvmObserveBinding::Erc20BalanceOf,
                EvmObserveRequest::Erc20BalanceOf { token, owner },
            ))
        }
        "evm.erc20_allowance" => {
            let token = address_param(spec, "token")?;
            let owner = address_param(spec, "owner")?;
            let spender = address_param(spec, "spender")?;
            Ok((
                EvmObserveBinding::Erc20Allowance,
                EvmObserveRequest::Erc20Allowance {
                    token,
                    owner,
                    spender,
                },
            ))
        }
        "evm.contract_state_read" => {
            let to = address_param(spec, "to")?;
            let data = bytes_param(spec, "data")?;
            Ok((
                EvmObserveBinding::ContractStateRead,
                EvmObserveRequest::ContractStateRead { to, data },
            ))
        }
        other => Err(format!(
            "execution_artifact observation `{}` uses unsupported kind `{other}`",
            spec.observation_id
        )),
    }
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

fn address_param(spec: &ObservationSpec, key: &str) -> Result<Address, String> {
    parse_address(
        string_param(spec, key)?,
        &format!(
            "execution_artifact.observations.{}.{}",
            spec.observation_id, key
        ),
    )
}

fn bytes_param(spec: &ObservationSpec, key: &str) -> Result<Bytes, String> {
    Bytes::from_str(string_param(spec, key)?).map_err(|error| {
        format!(
            "invalid execution_artifact observation `{}` bytes param `{key}`: {error}",
            spec.observation_id
        )
    })
}

fn u256_param(spec: &ObservationSpec, key: &str) -> Result<U256, String> {
    let value = string_param(spec, key)?;
    if let Some(value) = value.strip_prefix("0x") {
        U256::from_str_radix(value, 16).map_err(|error| {
            format!(
                "invalid execution_artifact observation `{}` hex param `{key}`: {error}",
                spec.observation_id
            )
        })
    } else {
        parse_u256_decimal(value)
    }
}

fn parse_address(value: &str, field: &str) -> Result<Address, String> {
    Address::from_str(value).map_err(|error| format!("invalid {field} address `{value}`: {error}"))
}

fn parse_u256_decimal(value: &str) -> Result<U256, String> {
    U256::from_str_radix(value, 10)
        .map_err(|error| format!("invalid atomic amount `{value}`: {error}"))
}

fn evm_call_request(
    sender: Option<Address>,
    candidate: &EvmTransactionCandidate,
) -> Result<EvmCallRequest, String> {
    let to = parse_address(candidate.to.as_str(), "execution_artifact.transactions.to")?;
    let value = candidate
        .value
        .as_deref()
        .map(parse_u256_decimal)
        .transpose()?;
    let data = parse_optional_calldata_hex(candidate.calldata.as_deref())?;
    Ok(EvmCallRequest {
        from: sender,
        to,
        data,
        value,
    })
}

fn parse_optional_calldata_hex(value: Option<&str>) -> Result<Bytes, String> {
    let Some(value) = value else {
        return Ok(Bytes::default());
    };
    Bytes::from_str(value)
        .map_err(|error| format!("invalid execution_artifact calldata `{value}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn planner_places_post_observations_after_verify() {
        let graph = plan_single_evm_transaction_stage(
            "owliabot.transfer",
            "native_transfer",
            &TransactionStage {
                stage_id: "stage.transfer".into(),
                candidate_ref: "tx.transfer".into(),
                exports: Vec::new(),
                next_stage_id: None,
            },
            &ais_agent_control::execution_artifact::ExecutionTransactionCandidate::EvmTransaction(
                EvmTransactionCandidate {
                    candidate_id: "tx.transfer".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("1".to_owned()),
                    calldata: None,
                },
            ),
            &[ObservationSpec {
                observation_id: "state.pre.sender_balance".to_owned(),
                kind: "evm.native_balance".to_owned(),
                params: BTreeMap::from([(
                    "address".to_owned(),
                    json!("0x2222222222222222222222222222222222222222"),
                )]),
            }],
            &[ObservationSpec {
                observation_id: "state.post.recipient_balance".to_owned(),
                kind: "evm.native_balance".to_owned(),
                params: BTreeMap::from([(
                    "address".to_owned(),
                    json!("0x3333333333333333333333333333333333333333"),
                )]),
            }],
            None,
            Some("http://127.0.0.1:8545"),
            "eip155:8453",
            None,
        )
        .expect("planned graph");

        let pre_observe_id = "artifact.stage.transfer.pre_observe.state.pre.sender_balance";
        let simulate_id = "artifact.stage.transfer.simulate";
        let verify_id = "artifact.stage.transfer.verify";
        let post_observe_id = "artifact.stage.transfer.post_observe.state.post.recipient_balance";

        assert_eq!(graph.roots, vec![pre_observe_id.to_owned()]);
        assert_eq!(graph.terminals, vec![post_observe_id.to_owned()]);
        assert_eq!(
            graph
                .nodes
                .get(pre_observe_id)
                .expect("pre observe node")
                .depends_on,
            Vec::<String>::new()
        );
        assert_eq!(
            graph
                .nodes
                .get(simulate_id)
                .expect("simulate node")
                .depends_on,
            vec![pre_observe_id.to_owned()]
        );
        assert_eq!(
            graph.nodes.get(verify_id).expect("verify node").depends_on,
            vec!["artifact.stage.transfer.actuate".to_owned()]
        );
        assert_eq!(
            graph
                .nodes
                .get(post_observe_id)
                .expect("post observe node")
                .depends_on,
            vec![verify_id.to_owned()]
        );
    }
}
