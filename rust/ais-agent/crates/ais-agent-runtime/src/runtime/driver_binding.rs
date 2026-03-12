use std::collections::{BTreeMap, BTreeSet};

use ais_agent_core::binding::evm::EvmVerifyBinding;
use ais_agent_core::{
    action::{
        kinds::{
            actuate::ActuateLiveBinding,
            observe::ObserveLiveBinding,
            simulate::SimulateLiveBinding,
            verify::{VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::EvmConnectionSpec,
    driver::{
        ActionGraphFragment, DriverBuildOutput, DriverEvmActuateHint, DriverEvmVerifyHint,
        DriverNodeLiveBindingHint,
    },
    envelope::{bind_raw_envelope_action, RawEnvelopeGateError, RuntimeEnvelopeKind},
};
use ais_agent_drivers::api_native::ApiNativeOutput;
use thiserror::Error;

use crate::runtime::ActiveRun;

#[derive(Debug, Clone, Default)]
pub struct DriverBindingContext {
    pub evm_connections_by_chain: BTreeMap<String, EvmConnectionSpec>,
    pub default_evm_connection: Option<EvmConnectionSpec>,
    pub envelope_refs_by_node: BTreeMap<String, String>,
}

impl DriverBindingContext {
    pub fn with_evm_connection(
        mut self,
        chain: impl Into<String>,
        conn: EvmConnectionSpec,
    ) -> Self {
        self.evm_connections_by_chain.insert(chain.into(), conn);
        self
    }

    pub fn with_default_evm_connection(mut self, conn: EvmConnectionSpec) -> Self {
        self.default_evm_connection = Some(conn);
        self
    }

    pub fn with_envelope_ref(
        mut self,
        node_id: impl Into<String>,
        envelope_ref: impl Into<String>,
    ) -> Self {
        self.envelope_refs_by_node
            .insert(node_id.into(), envelope_ref.into());
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimeDriverBindingError {
    #[error("driver fragment live-binding application failed: {0}")]
    LiveBinding(String),
    #[error(
        "driver fragment references duplicate node `{node_id}` already present in runtime graph"
    )]
    DuplicateNode { node_id: String },
    #[error("driver fragment references duplicate evidence requirement `{requirement_id}`")]
    DuplicateEvidenceRequirement { requirement_id: String },
    #[error("driver fragment references duplicate effect contract `{effect_id}`")]
    DuplicateEffectContract { effect_id: String },
    #[error("driver fragment references duplicate runtime envelope `{envelope_id}`")]
    DuplicateEnvelope { envelope_id: String },
    #[error("runtime envelope `{envelope_id}` not found")]
    EnvelopeNotFound { envelope_id: String },
    #[error("raw-envelope binding failed: {0}")]
    RawEnvelope(#[from] RawEnvelopeGateError),
}

#[derive(Debug, Clone, Default)]
pub struct RawEnvelopeBindingRequest {
    pub node_prefix: String,
    pub envelope_ref: String,
    pub effect_contract_ref: Option<String>,
    pub actuator_hint: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Default)]
pub struct RuntimeDriverBinder;

impl RuntimeDriverBinder {
    pub fn bind_output(
        runtime: &mut ActiveRun,
        mut output: DriverBuildOutput,
        ctx: &DriverBindingContext,
    ) -> Result<(), RuntimeDriverBindingError> {
        output
            .apply_live_binding_hints()
            .map_err(|error| RuntimeDriverBindingError::LiveBinding(error.to_string()))?;

        for node in output.fragment.nodes.values_mut() {
            inject_runtime_context(runtime, node, ctx);
        }

        let existing_requirements: BTreeSet<String> = runtime
            .checkpoint
            .evidence_graph
            .requirements
            .iter()
            .map(|req| req.requirement_id.clone())
            .collect();
        for requirement in &output.evidence_requirements {
            if existing_requirements.contains(&requirement.requirement_id) {
                return Err(RuntimeDriverBindingError::DuplicateEvidenceRequirement {
                    requirement_id: requirement.requirement_id.clone(),
                });
            }
        }

        for effect in &output.effect_contracts {
            if runtime
                .checkpoint
                .effect_contracts
                .contains_key(&effect.effect_id)
            {
                return Err(RuntimeDriverBindingError::DuplicateEffectContract {
                    effect_id: effect.effect_id.clone(),
                });
            }
        }

        for node_id in output.fragment.nodes.keys() {
            if runtime.checkpoint.action_graph.nodes.contains_key(node_id) {
                return Err(RuntimeDriverBindingError::DuplicateNode {
                    node_id: node_id.clone(),
                });
            }
        }

        runtime
            .checkpoint
            .action_graph
            .roots
            .extend(output.fragment.roots);
        runtime.checkpoint.action_graph.roots.sort();
        runtime.checkpoint.action_graph.roots.dedup();

        runtime
            .checkpoint
            .action_graph
            .terminals
            .extend(output.fragment.terminals);
        runtime.checkpoint.action_graph.terminals.sort();
        runtime.checkpoint.action_graph.terminals.dedup();

        runtime
            .checkpoint
            .action_graph
            .nodes
            .extend(output.fragment.nodes);
        runtime
            .checkpoint
            .evidence_graph
            .requirements
            .extend(output.evidence_requirements);
        runtime.checkpoint.effect_contracts.extend(
            output
                .effect_contracts
                .into_iter()
                .map(|effect| (effect.effect_id.clone(), effect)),
        );

        Ok(())
    }

    pub fn bind_api_native_output(
        runtime: &mut ActiveRun,
        output: ApiNativeOutput,
        ctx: &DriverBindingContext,
    ) -> Result<(), RuntimeDriverBindingError> {
        for record in output.evidence_records {
            runtime
                .checkpoint
                .evidence_graph
                .records
                .insert(record.evidence_id.clone(), record);
        }

        for envelope in output.runtime_envelopes {
            if runtime.envelopes.contains_key(&envelope.envelope_id) {
                return Err(RuntimeDriverBindingError::DuplicateEnvelope {
                    envelope_id: envelope.envelope_id,
                });
            }
            runtime
                .envelopes
                .insert(envelope.envelope_id.clone(), envelope);
        }

        Self::bind_output(
            runtime,
            DriverBuildOutput {
                fragment: output.fragment,
                evidence_requirements: Vec::new(),
                effect_contracts: output.effect_contracts,
            },
            ctx,
        )
    }

    pub fn bind_raw_envelope_path(
        runtime: &mut ActiveRun,
        request: RawEnvelopeBindingRequest,
        ctx: &DriverBindingContext,
    ) -> Result<(), RuntimeDriverBindingError> {
        let envelope = runtime
            .envelopes
            .get(&request.envelope_ref)
            .cloned()
            .ok_or_else(|| RuntimeDriverBindingError::EnvelopeNotFound {
                envelope_id: request.envelope_ref.clone(),
            })?;

        let actuate_id = format!("{}.actuate", request.node_prefix);
        let verify_id = format!("{}.verify", request.node_prefix);
        let mut actuation = bind_raw_envelope_action(
            actuate_id.clone(),
            &envelope,
            request.effect_contract_ref.clone(),
            request.actuator_hint,
        )?;
        actuation.depends_on = request.depends_on.clone();

        let verify = ActionNode {
            node_id: verify_id.clone(),
            kind: ActionNodeKind::Verify,
            origin: ActionOrigin::RawEnvelopePath,
            status: ActionNodeStatus::Pending,
            depends_on: vec![actuate_id.clone()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Verify(VerifyAction {
                verify_kind: VerifyKind::EffectContract,
                verifier_hint: format!("verify raw envelope {}", request.envelope_ref),
                pre_observation_ref: None,
                post_observation_ref: None,
                live: None,
            }),
            implementation_hint: envelope.provenance.clone(),
            expected_effect_ref: request.effect_contract_ref.clone(),
        };

        let mut nodes = BTreeMap::new();
        nodes.insert(actuate_id.clone(), actuation);
        nodes.insert(verify_id.clone(), verify);

        let mut live_binding_hints = BTreeMap::new();
        if envelope.kind == RuntimeEnvelopeKind::EvmEnvelope {
            live_binding_hints.insert(
                actuate_id.clone(),
                DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
                    binding:
                        ais_agent_core::binding::evm::EvmActuateBinding::BroadcastRawTransaction,
                }),
            );
            live_binding_hints.insert(
                verify_id.clone(),
                DriverNodeLiveBindingHint::EvmVerify(DriverEvmVerifyHint {
                    binding: EvmVerifyBinding::EffectContractFromReceipt,
                    post_evm_request: None,
                }),
            );
        }

        Self::bind_output(
            runtime,
            DriverBuildOutput {
                fragment: ActionGraphFragment {
                    roots: if request.depends_on.is_empty() {
                        vec![actuate_id]
                    } else {
                        Vec::new()
                    },
                    terminals: vec![verify_id],
                    nodes,
                    live_binding_hints,
                },
                evidence_requirements: Vec::new(),
                effect_contracts: Vec::new(),
            },
            ctx,
        )
    }
}

fn inject_runtime_context(runtime: &ActiveRun, node: &mut ActionNode, ctx: &DriverBindingContext) {
    let inferred_chain = inferred_chain_for_node(runtime, node);

    match &mut node.payload {
        ActionPayload::Observe(action) => {
            if let Some(ObserveLiveBinding::Evm(live)) = &mut action.live {
                if live.connection.is_none() {
                    live.connection = resolve_evm_connection(ctx, inferred_chain.as_deref());
                }
            }
        }
        ActionPayload::Simulate(action) => {
            if let Some(SimulateLiveBinding::Evm(live)) = &mut action.live {
                if live.connection.is_none() {
                    live.connection = resolve_evm_connection(ctx, inferred_chain.as_deref());
                }
            }
        }
        ActionPayload::Actuate(action) => {
            if let Some(ActuateLiveBinding::Evm(live)) = &mut action.live {
                if live.connection.is_none() {
                    live.connection = resolve_evm_connection(ctx, inferred_chain.as_deref());
                }
            }
            if matches!(action.live, Some(ActuateLiveBinding::Evm(_)))
                && action.envelope_ref.is_none()
            {
                if let Some(envelope_ref) = ctx.envelope_refs_by_node.get(&node.node_id) {
                    action.envelope_ref = Some(envelope_ref.clone());
                }
            }
        }
        ActionPayload::Verify(action) => {
            if let Some(VerifyLiveBinding::Evm(live)) = &mut action.live {
                if live.connection.is_none() {
                    live.connection = resolve_evm_connection(ctx, inferred_chain.as_deref());
                }
            }
        }
        ActionPayload::Derive(_) | ActionPayload::Recover(_) => {}
    }
}

fn inferred_chain_for_node(runtime: &ActiveRun, node: &ActionNode) -> Option<String> {
    let from_payload = match &node.payload {
        ActionPayload::Actuate(action) => action.chain.clone(),
        _ => None,
    };

    from_payload.or_else(|| runtime.mission.allowed_chains.first().cloned())
}

fn resolve_evm_connection(
    ctx: &DriverBindingContext,
    chain: Option<&str>,
) -> Option<EvmConnectionSpec> {
    chain
        .and_then(|chain| ctx.evm_connections_by_chain.get(chain).cloned())
        .or_else(|| ctx.default_evm_connection.clone())
}
