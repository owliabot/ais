use std::collections::BTreeMap;

use ais_agent_control::execution_artifact::{
    EffectSpec, EvmTransactionCandidate, ExecutionArtifactLaunchSpec, ExecutionChainFamily,
    ExecutionStage, ObservationSpec, SolanaInstructionAccount, SolanaInstructionCandidate,
    SolanaTransactionCandidate, TransactionStage,
};
use ais_agent_core::{
    action::{ActionGraph, ActionPayload},
    checkpoint::{CheckpointSnapshot, ExecutionArtifactRuntimeSnapshot},
    effect::EffectContract,
    evidence::EvidenceGraph,
};
use ais_agent_evm::artifact_planner::{
    plan_execution_artifact as plan_evm_execution_artifact, PlannedEvmExecutionArtifact,
};
use ais_agent_solana::artifact_planner::{
    plan_execution_artifact as plan_solana_execution_artifact, PlannedSolanaExecutionArtifact,
};
use serde_json::json;

use super::api_native_evm_common::RuntimeExecutionWiring;

pub(crate) fn seed_execution_artifact_checkpoint(
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
    spec: &ExecutionArtifactLaunchSpec,
) -> Result<(), String> {
    if !wiring.allows_protocol_package(spec.protocol_package_id.as_str()) {
        return Err(format!(
            "execution_artifact protocol_package_id `{}` is not enabled",
            spec.protocol_package_id
        ));
    }

    let chain_scope = spec.chain_scope().map(str::to_owned).ok_or_else(|| {
        "execution_artifact.allowed_chains must contain exactly one chain scope".to_owned()
    })?;
    let planned = plan_family_execution_artifact(spec, wiring, chain_scope.as_str())?;

    checkpoint.execution_artifact = Some(ExecutionArtifactRuntimeSnapshot {
        launch_spec: spec.clone(),
        active_stage_id: Some(spec.entry_stage_id.clone()),
        planned_stage_graphs: planned.planned_stage_graphs,
        exported_outputs: BTreeMap::new(),
        branch_trace: Vec::new(),
        awaiting_continuation: None,
    });
    activate_execution_artifact_stage(checkpoint)?;
    checkpoint.evidence_graph = EvidenceGraph::default();
    checkpoint.effect_contracts = planned.effect_contracts;
    Ok(())
}

pub(crate) fn activate_execution_artifact_stage(
    checkpoint: &mut CheckpointSnapshot,
) -> Result<(), String> {
    let Some(snapshot) = checkpoint.execution_artifact.as_ref() else {
        return Ok(());
    };
    let Some(active_stage_id) = snapshot.active_stage_id.as_ref() else {
        return Ok(());
    };
    let stage = snapshot
        .launch_spec
        .stage(active_stage_id.as_str())
        .ok_or_else(|| {
            format!(
                "execution_artifact runtime references unknown active stage `{active_stage_id}`"
            )
        })?;
    checkpoint.action_graph = match stage {
        ExecutionStage::Transaction(stage) => snapshot
            .planned_stage_graphs
            .get(&stage.stage_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "execution_artifact planned graph missing for transaction stage `{}`",
                    stage.stage_id
                )
            })?,
        ExecutionStage::Observe(stage) => snapshot
            .planned_stage_graphs
            .get(&stage.stage_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "execution_artifact planned graph missing for observe stage `{}`",
                    stage.stage_id
                )
            })?,
        ExecutionStage::Branch(_) | ExecutionStage::Continuation(_) => ActionGraph {
            graph_id: Some(format!(
                "artifact.{}.{}",
                snapshot.launch_spec.protocol_package_id, snapshot.launch_spec.action_key
            )),
            roots: Vec::new(),
            terminals: Vec::new(),
            nodes: BTreeMap::new(),
        },
    };
    Ok(())
}

#[derive(Debug, Clone)]
struct PlannedExecutionArtifact {
    planned_stage_graphs:
        BTreeMap<ais_agent_control::execution_artifact::ExecutionStageId, ActionGraph>,
    effect_contracts: BTreeMap<String, EffectContract>,
}

impl From<PlannedEvmExecutionArtifact> for PlannedExecutionArtifact {
    fn from(value: PlannedEvmExecutionArtifact) -> Self {
        Self {
            planned_stage_graphs: value.planned_stage_graphs,
            effect_contracts: value.effect_contracts,
        }
    }
}

impl From<PlannedSolanaExecutionArtifact> for PlannedExecutionArtifact {
    fn from(value: PlannedSolanaExecutionArtifact) -> Self {
        Self {
            planned_stage_graphs: value.planned_stage_graphs,
            effect_contracts: value.effect_contracts,
        }
    }
}

fn plan_family_execution_artifact(
    spec: &ExecutionArtifactLaunchSpec,
    wiring: &RuntimeExecutionWiring,
    chain_scope: &str,
) -> Result<PlannedExecutionArtifact, String> {
    match spec.chain_family {
        ExecutionChainFamily::Evm => {
            plan_evm_execution_artifact(spec, wiring.evm_rpc_url.as_deref(), chain_scope)
                .map(Into::into)
        }
        ExecutionChainFamily::Solana => {
            plan_solana_execution_artifact(spec, wiring.solana_rpc_url.as_deref(), chain_scope)
                .map(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ais_agent_core::{
        action::kinds::verify::VerifyKind,
        runtime::{RunLifecycleState, RunPhase},
    };

    #[test]
    fn seed_execution_artifact_checkpoint_persists_expected_effect_contracts_for_evm() {
        let mut checkpoint = sample_checkpoint();
        let spec = ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.transfer".to_owned(),
            action_key: "native_transfer".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["eip155:8453".to_owned()],
            entry_stage_id: "stage.transfer".into(),
            actor: None,
            transactions: vec![
                ais_agent_control::execution_artifact::ExecutionTransactionCandidate::EvmTransaction(
                    EvmTransactionCandidate {
                        candidate_id: "tx.transfer".into(),
                        to: "0x1111111111111111111111111111111111111111".to_owned(),
                        value: Some("1".to_owned()),
                        calldata: None,
                    },
                ),
            ],
            stages: vec![ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.transfer".into(),
                candidate_ref: "tx.transfer".into(),
                exports: Vec::new(),
                next_stage_id: None,
            })],
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: vec![ObservationSpec {
                observation_id: "state.post.recipient_balance".to_owned(),
                kind: "evm.native_balance".to_owned(),
                params: BTreeMap::from([(
                    "address".to_owned(),
                    json!("0x3333333333333333333333333333333333333333"),
                )]),
            }],
            expected_effects: vec![EffectSpec {
                effect_id: "effect.transfer".to_owned(),
                stage_id: "stage.transfer".into(),
                kind: "asset_delta".to_owned(),
                params: BTreeMap::from([(
                    "assertions".to_owned(),
                    json!([{
                        "expression": "receipt.status == true",
                        "description": "transfer receipt must succeed"
                    }]),
                )]),
            }],
            execution_policy: None,
            evidence: json!({}),
            metadata: BTreeMap::new(),
        };
        let wiring = RuntimeExecutionWiring {
            evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
            solana_rpc_url: None,
            allowed_protocol_packages: vec!["owliabot.transfer".to_owned()],
        };

        seed_execution_artifact_checkpoint(&mut checkpoint, &wiring, &spec)
            .expect("seed artifact checkpoint");

        assert!(checkpoint.effect_contracts.contains_key("effect.transfer"));
        let verify = checkpoint
            .action_graph
            .nodes
            .get("artifact.stage.transfer.verify")
            .expect("verify node");
        let ActionPayload::Verify(verify_payload) = &verify.payload else {
            panic!("expected verify payload");
        };
        assert_eq!(verify_payload.verify_kind, VerifyKind::EffectContract);
        assert_eq!(
            verify.expected_effect_ref.as_deref(),
            Some("effect.transfer")
        );
    }

    #[test]
    fn seed_execution_artifact_checkpoint_dispatches_solana_family_planner() {
        let mut checkpoint = sample_checkpoint();
        let spec = ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.solana".to_owned(),
            action_key: "native_transfer".to_owned(),
            chain_family: ExecutionChainFamily::Solana,
            allowed_chains: vec!["solana:mainnet".to_owned()],
            entry_stage_id: "stage.transfer".into(),
            actor: Some(ais_agent_control::execution_artifact::ExecutionArtifactActor {
                sender_address_hint: Some("11111111111111111111111111111111".to_owned()),
                recipient_address: None,
            }),
            transactions: vec![
                ais_agent_control::execution_artifact::ExecutionTransactionCandidate::SolanaTransaction(
                    SolanaTransactionCandidate {
                        candidate_id: "tx.transfer".into(),
                        instructions: vec![SolanaInstructionCandidate {
                            program_id: "11111111111111111111111111111111".to_owned(),
                            accounts: vec![SolanaInstructionAccount {
                                address: "11111111111111111111111111111111".to_owned(),
                                is_signer: true,
                                is_writable: true,
                            }],
                            data_base64: Some("AQID".to_owned()),
                        }],
                    },
                ),
            ],
            stages: vec![ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.transfer".into(),
                candidate_ref: "tx.transfer".into(),
                exports: Vec::new(),
                next_stage_id: None,
            })],
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            evidence: json!({}),
            metadata: BTreeMap::new(),
        };
        let wiring = RuntimeExecutionWiring {
            evm_rpc_url: None,
            solana_rpc_url: Some("http://127.0.0.1:8899".to_owned()),
            allowed_protocol_packages: vec!["owliabot.solana".to_owned()],
        };

        seed_execution_artifact_checkpoint(&mut checkpoint, &wiring, &spec)
            .expect("seed artifact checkpoint");

        assert_eq!(
            checkpoint
                .execution_artifact
                .as_ref()
                .and_then(|artifact| artifact.active_stage_id.as_ref())
                .map(|stage_id| stage_id.as_str()),
            Some("stage.transfer")
        );
        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("artifact.stage.transfer.simulate"));
        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("artifact.stage.transfer.verify"));
    }

    fn sample_checkpoint() -> CheckpointSnapshot {
        let mut lifecycle = RunLifecycleState::new("run-1".into(), "mission-1");
        lifecycle.mark_running(RunPhase::Planning);
        CheckpointSnapshot {
            run_id: "run-1".to_owned(),
            checkpoint_seq: 0,
            mission_id: "mission-1".into(),
            plan_epoch: 0,
            lifecycle,
            action_graph: ActionGraph::default(),
            evidence_graph: EvidenceGraph::default(),
            effect_contracts: BTreeMap::new(),
            pending_requests: ais_agent_core::checkpoint::PendingRequestsSnapshot::default(),
            last_completed_node_id: None,
            actuation_records: Vec::new(),
            execution_artifact: None,
        }
    }
}
