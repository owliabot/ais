use std::{collections::BTreeMap, str::FromStr};

use ais_agent_control::transfer::{
    Erc20TransferRequest, NativeTransferRequest, TransferActionFamily, TransferAmountEvidence,
    TransferEvidencePackage, TransferVerificationContract,
};
use ais_agent_control::uniswap::{
    UniswapLpOperationKind, UniswapSwapAmountMode, UniswapV3LpEvidencePackage, UniswapV3LpRequest,
    UniswapV3LpVerificationContract, UniswapV3SwapEvidencePackage, UniswapV3SwapRequest,
    UniswapV3SwapVerificationContract,
};
use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            observe::{
                EvmObserveLiveBinding, ObserveAction, ObserveLiveBinding, ObserveSourceKind,
            },
            simulate::{EvmSimulateLiveBinding, SimulateAction, SimulateKind, SimulateLiveBinding},
            verify::{EvmVerifyLiveBinding, VerifyAction, VerifyKind, VerifyLiveBinding},
        },
        ActionGraph, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{
        EvmCallRequest, EvmConnectionSpec, EvmObserveBinding, EvmObserveRequest,
        EvmSimulateBinding, EvmVerifyBinding,
    },
    checkpoint::CheckpointSnapshot,
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    evidence::{
        EvidenceFreshness, EvidenceGraph, EvidenceKind, EvidenceProvenance, EvidenceRecord,
        EvidenceRequirement,
    },
    mission::Mission,
    transfer::{Erc20TransferEffectTemplate, NativeTransferEffectTemplate, TransferEffectTemplate},
    uniswap::{UniswapV3EffectTemplate, UniswapV3LpEffectTemplate, UniswapV3SwapEffectTemplate},
};
use alloy::primitives::{Address, Bytes, U256};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct RuntimeExecutionWiring {
    pub evm_rpc_url: Option<String>,
    pub solana_rpc_url: Option<String>,
    pub native_transfer_enabled: bool,
    pub erc20_transfer_enabled: bool,
    pub uniswap_v3_swap_enabled: bool,
    pub uniswap_v3_lp_enabled: bool,
}

pub(crate) fn seed_action_family_checkpoint(
    mission: &Mission,
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
) -> Result<(), String> {
    let Some(action_family) = mission
        .constraints
        .get("owliabot_action_family")
        .and_then(Value::as_str)
    else {
        return Ok(());
    };

    match action_family {
        "native_transfer" => seed_native_transfer_checkpoint(mission, checkpoint, wiring),
        "erc20_transfer" => seed_erc20_transfer_checkpoint(mission, checkpoint, wiring),
        "uniswap_v3_swap" => seed_uniswap_v3_swap_checkpoint(mission, checkpoint, wiring),
        "uniswap_v3_lp" => seed_uniswap_v3_lp_checkpoint(mission, checkpoint, wiring),
        _ => Ok(()),
    }
}

fn seed_native_transfer_checkpoint(
    mission: &Mission,
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
) -> Result<(), String> {
    if !wiring.native_transfer_enabled {
        return Err("native_transfer is disabled in this ais-agent service".to_owned());
    }
    let rpc_url = wiring
        .evm_rpc_url
        .clone()
        .ok_or_else(|| "native_transfer requires service.providers.evm_rpc_url".to_owned())?;

    let submission = transfer_submission::<NativeTransferRequest>(mission)?;
    submission.payload.validate().map_err(str::to_owned)?;
    submission
        .evidence
        .validate_for(TransferActionFamily::NativeTransfer)
        .map_err(str::to_owned)?;

    let chain_scope = normalize_evm_chain_scope(&submission.payload.chain)?;
    let recipient = parse_address(
        submission.evidence.recipient.normalized_address.as_str(),
        "recipient",
    )?;
    let amount_atomic = parse_transfer_amount_atomic(&submission.evidence.amount, 18)?;

    checkpoint.action_graph =
        native_transfer_graph(&chain_scope, &rpc_url, recipient, amount_atomic);
    checkpoint.evidence_graph = native_transfer_evidence_graph(&submission.evidence);

    let effect_template = TransferEffectTemplate::Native(NativeTransferEffectTemplate {
        request: submission.payload.clone(),
        verification: TransferVerificationContract {
            chain: chain_scope.clone(),
            token_address: None,
            recipient_address: format!("{recipient:#x}"),
            expected_amount_atomic: amount_atomic.to_string(),
            sender_address_hint: submission.payload.sender_address_hint.clone(),
            require_recipient_delta: true,
            require_sender_delta: false,
        },
    });

    checkpoint.effect_contracts = BTreeMap::from([(
        "effect.native_transfer".to_owned(),
        effect_template.to_effect_contract("effect.native_transfer"),
    )]);

    Ok(())
}

fn seed_erc20_transfer_checkpoint(
    mission: &Mission,
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
) -> Result<(), String> {
    if !wiring.erc20_transfer_enabled {
        return Err("erc20_transfer is disabled in this ais-agent service".to_owned());
    }
    let rpc_url = wiring
        .evm_rpc_url
        .clone()
        .ok_or_else(|| "erc20_transfer requires service.providers.evm_rpc_url".to_owned())?;

    let submission = transfer_submission::<Erc20TransferRequest>(mission)?;
    submission.payload.validate().map_err(str::to_owned)?;
    submission
        .evidence
        .validate_for(TransferActionFamily::Erc20Transfer)
        .map_err(str::to_owned)?;

    let chain_scope = normalize_evm_chain_scope(&submission.payload.chain)?;
    let token_evidence = submission
        .evidence
        .token
        .as_ref()
        .ok_or_else(|| "erc20_transfer requires transfer_evidence.token".to_owned())?;
    let token = parse_address(token_evidence.token_address.as_str(), "token_address")?;
    let recipient = parse_address(
        submission.evidence.recipient.normalized_address.as_str(),
        "recipient",
    )?;
    let sender = submission
        .payload
        .sender_address_hint
        .as_deref()
        .map(|value| parse_address(value, "sender_address_hint"))
        .transpose()?;
    let amount_atomic =
        parse_transfer_amount_atomic(&submission.evidence.amount, token_evidence.decimals)?;

    checkpoint.action_graph = erc20_transfer_graph(
        &chain_scope,
        &rpc_url,
        token,
        recipient,
        sender,
        amount_atomic,
    );
    checkpoint.evidence_graph = erc20_transfer_evidence_graph(&submission.evidence);

    let effect_template = TransferEffectTemplate::Erc20(Erc20TransferEffectTemplate {
        request: submission.payload.clone(),
        verification: TransferVerificationContract {
            chain: chain_scope.clone(),
            token_address: Some(format!("{token:#x}")),
            recipient_address: format!("{recipient:#x}"),
            expected_amount_atomic: amount_atomic.to_string(),
            sender_address_hint: submission.payload.sender_address_hint.clone(),
            require_recipient_delta: true,
            require_sender_delta: false,
        },
    });

    checkpoint.effect_contracts = BTreeMap::from([(
        "effect.erc20_transfer".to_owned(),
        effect_template.to_effect_contract("effect.erc20_transfer"),
    )]);

    Ok(())
}

fn seed_uniswap_v3_swap_checkpoint(
    mission: &Mission,
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
) -> Result<(), String> {
    if !wiring.uniswap_v3_swap_enabled {
        return Err("uniswap_v3_swap is disabled in this ais-agent service".to_owned());
    }
    let rpc_url = wiring
        .evm_rpc_url
        .clone()
        .ok_or_else(|| "uniswap_v3_swap requires service.providers.evm_rpc_url".to_owned())?;

    let submission =
        owliabot_submission::<UniswapV3SwapRequest, UniswapV3SwapEvidencePackage>(mission)?;
    submission.payload.validate().map_err(str::to_owned)?;
    submission
        .evidence
        .validate_for(&submission.payload)
        .map_err(str::to_owned)?;

    if !matches!(
        submission.payload.amount_mode,
        UniswapSwapAmountMode::ExactIn
    ) {
        return Err(
            "uniswap_v3_swap exact_out is not yet enabled in this ais-agent service slice"
                .to_owned(),
        );
    }

    let chain_scope = normalize_evm_chain_scope(&submission.payload.chain)?;
    let token_in = parse_address(
        submission.payload.token_in_address.as_str(),
        "token_in_address",
    )?;
    let token_out = parse_address(
        submission.payload.token_out_address.as_str(),
        "token_out_address",
    )?;
    let router = parse_address(submission.payload.router_address.as_str(), "router_address")?;
    let approval_target = submission
        .evidence
        .router
        .approval_target_address
        .as_deref()
        .map(|value| parse_address(value, "approval_target_address"))
        .transpose()?;
    let recipient = submission
        .payload
        .recipient_address
        .as_deref()
        .or(submission.payload.sender_address_hint.as_deref())
        .ok_or_else(|| {
            "uniswap_v3_swap requires recipient_address or sender_address_hint".to_owned()
        })
        .and_then(|value| parse_address(value, "recipient_address"))?;
    let sender = submission
        .payload
        .sender_address_hint
        .as_deref()
        .map(|value| parse_address(value, "sender_address_hint"))
        .transpose()?;
    let approval_required = submission.evidence.router.approval_required;

    let amount_in_atomic = submission
        .evidence
        .quote
        .amount_in_atomic
        .as_deref()
        .map(parse_u256_decimal)
        .transpose()?
        .unwrap_or_else(|| {
            decimal_amount_to_u256(
                &submission.payload.requested_amount,
                submission.evidence.token_in.decimals,
            )
            .expect("validated exact_in requested amount should parse")
        });
    let min_amount_out_atomic = submission
        .evidence
        .quote
        .min_amount_out_atomic
        .as_deref()
        .or(submission.evidence.quote.amount_out_atomic.as_deref())
        .ok_or_else(|| {
            "uniswap_v3_swap exact_in requires quote.min_amount_out_atomic or amount_out_atomic"
                .to_owned()
        })
        .and_then(parse_u256_decimal)?;
    let deadline_unix_seconds = submission.evidence.deadline.deadline_unix_seconds;

    checkpoint.action_graph = if approval_required {
        let approval_target = approval_target.ok_or_else(|| {
            "uniswap_v3_swap approval-required path needs approval_target_address".to_owned()
        })?;
        let sender = sender.ok_or_else(|| {
            "uniswap_v3_swap approval-required path needs sender_address_hint".to_owned()
        })?;
        uniswap_v3_swap_graph_with_approval(
            &chain_scope,
            &rpc_url,
            token_in,
            token_out,
            router,
            approval_target,
            recipient,
            sender,
            submission.payload.fee_tier,
            deadline_unix_seconds,
            amount_in_atomic,
            min_amount_out_atomic,
        )
    } else {
        uniswap_v3_swap_graph(
            &chain_scope,
            &rpc_url,
            token_in,
            token_out,
            router,
            recipient,
            sender,
            submission.payload.fee_tier,
            deadline_unix_seconds,
            amount_in_atomic,
            min_amount_out_atomic,
        )
    };
    checkpoint.evidence_graph = uniswap_v3_swap_evidence_graph(&submission.evidence);

    let effect_template = UniswapV3EffectTemplate::Swap(UniswapV3SwapEffectTemplate {
        request: submission.payload.clone(),
        verification: UniswapV3SwapVerificationContract {
            chain: chain_scope.clone(),
            token_in_address: format!("{token_in:#x}"),
            token_out_address: format!("{token_out:#x}"),
            fee_tier: submission.payload.fee_tier,
            recipient_address: format!("{recipient:#x}"),
            amount_mode: submission.payload.amount_mode.clone(),
            quoted_amount_in_atomic: submission.evidence.quote.amount_in_atomic.clone(),
            quoted_amount_out_atomic: submission.evidence.quote.amount_out_atomic.clone(),
            min_amount_out_atomic: submission.evidence.quote.min_amount_out_atomic.clone(),
            max_amount_in_atomic: submission.evidence.quote.max_amount_in_atomic.clone(),
            router_address: format!("{router:#x}"),
            deadline_unix_seconds,
            sender_address_hint: submission.payload.sender_address_hint.clone(),
            require_recipient_out_delta: true,
        },
    });

    checkpoint.effect_contracts = BTreeMap::from([
        (
            "effect.uniswap_v3_swap".to_owned(),
            effect_template.to_effect_contract("effect.uniswap_v3_swap"),
        ),
        (
            "effect.uniswap_v3_swap.approval".to_owned(),
            uniswap_v3_approval_effect_contract("effect.uniswap_v3_swap.approval"),
        ),
    ]);

    Ok(())
}

fn seed_uniswap_v3_lp_checkpoint(
    mission: &Mission,
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
) -> Result<(), String> {
    if !wiring.uniswap_v3_lp_enabled {
        return Err("uniswap_v3_lp is disabled in this ais-agent service".to_owned());
    }
    let rpc_url = wiring
        .evm_rpc_url
        .clone()
        .ok_or_else(|| "uniswap_v3_lp requires service.providers.evm_rpc_url".to_owned())?;

    let submission =
        owliabot_submission::<UniswapV3LpRequest, UniswapV3LpEvidencePackage>(mission)?;
    submission.payload.validate().map_err(str::to_owned)?;
    submission
        .evidence
        .validate_for(&submission.payload)
        .map_err(str::to_owned)?;

    if !matches!(submission.payload.operation, UniswapLpOperationKind::Mint) {
        return Err(
            "uniswap_v3_lp currently supports only bounded mint in this ais-agent service slice"
                .to_owned(),
        );
    }

    let chain_scope = normalize_evm_chain_scope(&submission.payload.chain)?;
    let position_manager = parse_address(
        submission.payload.position_manager_address.as_str(),
        "position_manager_address",
    )?;
    let owner = submission
        .payload
        .sender_address_hint
        .as_deref()
        .ok_or_else(|| "uniswap_v3_lp mint requires sender_address_hint".to_owned())
        .and_then(|value| parse_address(value, "sender_address_hint"))?;
    let token0 = parse_address(submission.payload.token0_address.as_str(), "token0_address")?;
    let token1 = parse_address(submission.payload.token1_address.as_str(), "token1_address")?;
    let tick_lower = submission
        .payload
        .tick_lower
        .ok_or_else(|| "uniswap_v3_lp mint requires tick_lower".to_owned())?;
    let tick_upper = submission
        .payload
        .tick_upper
        .ok_or_else(|| "uniswap_v3_lp mint requires tick_upper".to_owned())?;
    let amount0_desired = parse_optional_decimal_amount(
        submission.payload.desired_amount0.as_deref(),
        submission.evidence.token0.decimals,
    )?;
    let amount1_desired = parse_optional_decimal_amount(
        submission.payload.desired_amount1.as_deref(),
        submission.evidence.token1.decimals,
    )?;
    let deadline_unix_seconds = submission
        .evidence
        .deadline
        .as_ref()
        .map(|deadline| deadline.deadline_unix_seconds)
        .or(submission.payload.deadline_seconds)
        .ok_or_else(|| {
            "uniswap_v3_lp mint requires deadline evidence or deadline_seconds".to_owned()
        })?;

    checkpoint.action_graph = uniswap_v3_lp_mint_graph(
        &chain_scope,
        &rpc_url,
        position_manager,
        owner,
        token0,
        token1,
        submission.payload.fee_tier,
        tick_lower,
        tick_upper,
        amount0_desired,
        amount1_desired,
        deadline_unix_seconds,
    );
    checkpoint.evidence_graph = uniswap_v3_lp_evidence_graph(&submission.evidence);

    let effect_template = UniswapV3EffectTemplate::Lp(UniswapV3LpEffectTemplate {
        request: submission.payload.clone(),
        verification: UniswapV3LpVerificationContract {
            chain: chain_scope.clone(),
            operation: submission.payload.operation.clone(),
            position_manager_address: format!("{position_manager:#x}"),
            pool_address: submission.evidence.pool.pool_address.clone(),
            token0_address: format!("{token0:#x}"),
            token1_address: format!("{token1:#x}"),
            fee_tier: submission.payload.fee_tier,
            position_token_id: None,
            expected_liquidity_delta: None,
            expected_amount0_max: if amount0_desired.is_zero() {
                None
            } else {
                Some(amount0_desired.to_string())
            },
            expected_amount1_max: if amount1_desired.is_zero() {
                None
            } else {
                Some(amount1_desired.to_string())
            },
            tick_lower: Some(tick_lower),
            tick_upper: Some(tick_upper),
        },
    });

    checkpoint.effect_contracts = BTreeMap::from([(
        "effect.uniswap_v3_lp".to_owned(),
        effect_template.to_effect_contract("effect.uniswap_v3_lp"),
    )]);

    Ok(())
}

fn uniswap_v3_swap_graph(
    chain_scope: &str,
    rpc_url: &str,
    token_in: Address,
    token_out: Address,
    router: Address,
    recipient: Address,
    sender: Option<Address>,
    fee_tier: u32,
    deadline_unix_seconds: u64,
    amount_in_atomic: U256,
    min_amount_out_atomic: U256,
) -> ActionGraph {
    let connection = EvmConnectionSpec {
        rpc_url: rpc_url.to_owned(),
    };

    let observe_node_id = "observe.uniswap_v3_swap.recipient_out_balance".to_owned();
    let simulate_node_id = "simulate.uniswap_v3_swap.call".to_owned();
    let actuate_node_id = "actuate.uniswap_v3_swap.send".to_owned();
    let verify_node_id = "verify.uniswap_v3_swap.effect".to_owned();

    ActionGraph {
        graph_id: Some("graph.uniswap_v3_swap".to_owned()),
        roots: vec![observe_node_id.clone()],
        terminals: vec![verify_node_id.clone()],
        nodes: BTreeMap::from([
            (
                observe_node_id.clone(),
                ActionNode {
                    node_id: observe_node_id.clone(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.token_out".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                    ],
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::ChainRead,
                        source_hint:
                            "capture recipient output-token balance before Uniswap V3 swap"
                                .to_owned(),
                        output_key: Some("state.pre.uniswap_v3_swap.recipient_out_balance".to_owned()),
                        live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmObserveBinding::Erc20BalanceOf,
                            request: EvmObserveRequest::Erc20BalanceOf {
                                token: token_out,
                                owner: recipient,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                simulate_node_id.clone(),
                ActionNode {
                    node_id: simulate_node_id.clone(),
                    kind: ActionNodeKind::Simulate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![observe_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.token_out".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.deadline".to_owned(),
                    ],
                    payload: ActionPayload::Simulate(SimulateAction {
                        simulate_kind: SimulateKind::Call,
                        simulator_hint:
                            "simulate Uniswap V3 exactInputSingle swap (pre-approved path)"
                                .to_owned(),
                        live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: sender,
                                to: router,
                                data: encode_uniswap_v3_exact_input_single_calldata(
                                    token_in,
                                    token_out,
                                    fee_tier,
                                    recipient,
                                    deadline_unix_seconds,
                                    amount_in_atomic,
                                    min_amount_out_atomic,
                                ),
                                value: None,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                actuate_node_id.clone(),
                ActionNode {
                    node_id: actuate_node_id.clone(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![simulate_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.token_out".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.deadline".to_owned(),
                    ],
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: format!(
                            "submit Uniswap V3 exactInputSingle swap on {chain_scope} router {router:#x} token_in {token_in:#x} token_out {token_out:#x}"
                        ),
                        chain: Some(chain_scope.to_owned()),
                        envelope_ref: None,
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_swap".to_owned()),
                },
            ),
            (
                verify_node_id.clone(),
                ActionNode {
                    node_id: verify_node_id.clone(),
                    kind: ActionNodeKind::Verify,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![actuate_node_id],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Verify(VerifyAction {
                        verify_kind: VerifyKind::EffectContract,
                        verifier_hint:
                            "verify recipient output-token balance delta after Uniswap V3 swap"
                                .to_owned(),
                        pre_observation_ref: Some(
                            "state.pre.uniswap_v3_swap.recipient_out_balance".to_owned(),
                        ),
                        post_observation_ref: Some(
                            "state.post.uniswap_v3_swap.recipient_out_balance".to_owned(),
                        ),
                        live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                            connection: Some(connection),
                            binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                            post_request: Some(EvmObserveRequest::Erc20BalanceOf {
                                token: token_out,
                                owner: recipient,
                            }),
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_swap".to_owned()),
                },
            ),
        ]),
    }
}

fn uniswap_v3_swap_graph_with_approval(
    chain_scope: &str,
    rpc_url: &str,
    token_in: Address,
    token_out: Address,
    router: Address,
    approval_target: Address,
    recipient: Address,
    sender: Address,
    fee_tier: u32,
    deadline_unix_seconds: u64,
    amount_in_atomic: U256,
    min_amount_out_atomic: U256,
) -> ActionGraph {
    let connection = EvmConnectionSpec {
        rpc_url: rpc_url.to_owned(),
    };

    let observe_out_node_id = "observe.uniswap_v3_swap.recipient_out_balance".to_owned();
    let observe_allowance_node_id = "observe.uniswap_v3_swap.allowance".to_owned();
    let simulate_approval_node_id = "simulate.uniswap_v3_swap.approve_call".to_owned();
    let actuate_approval_node_id = "actuate.uniswap_v3_swap.approve".to_owned();
    let verify_approval_node_id = "verify.uniswap_v3_swap.approval".to_owned();
    let simulate_swap_node_id = "simulate.uniswap_v3_swap.call".to_owned();
    let actuate_swap_node_id = "actuate.uniswap_v3_swap.send".to_owned();
    let verify_swap_node_id = "verify.uniswap_v3_swap.effect".to_owned();

    ActionGraph {
        graph_id: Some("graph.uniswap_v3_swap".to_owned()),
        roots: vec![observe_out_node_id.clone(), observe_allowance_node_id.clone()],
        terminals: vec![verify_swap_node_id.clone()],
        nodes: BTreeMap::from([
            (
                observe_out_node_id.clone(),
                ActionNode {
                    node_id: observe_out_node_id.clone(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.token_out".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                    ],
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::ChainRead,
                        source_hint:
                            "capture recipient output-token balance before Uniswap V3 swap"
                                .to_owned(),
                        output_key: Some(
                            "state.pre.uniswap_v3_swap.recipient_out_balance".to_owned(),
                        ),
                        live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmObserveBinding::Erc20BalanceOf,
                            request: EvmObserveRequest::Erc20BalanceOf {
                                token: token_out,
                                owner: recipient,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                observe_allowance_node_id.clone(),
                ActionNode {
                    node_id: observe_allowance_node_id.clone(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                    ],
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::ChainRead,
                        source_hint: "capture token-in allowance before approve+swap flow"
                            .to_owned(),
                        output_key: Some("state.pre.uniswap_v3_swap.allowance".to_owned()),
                        live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmObserveBinding::Erc20Allowance,
                            request: EvmObserveRequest::Erc20Allowance {
                                token: token_in,
                                owner: sender,
                                spender: approval_target,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                simulate_approval_node_id.clone(),
                ActionNode {
                    node_id: simulate_approval_node_id.clone(),
                    kind: ActionNodeKind::Simulate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![observe_allowance_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                    ],
                    payload: ActionPayload::Simulate(SimulateAction {
                        simulate_kind: SimulateKind::Call,
                        simulator_hint:
                            "simulate ERC20 approve before Uniswap V3 swap".to_owned(),
                        live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: Some(sender),
                                to: token_in,
                                data: encode_erc20_approve_calldata(
                                    approval_target,
                                    amount_in_atomic,
                                ),
                                value: None,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                actuate_approval_node_id.clone(),
                ActionNode {
                    node_id: actuate_approval_node_id.clone(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![simulate_approval_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                    ],
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: format!(
                            "submit ERC20 approve for Uniswap V3 router path on {chain_scope} token_in {token_in:#x} spender {approval_target:#x} amount {amount_in_atomic}"
                        ),
                        chain: Some(chain_scope.to_owned()),
                        envelope_ref: None,
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_swap.approval".to_owned()),
                },
            ),
            (
                verify_approval_node_id.clone(),
                ActionNode {
                    node_id: verify_approval_node_id.clone(),
                    kind: ActionNodeKind::Verify,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![actuate_approval_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Verify(VerifyAction {
                        verify_kind: VerifyKind::EffectContract,
                        verifier_hint:
                            "verify ERC20 allowance changed before Uniswap V3 swap".to_owned(),
                        pre_observation_ref: Some("state.pre.uniswap_v3_swap.allowance".to_owned()),
                        post_observation_ref: Some(
                            "state.post.uniswap_v3_swap.allowance".to_owned(),
                        ),
                        live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                            post_request: Some(EvmObserveRequest::Erc20Allowance {
                                token: token_in,
                                owner: sender,
                                spender: approval_target,
                            }),
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_swap.approval".to_owned()),
                },
            ),
            (
                simulate_swap_node_id.clone(),
                ActionNode {
                    node_id: simulate_swap_node_id.clone(),
                    kind: ActionNodeKind::Simulate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![observe_out_node_id.clone(), verify_approval_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.token_out".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.deadline".to_owned(),
                    ],
                    payload: ActionPayload::Simulate(SimulateAction {
                        simulate_kind: SimulateKind::Call,
                        simulator_hint:
                            "simulate Uniswap V3 exactInputSingle swap after approval"
                                .to_owned(),
                        live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: Some(sender),
                                to: router,
                                data: encode_uniswap_v3_exact_input_single_calldata(
                                    token_in,
                                    token_out,
                                    fee_tier,
                                    recipient,
                                    deadline_unix_seconds,
                                    amount_in_atomic,
                                    min_amount_out_atomic,
                                ),
                                value: None,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                actuate_swap_node_id.clone(),
                ActionNode {
                    node_id: actuate_swap_node_id.clone(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![simulate_swap_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.swap.token_in".to_owned(),
                        "evidence.uniswap.swap.token_out".to_owned(),
                        "evidence.uniswap.swap.quote".to_owned(),
                        "evidence.uniswap.swap.router".to_owned(),
                        "evidence.uniswap.swap.deadline".to_owned(),
                    ],
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: format!(
                            "submit Uniswap V3 exactInputSingle swap on {chain_scope} router {router:#x} token_in {token_in:#x} token_out {token_out:#x}"
                        ),
                        chain: Some(chain_scope.to_owned()),
                        envelope_ref: None,
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_swap".to_owned()),
                },
            ),
            (
                verify_swap_node_id.clone(),
                ActionNode {
                    node_id: verify_swap_node_id.clone(),
                    kind: ActionNodeKind::Verify,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![actuate_swap_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Verify(VerifyAction {
                        verify_kind: VerifyKind::EffectContract,
                        verifier_hint:
                            "verify recipient output-token balance delta after Uniswap V3 swap"
                                .to_owned(),
                        pre_observation_ref: Some(
                            "state.pre.uniswap_v3_swap.recipient_out_balance".to_owned(),
                        ),
                        post_observation_ref: Some(
                            "state.post.uniswap_v3_swap.recipient_out_balance".to_owned(),
                        ),
                        live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                            connection: Some(connection),
                            binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                            post_request: Some(EvmObserveRequest::Erc20BalanceOf {
                                token: token_out,
                                owner: recipient,
                            }),
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_swap".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_swap".to_owned()),
                },
            ),
        ]),
    }
}

fn uniswap_v3_swap_evidence_graph(evidence: &UniswapV3SwapEvidencePackage) -> EvidenceGraph {
    let token_in_id = "evidence.uniswap.swap.token_in".to_owned();
    let token_out_id = "evidence.uniswap.swap.token_out".to_owned();
    let quote_id = "evidence.uniswap.swap.quote".to_owned();
    let router_id = "evidence.uniswap.swap.router".to_owned();
    let deadline_id = "evidence.uniswap.swap.deadline".to_owned();

    EvidenceGraph {
        records: BTreeMap::from([
            (
                token_in_id.clone(),
                evidence_record(
                    &token_in_id,
                    EvidenceKind::Fact,
                    "owliabot.uniswap.swap.token_in",
                    &serde_json::json!({
                        "token_address": evidence.token_in.token_address,
                        "token_symbol": evidence.token_in.token_symbol,
                        "decimals": evidence.token_in.decimals,
                        "resolution_source": evidence.token_in.resolution_source,
                        "user_confirmed": evidence.token_in.user_confirmed,
                    }),
                    evidence.quote.quoted_at_ms,
                ),
            ),
            (
                token_out_id.clone(),
                evidence_record(
                    &token_out_id,
                    EvidenceKind::Fact,
                    "owliabot.uniswap.swap.token_out",
                    &serde_json::json!({
                        "token_address": evidence.token_out.token_address,
                        "token_symbol": evidence.token_out.token_symbol,
                        "decimals": evidence.token_out.decimals,
                        "resolution_source": evidence.token_out.resolution_source,
                        "user_confirmed": evidence.token_out.user_confirmed,
                    }),
                    evidence.quote.quoted_at_ms,
                ),
            ),
            (
                quote_id.clone(),
                evidence_record_with_freshness(
                    &quote_id,
                    EvidenceKind::ExternalObservation,
                    "owliabot.uniswap.swap.quote",
                    &serde_json::json!({
                        "source": evidence.quote.source,
                        "quoted_at_ms": evidence.quote.quoted_at_ms,
                        "expires_at_ms": evidence.quote.expires_at_ms,
                        "route_summary": evidence.quote.route_summary,
                        "amount_in_atomic": evidence.quote.amount_in_atomic,
                        "amount_out_atomic": evidence.quote.amount_out_atomic,
                        "min_amount_out_atomic": evidence.quote.min_amount_out_atomic,
                        "max_amount_in_atomic": evidence.quote.max_amount_in_atomic,
                        "user_confirmed": evidence.quote.user_confirmed,
                    }),
                    evidence.quote.quoted_at_ms,
                    evidence.quote.expires_at_ms,
                ),
            ),
            (
                router_id.clone(),
                evidence_record(
                    &router_id,
                    EvidenceKind::Fact,
                    "owliabot.uniswap.swap.router",
                    &serde_json::json!({
                        "router_address": evidence.router.router_address,
                        "approval_target_address": evidence.router.approval_target_address,
                        "approval_required": evidence.router.approval_required,
                        "quoter_address": evidence.router.quoter_address,
                        "resolution_source": evidence.router.resolution_source,
                        "user_confirmed": evidence.router.user_confirmed,
                    }),
                    evidence.quote.quoted_at_ms,
                ),
            ),
            (
                deadline_id.clone(),
                evidence_record_with_freshness(
                    &deadline_id,
                    EvidenceKind::Fact,
                    "owliabot.uniswap.swap.deadline",
                    &serde_json::json!({
                        "deadline_unix_seconds": evidence.deadline.deadline_unix_seconds,
                        "source": evidence.deadline.source,
                        "user_confirmed": evidence.deadline.user_confirmed,
                    }),
                    evidence.quote.quoted_at_ms,
                    Some(evidence.deadline.deadline_unix_seconds.saturating_mul(1000)),
                ),
            ),
        ]),
        requirements: vec![
            EvidenceRequirement {
                requirement_id: token_in_id.clone(),
                reference: token_in_id.clone(),
                reason: "token_in must be resolved before Uniswap V3 swap execution".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_swap.send".to_owned()),
                satisfied_by_evidence_id: Some(token_in_id),
            },
            EvidenceRequirement {
                requirement_id: token_out_id.clone(),
                reference: token_out_id.clone(),
                reason: "token_out must be resolved before Uniswap V3 swap execution".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_swap.send".to_owned()),
                satisfied_by_evidence_id: Some(token_out_id),
            },
            EvidenceRequirement {
                requirement_id: quote_id.clone(),
                reference: quote_id.clone(),
                reason: "quote must be fresh before Uniswap V3 swap execution".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_swap.send".to_owned()),
                satisfied_by_evidence_id: Some(quote_id),
            },
            EvidenceRequirement {
                requirement_id: router_id.clone(),
                reference: router_id.clone(),
                reason: "router must be resolved before Uniswap V3 swap execution".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_swap.send".to_owned()),
                satisfied_by_evidence_id: Some(router_id),
            },
            EvidenceRequirement {
                requirement_id: deadline_id.clone(),
                reference: deadline_id.clone(),
                reason: "deadline must remain valid before Uniswap V3 swap execution".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_swap.send".to_owned()),
                satisfied_by_evidence_id: Some(deadline_id),
            },
        ],
        usages: Vec::new(),
    }
}

fn uniswap_v3_lp_mint_graph(
    chain_scope: &str,
    rpc_url: &str,
    position_manager: Address,
    owner: Address,
    token0: Address,
    token1: Address,
    fee_tier: u32,
    tick_lower: i32,
    tick_upper: i32,
    amount0_desired: U256,
    amount1_desired: U256,
    deadline_unix_seconds: u64,
) -> ActionGraph {
    let connection = EvmConnectionSpec {
        rpc_url: rpc_url.to_owned(),
    };

    let observe_node_id = "observe.uniswap_v3_lp.position_count".to_owned();
    let simulate_node_id = "simulate.uniswap_v3_lp.mint_call".to_owned();
    let actuate_node_id = "actuate.uniswap_v3_lp.mint".to_owned();
    let verify_node_id = "verify.uniswap_v3_lp.effect".to_owned();

    ActionGraph {
        graph_id: Some("graph.uniswap_v3_lp".to_owned()),
        roots: vec![observe_node_id.clone()],
        terminals: vec![verify_node_id.clone()],
        nodes: BTreeMap::from([
            (
                observe_node_id.clone(),
                ActionNode {
                    node_id: observe_node_id.clone(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.lp.token0".to_owned(),
                        "evidence.uniswap.lp.token1".to_owned(),
                        "evidence.uniswap.lp.pool".to_owned(),
                    ],
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::ChainRead,
                        source_hint:
                            "capture position-manager NFT count before Uniswap V3 LP mint"
                                .to_owned(),
                        output_key: Some("state.pre.uniswap_v3_lp.position_count".to_owned()),
                        live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmObserveBinding::ContractStateRead,
                            request: EvmObserveRequest::ContractStateRead {
                                to: position_manager,
                                data: encode_balance_of_calldata(owner),
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_lp".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                simulate_node_id.clone(),
                ActionNode {
                    node_id: simulate_node_id.clone(),
                    kind: ActionNodeKind::Simulate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![observe_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.lp.token0".to_owned(),
                        "evidence.uniswap.lp.token1".to_owned(),
                        "evidence.uniswap.lp.pool".to_owned(),
                        "evidence.uniswap.lp.deadline".to_owned(),
                    ],
                    payload: ActionPayload::Simulate(SimulateAction {
                        simulate_kind: SimulateKind::Call,
                        simulator_hint:
                            "simulate Uniswap V3 position-manager mint (pre-approved path)"
                                .to_owned(),
                        live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: Some(owner),
                                to: position_manager,
                                data: encode_uniswap_v3_mint_calldata(
                                    token0,
                                    token1,
                                    fee_tier,
                                    tick_lower,
                                    tick_upper,
                                    amount0_desired,
                                    amount1_desired,
                                    owner,
                                    deadline_unix_seconds,
                                ),
                                value: None,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_lp".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                actuate_node_id.clone(),
                ActionNode {
                    node_id: actuate_node_id.clone(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![simulate_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.uniswap.lp.token0".to_owned(),
                        "evidence.uniswap.lp.token1".to_owned(),
                        "evidence.uniswap.lp.pool".to_owned(),
                        "evidence.uniswap.lp.deadline".to_owned(),
                    ],
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: format!(
                            "submit Uniswap V3 LP mint on {chain_scope} position_manager {position_manager:#x} token0 {token0:#x} token1 {token1:#x}"
                        ),
                        chain: Some(chain_scope.to_owned()),
                        envelope_ref: None,
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_lp".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_lp".to_owned()),
                },
            ),
            (
                verify_node_id.clone(),
                ActionNode {
                    node_id: verify_node_id.clone(),
                    kind: ActionNodeKind::Verify,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![actuate_node_id],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Verify(VerifyAction {
                        verify_kind: VerifyKind::EffectContract,
                        verifier_hint:
                            "verify position-manager NFT count changed after Uniswap V3 LP mint"
                                .to_owned(),
                        pre_observation_ref: Some(
                            "state.pre.uniswap_v3_lp.position_count".to_owned(),
                        ),
                        post_observation_ref: Some(
                            "state.post.uniswap_v3_lp.position_count".to_owned(),
                        ),
                        live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                            connection: Some(connection),
                            binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                            post_request: Some(EvmObserveRequest::ContractStateRead {
                                to: position_manager,
                                data: encode_balance_of_calldata(owner),
                            }),
                        })),
                    }),
                    implementation_hint: Some("owliabot.uniswap_v3_lp".to_owned()),
                    expected_effect_ref: Some("effect.uniswap_v3_lp".to_owned()),
                },
            ),
        ]),
    }
}

fn uniswap_v3_lp_evidence_graph(evidence: &UniswapV3LpEvidencePackage) -> EvidenceGraph {
    let token0_id = "evidence.uniswap.lp.token0".to_owned();
    let token1_id = "evidence.uniswap.lp.token1".to_owned();
    let pool_id = "evidence.uniswap.lp.pool".to_owned();
    let deadline_id = "evidence.uniswap.lp.deadline".to_owned();

    let mut records = BTreeMap::from([
        (
            token0_id.clone(),
            evidence_record(
                &token0_id,
                EvidenceKind::Fact,
                "owliabot.uniswap.lp.token0",
                &serde_json::json!({
                    "token_address": evidence.token0.token_address,
                    "token_symbol": evidence.token0.token_symbol,
                    "decimals": evidence.token0.decimals,
                    "resolution_source": evidence.token0.resolution_source,
                    "user_confirmed": evidence.token0.user_confirmed,
                }),
                evidence.pool.observed_at_ms.unwrap_or_default(),
            ),
        ),
        (
            token1_id.clone(),
            evidence_record(
                &token1_id,
                EvidenceKind::Fact,
                "owliabot.uniswap.lp.token1",
                &serde_json::json!({
                    "token_address": evidence.token1.token_address,
                    "token_symbol": evidence.token1.token_symbol,
                    "decimals": evidence.token1.decimals,
                    "resolution_source": evidence.token1.resolution_source,
                    "user_confirmed": evidence.token1.user_confirmed,
                }),
                evidence.pool.observed_at_ms.unwrap_or_default(),
            ),
        ),
        (
            pool_id.clone(),
            evidence_record(
                &pool_id,
                EvidenceKind::ExternalObservation,
                "owliabot.uniswap.lp.pool",
                &serde_json::json!({
                    "pool_address": evidence.pool.pool_address,
                    "token0_address": evidence.pool.token0_address,
                    "token1_address": evidence.pool.token1_address,
                    "fee_tier": evidence.pool.fee_tier,
                    "tick_spacing": evidence.pool.tick_spacing,
                    "slot0_sqrt_price_x96": evidence.pool.slot0_sqrt_price_x96,
                    "slot0_tick": evidence.pool.slot0_tick,
                    "observed_at_ms": evidence.pool.observed_at_ms,
                    "resolution_source": evidence.pool.resolution_source,
                    "user_confirmed": evidence.pool.user_confirmed,
                }),
                evidence.pool.observed_at_ms.unwrap_or_default(),
            ),
        ),
    ]);

    if let Some(deadline) = &evidence.deadline {
        records.insert(
            deadline_id.clone(),
            evidence_record_with_freshness(
                &deadline_id,
                EvidenceKind::Fact,
                "owliabot.uniswap.lp.deadline",
                &serde_json::json!({
                    "deadline_unix_seconds": deadline.deadline_unix_seconds,
                    "source": deadline.source,
                    "user_confirmed": deadline.user_confirmed,
                }),
                evidence.pool.observed_at_ms.unwrap_or_default(),
                Some(deadline.deadline_unix_seconds.saturating_mul(1000)),
            ),
        );
    }

    let has_deadline = records.contains_key(&deadline_id);

    EvidenceGraph {
        records,
        requirements: vec![
            EvidenceRequirement {
                requirement_id: token0_id.clone(),
                reference: token0_id.clone(),
                reason: "token0 must be resolved before Uniswap V3 LP mint".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_lp.mint".to_owned()),
                satisfied_by_evidence_id: Some(token0_id),
            },
            EvidenceRequirement {
                requirement_id: token1_id.clone(),
                reference: token1_id.clone(),
                reason: "token1 must be resolved before Uniswap V3 LP mint".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_lp.mint".to_owned()),
                satisfied_by_evidence_id: Some(token1_id),
            },
            EvidenceRequirement {
                requirement_id: pool_id.clone(),
                reference: pool_id.clone(),
                reason: "pool facts must be resolved before Uniswap V3 LP mint".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_lp.mint".to_owned()),
                satisfied_by_evidence_id: Some(pool_id),
            },
            EvidenceRequirement {
                requirement_id: deadline_id.clone(),
                reference: deadline_id.clone(),
                reason: "deadline must remain valid before Uniswap V3 LP mint".to_owned(),
                required_by_node_id: Some("actuate.uniswap_v3_lp.mint".to_owned()),
                satisfied_by_evidence_id: has_deadline.then_some(deadline_id),
            },
        ],
        usages: Vec::new(),
    }
}

fn uniswap_v3_approval_effect_contract(effect_id: impl Into<String>) -> EffectContract {
    EffectContract {
        effect_id: effect_id.into(),
        kind: EffectContractKind::AssetDelta,
        assertions: vec![
            EffectAssertion {
                expression: "receipt.status == true".to_owned(),
                description: "approval receipt must succeed before swap".to_owned(),
            },
            EffectAssertion {
                expression: "post.decoded_u256 != pre.decoded_u256".to_owned(),
                description: "allowance should change after approval".to_owned(),
            },
        ],
        tolerance_hint: Some(
            "approval verification currently checks allowance change, not exact allowance target"
                .to_owned(),
        ),
    }
}

fn native_transfer_graph(
    chain_scope: &str,
    rpc_url: &str,
    recipient: Address,
    amount_atomic: U256,
) -> ActionGraph {
    let connection = EvmConnectionSpec {
        rpc_url: rpc_url.to_owned(),
    };

    let observe_node_id = "observe.native_transfer.recipient_balance".to_owned();
    let simulate_node_id = "simulate.native_transfer.call".to_owned();
    let actuate_node_id = "actuate.native_transfer.send".to_owned();
    let verify_node_id = "verify.native_transfer.effect".to_owned();

    ActionGraph {
        graph_id: Some("graph.native_transfer".to_owned()),
        roots: vec![observe_node_id.clone()],
        terminals: vec![verify_node_id.clone()],
        nodes: BTreeMap::from([
            (
                observe_node_id.clone(),
                ActionNode {
                    node_id: observe_node_id.clone(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.transfer.recipient".to_owned(),
                        "evidence.transfer.amount".to_owned(),
                    ],
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::WalletState,
                        source_hint: "capture recipient native balance before transfer".to_owned(),
                        output_key: Some("state.pre.recipient_balance".to_owned()),
                        live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmObserveBinding::NativeBalance,
                            request: EvmObserveRequest::NativeBalance { address: recipient },
                        })),
                    }),
                    implementation_hint: Some("owliabot.native_transfer".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                simulate_node_id.clone(),
                ActionNode {
                    node_id: simulate_node_id.clone(),
                    kind: ActionNodeKind::Simulate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![observe_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.transfer.recipient".to_owned(),
                        "evidence.transfer.amount".to_owned(),
                    ],
                    payload: ActionPayload::Simulate(SimulateAction {
                        simulate_kind: SimulateKind::Call,
                        simulator_hint: "simulate native value transfer".to_owned(),
                        live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: None,
                                to: recipient,
                                data: Bytes::default(),
                                value: Some(amount_atomic),
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.native_transfer".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                actuate_node_id.clone(),
                ActionNode {
                    node_id: actuate_node_id.clone(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![simulate_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.transfer.recipient".to_owned(),
                        "evidence.transfer.amount".to_owned(),
                    ],
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: format!(
                            "submit native transfer on {chain_scope} to {recipient:#x} for {amount_atomic} wei"
                        ),
                        chain: Some(chain_scope.to_owned()),
                        envelope_ref: None,
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("owliabot.native_transfer".to_owned()),
                    expected_effect_ref: Some("effect.native_transfer".to_owned()),
                },
            ),
            (
                verify_node_id.clone(),
                ActionNode {
                    node_id: verify_node_id.clone(),
                    kind: ActionNodeKind::Verify,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![actuate_node_id],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Verify(VerifyAction {
                        verify_kind: VerifyKind::EffectContract,
                        verifier_hint: "verify recipient native balance delta after submitted transfer"
                            .to_owned(),
                        pre_observation_ref: Some("state.pre.recipient_balance".to_owned()),
                        post_observation_ref: Some("state.post.recipient_balance".to_owned()),
                        live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                            connection: Some(connection),
                            binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                            post_request: Some(EvmObserveRequest::NativeBalance {
                                address: recipient,
                            }),
                        })),
                    }),
                    implementation_hint: Some("owliabot.native_transfer".to_owned()),
                    expected_effect_ref: Some("effect.native_transfer".to_owned()),
                },
            ),
        ]),
    }
}

fn native_transfer_evidence_graph(evidence: &TransferEvidencePackage) -> EvidenceGraph {
    let now_ms = current_time_ms();
    let recipient_id = "evidence.transfer.recipient".to_owned();
    let amount_id = "evidence.transfer.amount".to_owned();

    EvidenceGraph {
        records: BTreeMap::from([
            (
                recipient_id.clone(),
                evidence_record(
                    &recipient_id,
                    EvidenceKind::Fact,
                    "owliabot.transfer.recipient",
                    &serde_json::json!({
                        "user_input": evidence.recipient.user_input,
                        "normalized_address": evidence.recipient.normalized_address,
                        "user_confirmed": evidence.recipient.user_confirmed,
                        "source": evidence.recipient.source,
                    }),
                    now_ms,
                ),
            ),
            (
                amount_id.clone(),
                evidence_record(
                    &amount_id,
                    EvidenceKind::Fact,
                    "owliabot.transfer.amount",
                    &serde_json::json!({
                        "user_input": evidence.amount.user_input,
                        "normalized_amount": evidence.amount.normalized_amount,
                        "atomic_amount": evidence.amount.atomic_amount,
                        "decimals": evidence.amount.decimals,
                        "user_confirmed": evidence.amount.user_confirmed,
                        "source": evidence.amount.source,
                    }),
                    now_ms,
                ),
            ),
        ]),
        requirements: vec![
            EvidenceRequirement {
                requirement_id: recipient_id.clone(),
                reference: recipient_id.clone(),
                reason: "recipient must be normalized before transfer execution".to_owned(),
                required_by_node_id: Some("actuate.native_transfer.send".to_owned()),
                satisfied_by_evidence_id: Some(recipient_id),
            },
            EvidenceRequirement {
                requirement_id: amount_id.clone(),
                reference: amount_id.clone(),
                reason: "amount must be normalized before transfer execution".to_owned(),
                required_by_node_id: Some("actuate.native_transfer.send".to_owned()),
                satisfied_by_evidence_id: Some(amount_id),
            },
        ],
        usages: Vec::new(),
    }
}

fn erc20_transfer_graph(
    chain_scope: &str,
    rpc_url: &str,
    token: Address,
    recipient: Address,
    sender: Option<Address>,
    amount_atomic: U256,
) -> ActionGraph {
    let connection = EvmConnectionSpec {
        rpc_url: rpc_url.to_owned(),
    };

    let observe_node_id = "observe.erc20_transfer.recipient_token_balance".to_owned();
    let simulate_node_id = "simulate.erc20_transfer.call".to_owned();
    let actuate_node_id = "actuate.erc20_transfer.send".to_owned();
    let verify_node_id = "verify.erc20_transfer.effect".to_owned();

    ActionGraph {
        graph_id: Some("graph.erc20_transfer".to_owned()),
        roots: vec![observe_node_id.clone()],
        terminals: vec![verify_node_id.clone()],
        nodes: BTreeMap::from([
            (
                observe_node_id.clone(),
                ActionNode {
                    node_id: observe_node_id.clone(),
                    kind: ActionNodeKind::Observe,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: Vec::new(),
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.transfer.recipient".to_owned(),
                        "evidence.transfer.amount".to_owned(),
                        "evidence.transfer.token".to_owned(),
                    ],
                    payload: ActionPayload::Observe(ObserveAction {
                        source_kind: ObserveSourceKind::ChainRead,
                        source_hint: "capture recipient ERC20 balance before transfer".to_owned(),
                        output_key: Some("state.pre.recipient_token_balance".to_owned()),
                        live: Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmObserveBinding::Erc20BalanceOf,
                            request: EvmObserveRequest::Erc20BalanceOf {
                                token,
                                owner: recipient,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.erc20_transfer".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                simulate_node_id.clone(),
                ActionNode {
                    node_id: simulate_node_id.clone(),
                    kind: ActionNodeKind::Simulate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![observe_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.transfer.recipient".to_owned(),
                        "evidence.transfer.amount".to_owned(),
                        "evidence.transfer.token".to_owned(),
                    ],
                    payload: ActionPayload::Simulate(SimulateAction {
                        simulate_kind: SimulateKind::Call,
                        simulator_hint: "simulate ERC20 transfer(address,uint256)".to_owned(),
                        live: Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                            connection: Some(connection.clone()),
                            binding: EvmSimulateBinding::EthCall,
                            request: EvmCallRequest {
                                from: sender,
                                to: token,
                                data: encode_erc20_transfer_calldata(recipient, amount_atomic),
                                value: None,
                            },
                        })),
                    }),
                    implementation_hint: Some("owliabot.erc20_transfer".to_owned()),
                    expected_effect_ref: None,
                },
            ),
            (
                actuate_node_id.clone(),
                ActionNode {
                    node_id: actuate_node_id.clone(),
                    kind: ActionNodeKind::Actuate,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![simulate_node_id.clone()],
                    inputs: Vec::new(),
                    evidence_refs: vec![
                        "evidence.transfer.recipient".to_owned(),
                        "evidence.transfer.amount".to_owned(),
                        "evidence.transfer.token".to_owned(),
                    ],
                    payload: ActionPayload::Actuate(ActuateAction {
                        mode: ActuateMode::DriverCall,
                        actuator_hint: format!(
                            "submit ERC20 transfer on {chain_scope} token {token:#x} to {recipient:#x} for {amount_atomic} units"
                        ),
                        chain: Some(chain_scope.to_owned()),
                        envelope_ref: None,
                        requires_effect_contract: true,
                        live: None,
                    }),
                    implementation_hint: Some("owliabot.erc20_transfer".to_owned()),
                    expected_effect_ref: Some("effect.erc20_transfer".to_owned()),
                },
            ),
            (
                verify_node_id.clone(),
                ActionNode {
                    node_id: verify_node_id.clone(),
                    kind: ActionNodeKind::Verify,
                    origin: ActionOrigin::DriverFragment,
                    status: ActionNodeStatus::Pending,
                    depends_on: vec![actuate_node_id],
                    inputs: Vec::new(),
                    evidence_refs: Vec::new(),
                    payload: ActionPayload::Verify(VerifyAction {
                        verify_kind: VerifyKind::EffectContract,
                        verifier_hint:
                            "verify recipient ERC20 balance delta after submitted transfer"
                                .to_owned(),
                        pre_observation_ref: Some("state.pre.recipient_token_balance".to_owned()),
                        post_observation_ref: Some("state.post.recipient_token_balance".to_owned()),
                        live: Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                            connection: Some(connection),
                            binding: EvmVerifyBinding::EffectContractFromReceiptAndPostState,
                            post_request: Some(EvmObserveRequest::Erc20BalanceOf {
                                token,
                                owner: recipient,
                            }),
                        })),
                    }),
                    implementation_hint: Some("owliabot.erc20_transfer".to_owned()),
                    expected_effect_ref: Some("effect.erc20_transfer".to_owned()),
                },
            ),
        ]),
    }
}

fn erc20_transfer_evidence_graph(evidence: &TransferEvidencePackage) -> EvidenceGraph {
    let now_ms = current_time_ms();
    let recipient_id = "evidence.transfer.recipient".to_owned();
    let amount_id = "evidence.transfer.amount".to_owned();
    let token_id = "evidence.transfer.token".to_owned();
    let sender_balance_id = "evidence.transfer.sender_balance".to_owned();
    let mut records = BTreeMap::from([
        (
            recipient_id.clone(),
            evidence_record(
                &recipient_id,
                EvidenceKind::Fact,
                "owliabot.transfer.recipient",
                &serde_json::json!({
                    "user_input": evidence.recipient.user_input,
                    "normalized_address": evidence.recipient.normalized_address,
                    "user_confirmed": evidence.recipient.user_confirmed,
                    "source": evidence.recipient.source,
                }),
                now_ms,
            ),
        ),
        (
            amount_id.clone(),
            evidence_record(
                &amount_id,
                EvidenceKind::Fact,
                "owliabot.transfer.amount",
                &serde_json::json!({
                    "user_input": evidence.amount.user_input,
                    "normalized_amount": evidence.amount.normalized_amount,
                    "atomic_amount": evidence.amount.atomic_amount,
                    "decimals": evidence.amount.decimals,
                    "user_confirmed": evidence.amount.user_confirmed,
                    "source": evidence.amount.source,
                }),
                now_ms,
            ),
        ),
    ]);

    if let Some(token) = &evidence.token {
        records.insert(
            token_id.clone(),
            evidence_record(
                &token_id,
                EvidenceKind::Fact,
                "owliabot.transfer.token",
                &serde_json::json!({
                    "token_address": token.token_address,
                    "token_symbol": token.token_symbol,
                    "decimals": token.decimals,
                    "resolution_source": token.resolution_source,
                    "user_confirmed": token.user_confirmed,
                }),
                now_ms,
            ),
        );
    }

    if let Some(sender_balance) = &evidence.sender_balance {
        records.insert(
            sender_balance_id.clone(),
            evidence_record(
                &sender_balance_id,
                EvidenceKind::ExternalObservation,
                "owliabot.transfer.sender_balance",
                &serde_json::json!({
                    "owner": sender_balance.owner,
                    "balance_atomic": sender_balance.balance_atomic,
                    "decimals": sender_balance.decimals,
                    "observed_at_ms": sender_balance.observed_at_ms,
                    "source": sender_balance.source,
                }),
                sender_balance.observed_at_ms,
            ),
        );
    }

    EvidenceGraph {
        records,
        requirements: vec![
            EvidenceRequirement {
                requirement_id: recipient_id.clone(),
                reference: recipient_id.clone(),
                reason: "recipient must be normalized before ERC20 transfer execution".to_owned(),
                required_by_node_id: Some("actuate.erc20_transfer.send".to_owned()),
                satisfied_by_evidence_id: Some(recipient_id),
            },
            EvidenceRequirement {
                requirement_id: amount_id.clone(),
                reference: amount_id.clone(),
                reason: "amount must be normalized before ERC20 transfer execution".to_owned(),
                required_by_node_id: Some("actuate.erc20_transfer.send".to_owned()),
                satisfied_by_evidence_id: Some(amount_id),
            },
            EvidenceRequirement {
                requirement_id: token_id.clone(),
                reference: token_id.clone(),
                reason: "token metadata must be resolved before ERC20 transfer execution"
                    .to_owned(),
                required_by_node_id: Some("actuate.erc20_transfer.send".to_owned()),
                satisfied_by_evidence_id: Some(token_id),
            },
        ],
        usages: Vec::new(),
    }
}

fn evidence_record(
    evidence_id: &str,
    kind: EvidenceKind,
    source: &str,
    payload: &Value,
    observed_at_ms: u64,
) -> EvidenceRecord {
    evidence_record_with_freshness(evidence_id, kind, source, payload, observed_at_ms, None)
}

fn evidence_record_with_freshness(
    evidence_id: &str,
    kind: EvidenceKind,
    source: &str,
    payload: &Value,
    observed_at_ms: u64,
    expires_at_ms: Option<u64>,
) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: evidence_id.to_owned(),
        kind,
        provenance: EvidenceProvenance {
            source: source.to_owned(),
            chain_scope: None,
            trace_hint: None,
        },
        freshness: EvidenceFreshness {
            observed_at_ms: Some(observed_at_ms),
            expires_at_ms,
            max_age_ms: None,
        },
        confidence_ppm: Some(1_000_000),
        payload: payload.clone(),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OwliabotSubmission<TPayload, TEvidence> {
    payload: TPayload,
    evidence: TEvidence,
}

fn transfer_submission<TPayload>(
    mission: &Mission,
) -> Result<OwliabotSubmission<TPayload, TransferEvidencePackage>, String>
where
    TPayload: for<'de> Deserialize<'de>,
{
    owliabot_submission(mission)
}

fn owliabot_submission<TPayload, TEvidence>(
    mission: &Mission,
) -> Result<OwliabotSubmission<TPayload, TEvidence>, String>
where
    TPayload: for<'de> Deserialize<'de>,
    TEvidence: for<'de> Deserialize<'de>,
{
    let submission = mission
        .constraints
        .get("owliabot_submission")
        .cloned()
        .ok_or_else(|| "missing mission.constraints.owliabot_submission".to_owned())?;
    serde_json::from_value(submission)
        .map_err(|error| format!("invalid owliabot submission: {error}"))
}

fn normalize_evm_chain_scope(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("transfer chain must not be empty".to_owned());
    }
    if raw.starts_with("eip155:") {
        return Ok(raw.to_owned());
    }
    if raw.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(format!("eip155:{raw}"));
    }
    Err(format!(
        "unsupported transfer chain scope `{raw}`; expected eip155:<id> or numeric chain id"
    ))
}

fn parse_address(value: &str, field: &str) -> Result<Address, String> {
    Address::from_str(value).map_err(|error| format!("invalid {field} address `{value}`: {error}"))
}

fn parse_transfer_amount_atomic(
    evidence: &TransferAmountEvidence,
    default_decimals: u8,
) -> Result<U256, String> {
    if let Some(atomic) = &evidence.atomic_amount {
        return U256::from_str_radix(atomic, 10)
            .map_err(|error| format!("invalid atomic transfer amount `{atomic}`: {error}"));
    }

    let decimals = evidence.decimals.unwrap_or(default_decimals);
    decimal_amount_to_u256(&evidence.normalized_amount, decimals)
}

fn encode_balance_of_calldata(owner: Address) -> Bytes {
    let mut encoded = Vec::with_capacity(4 + 32);
    encoded.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
    push_address_word(&mut encoded, owner);
    Bytes::from(encoded)
}

fn encode_erc20_transfer_calldata(recipient: Address, amount: U256) -> Bytes {
    let mut encoded = Vec::with_capacity(4 + 32 + 32);
    encoded.extend_from_slice(&[0xa9, 0x05, 0x9c, 0xbb]);
    push_address_word(&mut encoded, recipient);
    encoded.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(encoded)
}

fn encode_uniswap_v3_exact_input_single_calldata(
    token_in: Address,
    token_out: Address,
    fee_tier: u32,
    recipient: Address,
    deadline_unix_seconds: u64,
    amount_in: U256,
    amount_out_minimum: U256,
) -> Bytes {
    let mut encoded = Vec::with_capacity(4 + (32 * 8));
    encoded.extend_from_slice(&[0x41, 0x4b, 0xf3, 0x89]);
    push_address_word(&mut encoded, token_in);
    push_address_word(&mut encoded, token_out);
    push_uint24_word(&mut encoded, fee_tier);
    push_address_word(&mut encoded, recipient);
    encoded.extend_from_slice(&U256::from(deadline_unix_seconds).to_be_bytes::<32>());
    encoded.extend_from_slice(&amount_in.to_be_bytes::<32>());
    encoded.extend_from_slice(&amount_out_minimum.to_be_bytes::<32>());
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());

    Bytes::from(encoded)
}

fn encode_erc20_approve_calldata(spender: Address, amount: U256) -> Bytes {
    let mut encoded = Vec::with_capacity(4 + (32 * 2));
    encoded.extend_from_slice(&[0x09, 0x5e, 0xa7, 0xb3]);
    push_address_word(&mut encoded, spender);
    encoded.extend_from_slice(&amount.to_be_bytes::<32>());

    Bytes::from(encoded)
}

fn encode_uniswap_v3_mint_calldata(
    token0: Address,
    token1: Address,
    fee_tier: u32,
    tick_lower: i32,
    tick_upper: i32,
    amount0_desired: U256,
    amount1_desired: U256,
    recipient: Address,
    deadline_unix_seconds: u64,
) -> Bytes {
    let mut encoded = Vec::with_capacity(4 + (32 * 11));
    encoded.extend_from_slice(&[0x88, 0x31, 0x64, 0x56]);
    push_address_word(&mut encoded, token0);
    push_address_word(&mut encoded, token1);
    push_uint24_word(&mut encoded, fee_tier);
    push_int24_word(&mut encoded, tick_lower);
    push_int24_word(&mut encoded, tick_upper);
    encoded.extend_from_slice(&amount0_desired.to_be_bytes::<32>());
    encoded.extend_from_slice(&amount1_desired.to_be_bytes::<32>());
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
    encoded.extend_from_slice(&U256::ZERO.to_be_bytes::<32>());
    push_address_word(&mut encoded, recipient);
    encoded.extend_from_slice(&U256::from(deadline_unix_seconds).to_be_bytes::<32>());
    Bytes::from(encoded)
}

fn parse_u256_decimal(value: &str) -> Result<U256, String> {
    U256::from_str_radix(value, 10)
        .map_err(|error| format!("invalid atomic amount `{value}`: {error}"))
}

fn parse_optional_decimal_amount(value: Option<&str>, decimals: u8) -> Result<U256, String> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        return Ok(U256::ZERO);
    }
    decimal_amount_to_u256(value, decimals)
}

fn decimal_amount_to_u256(value: &str, decimals: u8) -> Result<U256, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("normalized transfer amount must not be empty".to_owned());
    }
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fractional = parts.next();
    if parts.next().is_some() {
        return Err(format!("invalid decimal amount `{value}`"));
    }
    if !whole.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!("invalid whole-number portion in `{value}`"));
    }
    let fractional = fractional.unwrap_or_default();
    if !fractional.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!("invalid fractional portion in `{value}`"));
    }
    if fractional.len() > decimals as usize {
        return Err(format!(
            "amount `{value}` has too many fractional digits for decimals={decimals}"
        ));
    }

    let mut atomic = whole.to_owned();
    atomic.push_str(fractional);
    let padding = decimals as usize - fractional.len();
    atomic.extend(std::iter::repeat_n('0', padding));
    let normalized = atomic.trim_start_matches('0');
    let normalized = if normalized.is_empty() {
        "0"
    } else {
        normalized
    };
    U256::from_str_radix(normalized, 10)
        .map_err(|error| format!("invalid transfer amount `{value}`: {error}"))
}

fn push_address_word(encoded: &mut Vec<u8>, address: Address) {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(address.as_slice());
    encoded.extend_from_slice(&word);
}

fn push_uint24_word(encoded: &mut Vec<u8>, value: u32) {
    let mut word = [0u8; 32];
    let bytes = value.to_be_bytes();
    word[29..].copy_from_slice(&bytes[1..]);
    encoded.extend_from_slice(&word);
}

fn push_int24_word(encoded: &mut Vec<u8>, value: i32) {
    let fill = if value.is_negative() { 0xff } else { 0x00 };
    let mut word = [fill; 32];
    let bytes = value.to_be_bytes();
    word[29..].copy_from_slice(&bytes[1..]);
    encoded.extend_from_slice(&word);
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ais_agent_control::ids::SignerRequestId;
    use ais_agent_core::{
        action::{ActionGraph, ActionPayload},
        checkpoint::{CheckpointSnapshot, PendingRequestsSnapshot},
        evidence::EvidenceGraph,
        mission::{Mission, MissionBudget, MissionPolicy},
        runtime::{RunLifecycleState, RunPhase, RunStatus, SignerDecision, SignerDecisionKind},
    };
    use alloy::{
        consensus::{Receipt, ReceiptEnvelope},
        primitives::{address, b256, bytes, Bytes, TxHash, U256},
        providers::ProviderBuilder,
        rpc::types::TransactionReceipt,
        transports::mock::Asserter,
    };
    use serde_json::json;

    use crate::{
        persistence::restore_active_run_from_parts,
        runtime::ActiveRun,
        stepper::{
            apply_live_evm_observe_with_provider, apply_live_evm_simulate_with_provider,
            apply_live_evm_verify_with_provider, StepOnce, StepTransitionKind,
        },
    };

    use super::{seed_action_family_checkpoint, RuntimeExecutionWiring};

    #[test]
    fn native_transfer_checkpoint_seed_builds_live_graph_and_effects() {
        let mission = sample_native_transfer_mission(json!({
            "payload": {
                "chain": "11155111",
                "recipient": "0x1111111111111111111111111111111111111111",
                "requested_amount": "0.5",
                "asset_symbol": "ETH"
            },
            "evidence": {
                "recipient": {
                    "user_input": "0x1111111111111111111111111111111111111111",
                    "normalized_address": "0x1111111111111111111111111111111111111111",
                    "source": "user",
                    "user_confirmed": true
                },
                "amount": {
                    "user_input": "0.5",
                    "normalized_amount": "0.5",
                    "atomic_amount": "500000000000000000",
                    "decimals": 18,
                    "source": "user",
                    "user_confirmed": true
                }
            }
        }));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: true,
                erc20_transfer_enabled: false,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("native transfer should seed");

        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("actuate.native_transfer.send"));
        assert!(checkpoint
            .effect_contracts
            .contains_key("effect.native_transfer"));
        assert!(checkpoint
            .evidence_graph
            .records
            .contains_key("evidence.transfer.amount"));
    }

    #[test]
    fn native_transfer_seed_requires_rpc_wiring() {
        let mission = sample_native_transfer_mission(json!({
            "payload": {
                "chain": "11155111",
                "recipient": "0x1111111111111111111111111111111111111111",
                "requested_amount": "1"
            },
            "evidence": {
                "recipient": {
                    "user_input": "0x1111111111111111111111111111111111111111",
                    "normalized_address": "0x1111111111111111111111111111111111111111",
                    "source": "user",
                    "user_confirmed": true
                },
                "amount": {
                    "user_input": "1",
                    "normalized_amount": "1",
                    "source": "user",
                    "user_confirmed": true
                }
            }
        }));
        let mut checkpoint = sample_checkpoint();

        let err = seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: None,
                solana_rpc_url: None,
                native_transfer_enabled: true,
                erc20_transfer_enabled: false,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
            },
        )
        .expect_err("missing rpc should fail");

        assert!(err.contains("evm_rpc_url"));
    }

    #[test]
    fn erc20_transfer_checkpoint_seed_builds_live_graph_and_effects() {
        let mission = sample_erc20_transfer_mission(json!({
            "payload": {
                "chain": "11155111",
                "token_address": "0x3333333333333333333333333333333333333333",
                "token_symbol": "USDC",
                "recipient": "0x1111111111111111111111111111111111111111",
                "requested_amount": "10",
                "sender_address_hint": "0x2222222222222222222222222222222222222222"
            },
            "evidence": {
                "recipient": {
                    "user_input": "0x1111111111111111111111111111111111111111",
                    "normalized_address": "0x1111111111111111111111111111111111111111",
                    "source": "wallet_transfer",
                    "user_confirmed": true
                },
                "amount": {
                    "user_input": "10",
                    "normalized_amount": "10",
                    "atomic_amount": "10000000",
                    "decimals": 6,
                    "source": "wallet_transfer",
                    "user_confirmed": true
                },
                "token": {
                    "token_address": "0x3333333333333333333333333333333333333333",
                    "token_symbol": "USDC",
                    "decimals": 6,
                    "resolution_source": "token_registry",
                    "user_confirmed": true
                }
            }
        }));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: false,
                erc20_transfer_enabled: true,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("erc20 transfer should seed");

        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("actuate.erc20_transfer.send"));
        assert!(checkpoint
            .effect_contracts
            .contains_key("effect.erc20_transfer"));
        assert!(checkpoint
            .evidence_graph
            .records
            .contains_key("evidence.transfer.token"));

        match &checkpoint
            .action_graph
            .nodes
            .get("simulate.erc20_transfer.call")
            .expect("simulate node")
            .payload
        {
            ActionPayload::Simulate(action) => {
                let live = action.live.as_ref().expect("live simulate binding");
                let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
                else {
                    panic!("expected evm simulate binding");
                };
                assert_eq!(
                    live.request.to,
                    address!("3333333333333333333333333333333333333333")
                );
                assert_eq!(live.request.data[..4], bytes!("a9059cbb")[..]);
            }
            other => panic!("unexpected simulate payload: {other:?}"),
        }
    }

    #[test]
    fn uniswap_v3_swap_checkpoint_seed_builds_approval_branch_when_requested() {
        let mission = sample_uniswap_v3_swap_mission(json!({
            "payload": {
                "chain": "11155111",
                "token_in_address": "0x3333333333333333333333333333333333333333",
                "token_in_symbol": "USDC",
                "token_out_address": "0x4444444444444444444444444444444444444444",
                "token_out_symbol": "WETH",
                "fee_tier": 3000,
                "requested_amount": "10",
                "amount_mode": "exact_in",
                "slippage_bps": 50,
                "deadline_seconds": 4102444800u64,
                "router_address": "0x5555555555555555555555555555555555555555",
                "recipient_address": "0x1111111111111111111111111111111111111111",
                "sender_address_hint": "0x2222222222222222222222222222222222222222",
                "unwrap_native_out": false
            },
            "evidence": {
                "token_in": {
                    "token_address": "0x3333333333333333333333333333333333333333",
                    "token_symbol": "USDC",
                    "decimals": 6,
                    "resolution_source": "token_registry",
                    "user_confirmed": true
                },
                "token_out": {
                    "token_address": "0x4444444444444444444444444444444444444444",
                    "token_symbol": "WETH",
                    "decimals": 18,
                    "resolution_source": "token_registry",
                    "user_confirmed": true
                },
                "quote": {
                    "source": "quoter",
                    "quoted_at_ms": 4102444800000u64,
                    "expires_at_ms": 4102444900000u64,
                    "route_summary": "USDC/WETH 0.3%",
                    "amount_in_atomic": "10000000",
                    "amount_out_atomic": "3000000000000000",
                    "min_amount_out_atomic": "2900000000000000",
                    "user_confirmed": true
                },
                "router": {
                    "router_address": "0x5555555555555555555555555555555555555555",
                    "approval_target_address": "0x5555555555555555555555555555555555555555",
                    "approval_required": true,
                    "quoter_address": "0x6666666666666666666666666666666666666666",
                    "resolution_source": "sepolia_registry",
                    "user_confirmed": true
                },
                "deadline": {
                    "deadline_unix_seconds": 4102444800u64,
                    "source": "policy",
                    "user_confirmed": true
                }
            }
        }));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: false,
                erc20_transfer_enabled: false,
                uniswap_v3_swap_enabled: true,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("approval-required uniswap swap should seed");

        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("observe.uniswap_v3_swap.allowance"));
        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("actuate.uniswap_v3_swap.approve"));
        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("verify.uniswap_v3_swap.approval"));
        assert!(checkpoint
            .effect_contracts
            .contains_key("effect.uniswap_v3_swap.approval"));

        match &checkpoint
            .action_graph
            .nodes
            .get("simulate.uniswap_v3_swap.approve_call")
            .expect("approval simulate node")
            .payload
        {
            ActionPayload::Simulate(action) => {
                let live = action.live.as_ref().expect("live simulate binding");
                let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
                else {
                    panic!("expected evm approval simulate binding");
                };
                assert_eq!(
                    live.request.to,
                    address!("3333333333333333333333333333333333333333")
                );
                assert_eq!(live.request.data[..4], bytes!("095ea7b3")[..]);
            }
            other => panic!("unexpected approval simulate payload: {other:?}"),
        }
    }

    #[test]
    fn uniswap_v3_lp_checkpoint_seed_builds_bounded_mint_graph() {
        let mission = sample_uniswap_v3_lp_mission(json!({
            "payload": {
                "chain": "11155111",
                "operation": "mint",
                "token0_address": "0x3333333333333333333333333333333333333333",
                "token0_symbol": "USDC",
                "token1_address": "0x4444444444444444444444444444444444444444",
                "token1_symbol": "WETH",
                "fee_tier": 3000,
                "desired_amount0": "10",
                "desired_amount1": "0.003",
                "tick_lower": -600,
                "tick_upper": 600,
                "position_manager_address": "0x1238536071E1c677A632429e3655c799b22cDA52",
                "deadline_seconds": 4102444800u64,
                "sender_address_hint": "0x2222222222222222222222222222222222222222"
            },
            "evidence": {
                "token0": {
                    "token_address": "0x3333333333333333333333333333333333333333",
                    "token_symbol": "USDC",
                    "decimals": 6,
                    "resolution_source": "token_registry",
                    "user_confirmed": true
                },
                "token1": {
                    "token_address": "0x4444444444444444444444444444444444444444",
                    "token_symbol": "WETH",
                    "decimals": 18,
                    "resolution_source": "token_registry",
                    "user_confirmed": true
                },
                "pool": {
                    "pool_address": "0x5555555555555555555555555555555555555555",
                    "token0_address": "0x3333333333333333333333333333333333333333",
                    "token1_address": "0x4444444444444444444444444444444444444444",
                    "fee_tier": 3000,
                    "tick_spacing": 60,
                    "slot0_tick": 0,
                    "observed_at_ms": 4102444800000u64,
                    "resolution_source": "sepolia_registry",
                    "user_confirmed": true
                },
                "deadline": {
                    "deadline_unix_seconds": 4102444800u64,
                    "source": "policy",
                    "user_confirmed": true
                }
            }
        }));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: false,
                erc20_transfer_enabled: false,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: true,
                solana_rpc_url: None,
            },
        )
        .expect("uniswap v3 lp mint should seed");

        assert!(checkpoint
            .action_graph
            .nodes
            .contains_key("actuate.uniswap_v3_lp.mint"));
        assert!(checkpoint
            .effect_contracts
            .contains_key("effect.uniswap_v3_lp"));
        assert!(checkpoint
            .evidence_graph
            .records
            .contains_key("evidence.uniswap.lp.pool"));

        match &checkpoint
            .action_graph
            .nodes
            .get("simulate.uniswap_v3_lp.mint_call")
            .expect("simulate node")
            .payload
        {
            ActionPayload::Simulate(action) => {
                let live = action.live.as_ref().expect("live simulate binding");
                let ais_agent_core::action::kinds::simulate::SimulateLiveBinding::Evm(live) = live
                else {
                    panic!("expected evm simulate binding");
                };
                assert_eq!(
                    live.request.to,
                    address!("1238536071E1c677A632429e3655c799b22cDA52")
                );
                assert_eq!(live.request.data[..4], bytes!("88316456")[..]);
            }
            other => panic!("unexpected LP simulate payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn native_transfer_seeded_runtime_executes_signer_submission_and_verify_path() {
        let tx_hash = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let mission = sample_native_transfer_mission(sample_native_transfer_submission(
            "0x9999999999999999999999999999999999999999",
        ));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: true,
                erc20_transfer_enabled: false,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("native transfer should seed");

        assert_eq!(
            checkpoint.effect_contracts["effect.native_transfer"].assertions[1].expression,
            "post.decoded_u256 != pre.decoded_u256"
        );

        let mut runtime = ActiveRun::new(mission, checkpoint);
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&U256::from(10u64));
        asserter.push_success(&Bytes::default());
        asserter.push_success(&sample_receipt(tx_hash, 1));
        asserter.push_success(&4u64);
        asserter.push_success(&U256::from(500000000000000010u64));

        let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
            .await
            .expect("observe");
        assert_eq!(observe.kind, StepTransitionKind::Observe);

        let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
            .await
            .expect("simulate");
        assert_eq!(simulate.kind, StepTransitionKind::Simulate);

        let govern = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            govern.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Govern)
        );
        assert_eq!(
            runtime.checkpoint.lifecycle.status,
            RunStatus::AwaitingSigner
        );
        assert_eq!(
            runtime
                .pending_signer_state
                .as_ref()
                .map(|state| state.summary.as_str()),
            Some(
                "submit native transfer on eip155:11155111 to 0x1111111111111111111111111111111111111111 for 500000000000000000 wei"
            )
        );

        let request_id = SignerRequestId(
            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id
                .clone()
                .expect("signer request"),
        );
        runtime
            .pending_signer_state
            .as_mut()
            .expect("pending signer")
            .apply_decision(SignerDecision {
                request_id,
                kind: SignerDecisionKind::Submitted,
                decision_at_ms: Some(1_735_000_000_000),
                tx_hash: Some(format!("{tx_hash:#x}")),
            });

        let signer = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            signer.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Signer)
        );
        assert_eq!(
            runtime.checkpoint.lifecycle.status,
            RunStatus::AwaitingConfirmation
        );
        assert_eq!(
            runtime
                .checkpoint
                .pending_requests
                .pending_confirmation_id
                .as_deref(),
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );

        let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
            .await
            .expect("verify");
        assert_eq!(verify.kind, StepTransitionKind::Verify);
        assert_eq!(
            runtime
                .checkpoint
                .action_graph
                .nodes
                .get("verify.native_transfer.effect")
                .map(|node| node.status.clone()),
            Some(ais_agent_core::action::ActionNodeStatus::Succeeded)
        );
        assert!(runtime
            .checkpoint
            .evidence_graph
            .records
            .contains_key("state.pre.recipient_balance"));
        assert!(runtime
            .checkpoint
            .evidence_graph
            .records
            .contains_key("state.post.recipient_balance"));
        assert!(runtime
            .checkpoint
            .evidence_graph
            .records
            .contains_key("effect.verify.native_transfer.effect"));

        let complete = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            complete.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Complete)
        );
        assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
    }

    #[tokio::test]
    async fn native_transfer_seeded_runtime_restores_after_signer_submitted_confirmation_cut() {
        let tx_hash = b256!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let mission = sample_native_transfer_mission(sample_native_transfer_submission(
            "0x9999999999999999999999999999999999999999",
        ));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: true,
                erc20_transfer_enabled: false,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("native transfer should seed");

        let mut runtime = ActiveRun::new(mission.clone(), checkpoint);
        let pre_asserter = Asserter::new();
        let pre_provider = ProviderBuilder::new().connect_mocked_client(pre_asserter.clone());
        pre_asserter.push_success(&U256::from(20u64));
        pre_asserter.push_success(&Bytes::default());

        apply_live_evm_observe_with_provider(&mut runtime, &pre_provider)
            .await
            .expect("observe");
        apply_live_evm_simulate_with_provider(&mut runtime, &pre_provider)
            .await
            .expect("simulate");
        StepOnce::apply(&mut runtime).await;

        let request_id = SignerRequestId(
            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id
                .clone()
                .expect("signer request"),
        );
        runtime
            .pending_signer_state
            .as_mut()
            .expect("pending signer")
            .apply_decision(SignerDecision {
                request_id,
                kind: SignerDecisionKind::Submitted,
                decision_at_ms: Some(1_735_000_000_001),
                tx_hash: Some(format!("{tx_hash:#x}")),
            });
        StepOnce::apply(&mut runtime).await;

        let mut restored = restore_active_run_from_parts(mission, runtime.checkpoint.clone(), None)
            .expect("restore after signer submission");
        assert_eq!(
            restored
                .checkpoint
                .pending_requests
                .pending_confirmation_id
                .as_deref(),
            Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );

        let receipt_asserter = Asserter::new();
        let receipt_provider =
            ProviderBuilder::new().connect_mocked_client(receipt_asserter.clone());
        receipt_asserter.push_success(&sample_receipt(tx_hash, 2));
        receipt_asserter.push_success(&5u64);
        receipt_asserter.push_success(&U256::from(500000000000000020u64));

        let verify = apply_live_evm_verify_with_provider(&mut restored, &receipt_provider)
            .await
            .expect("verify");
        assert_eq!(verify.kind, StepTransitionKind::Verify);
        assert_eq!(
            restored.checkpoint.pending_requests.pending_confirmation_id,
            None
        );

        let complete = StepOnce::apply(&mut restored).await;
        assert_eq!(
            complete.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Complete)
        );
        assert_eq!(restored.checkpoint.lifecycle.status, RunStatus::Completed);
    }

    #[tokio::test]
    async fn erc20_transfer_seeded_runtime_executes_signer_submission_and_verify_path() {
        let tx_hash = b256!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
        let mission = sample_erc20_transfer_mission(sample_erc20_transfer_submission(
            "0x9999999999999999999999999999999999999999",
        ));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: false,
                erc20_transfer_enabled: true,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("erc20 transfer should seed");

        let mut runtime = ActiveRun::new(mission, checkpoint);
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&encode_u256_return(90));
        asserter.push_success(&Bytes::default());
        asserter.push_success(&sample_receipt(tx_hash, 1));
        asserter.push_success(&4u64);
        asserter.push_success(&encode_u256_return(120));

        let observe = apply_live_evm_observe_with_provider(&mut runtime, &provider)
            .await
            .expect("observe");
        assert_eq!(observe.kind, StepTransitionKind::Observe);

        let simulate = apply_live_evm_simulate_with_provider(&mut runtime, &provider)
            .await
            .expect("simulate");
        assert_eq!(simulate.kind, StepTransitionKind::Simulate);

        let govern = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            govern.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Govern)
        );
        assert_eq!(
            runtime.checkpoint.lifecycle.status,
            RunStatus::AwaitingSigner
        );

        let request_id = SignerRequestId(
            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id
                .clone()
                .expect("signer request"),
        );
        runtime
            .pending_signer_state
            .as_mut()
            .expect("pending signer")
            .apply_decision(SignerDecision {
                request_id,
                kind: SignerDecisionKind::Submitted,
                decision_at_ms: Some(1_735_000_000_002),
                tx_hash: Some(format!("{tx_hash:#x}")),
            });

        let signer = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            signer.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Signer)
        );
        assert_eq!(
            runtime
                .checkpoint
                .pending_requests
                .pending_confirmation_id
                .as_deref(),
            Some("0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );

        let verify = apply_live_evm_verify_with_provider(&mut runtime, &provider)
            .await
            .expect("verify");
        assert_eq!(verify.kind, StepTransitionKind::Verify);
        assert_eq!(
            runtime
                .checkpoint
                .action_graph
                .nodes
                .get("verify.erc20_transfer.effect")
                .map(|node| node.status.clone()),
            Some(ais_agent_core::action::ActionNodeStatus::Succeeded)
        );
        assert!(runtime
            .checkpoint
            .evidence_graph
            .records
            .contains_key("state.pre.recipient_token_balance"));
        assert!(runtime
            .checkpoint
            .evidence_graph
            .records
            .contains_key("state.post.recipient_token_balance"));

        let complete = StepOnce::apply(&mut runtime).await;
        assert_eq!(
            complete.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Complete)
        );
        assert_eq!(runtime.checkpoint.lifecycle.status, RunStatus::Completed);
    }

    #[tokio::test]
    async fn erc20_transfer_seeded_runtime_restores_after_signer_submitted_confirmation_cut() {
        let tx_hash = b256!("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");
        let mission = sample_erc20_transfer_mission(sample_erc20_transfer_submission(
            "0x9999999999999999999999999999999999999999",
        ));
        let mut checkpoint = sample_checkpoint();

        seed_action_family_checkpoint(
            &mission,
            &mut checkpoint,
            &RuntimeExecutionWiring {
                evm_rpc_url: Some("http://127.0.0.1:8545".to_owned()),
                native_transfer_enabled: false,
                erc20_transfer_enabled: true,
                uniswap_v3_swap_enabled: false,
                uniswap_v3_lp_enabled: false,
                solana_rpc_url: None,
            },
        )
        .expect("erc20 transfer should seed");

        let mut runtime = ActiveRun::new(mission.clone(), checkpoint);
        let pre_asserter = Asserter::new();
        let pre_provider = ProviderBuilder::new().connect_mocked_client(pre_asserter.clone());
        pre_asserter.push_success(&encode_u256_return(20));
        pre_asserter.push_success(&Bytes::default());

        apply_live_evm_observe_with_provider(&mut runtime, &pre_provider)
            .await
            .expect("observe");
        apply_live_evm_simulate_with_provider(&mut runtime, &pre_provider)
            .await
            .expect("simulate");
        StepOnce::apply(&mut runtime).await;

        let request_id = SignerRequestId(
            runtime
                .checkpoint
                .pending_requests
                .pending_signer_request_id
                .clone()
                .expect("signer request"),
        );
        runtime
            .pending_signer_state
            .as_mut()
            .expect("pending signer")
            .apply_decision(SignerDecision {
                request_id,
                kind: SignerDecisionKind::Submitted,
                decision_at_ms: Some(1_735_000_000_003),
                tx_hash: Some(format!("{tx_hash:#x}")),
            });
        StepOnce::apply(&mut runtime).await;

        let mut restored = restore_active_run_from_parts(mission, runtime.checkpoint.clone(), None)
            .expect("restore after signer submission");
        assert_eq!(
            restored
                .checkpoint
                .pending_requests
                .pending_confirmation_id
                .as_deref(),
            Some("0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
        );

        let receipt_asserter = Asserter::new();
        let receipt_provider =
            ProviderBuilder::new().connect_mocked_client(receipt_asserter.clone());
        receipt_asserter.push_success(&sample_receipt(tx_hash, 2));
        receipt_asserter.push_success(&5u64);
        receipt_asserter.push_success(&encode_u256_return(120));

        let verify = apply_live_evm_verify_with_provider(&mut restored, &receipt_provider)
            .await
            .expect("verify");
        assert_eq!(verify.kind, StepTransitionKind::Verify);
        assert_eq!(
            restored.checkpoint.pending_requests.pending_confirmation_id,
            None
        );

        let complete = StepOnce::apply(&mut restored).await;
        assert_eq!(
            complete.applied_transition.as_ref().map(|step| step.kind),
            Some(StepTransitionKind::Complete)
        );
        assert_eq!(restored.checkpoint.lifecycle.status, RunStatus::Completed);
    }

    fn sample_native_transfer_submission(sender_address_hint: &str) -> serde_json::Value {
        json!({
            "payload": {
                "chain": "11155111",
                "recipient": "0x1111111111111111111111111111111111111111",
                "requested_amount": "0.5",
                "asset_symbol": "ETH",
                "sender_address_hint": sender_address_hint
            },
            "evidence": {
                "recipient": {
                    "user_input": "0x1111111111111111111111111111111111111111",
                    "normalized_address": "0x1111111111111111111111111111111111111111",
                    "source": "wallet_transfer",
                    "user_confirmed": true
                },
                "amount": {
                    "user_input": "0.5",
                    "normalized_amount": "0.5",
                    "atomic_amount": "500000000000000000",
                    "decimals": 18,
                    "source": "wallet_transfer",
                    "user_confirmed": true
                }
            }
        })
    }

    fn sample_erc20_transfer_submission(sender_address_hint: &str) -> serde_json::Value {
        json!({
            "payload": {
                "chain": "11155111",
                "token_address": "0x3333333333333333333333333333333333333333",
                "token_symbol": "USDC",
                "recipient": "0x1111111111111111111111111111111111111111",
                "requested_amount": "10",
                "sender_address_hint": sender_address_hint
            },
            "evidence": {
                "recipient": {
                    "user_input": "0x1111111111111111111111111111111111111111",
                    "normalized_address": "0x1111111111111111111111111111111111111111",
                    "source": "wallet_transfer",
                    "user_confirmed": true
                },
                "amount": {
                    "user_input": "10",
                    "normalized_amount": "10",
                    "atomic_amount": "10000000",
                    "decimals": 6,
                    "source": "wallet_transfer",
                    "user_confirmed": true
                },
                "token": {
                    "token_address": "0x3333333333333333333333333333333333333333",
                    "token_symbol": "USDC",
                    "decimals": 6,
                    "resolution_source": "token_registry",
                    "user_confirmed": true
                }
            }
        })
    }

    fn sample_uniswap_v3_swap_mission(submission: serde_json::Value) -> Mission {
        Mission {
            mission_id: "mission-uniswap-v3-swap".to_owned(),
            goal: "owliabot:uniswap_v3_swap".to_owned(),
            allowed_chains: vec!["11155111".to_owned()],
            budget: MissionBudget::default(),
            policy: MissionPolicy {
                policy_mode: Some("guarded".to_owned()),
                allow_raw_envelopes: true,
                require_effect_contract_for_writes: true,
            },
            constraints: BTreeMap::from([
                (
                    "owliabot_action_family".to_owned(),
                    json!("uniswap_v3_swap"),
                ),
                ("owliabot_submission".to_owned(), submission),
            ]),
            metadata: BTreeMap::new(),
        }
    }

    fn sample_uniswap_v3_lp_mission(submission: serde_json::Value) -> Mission {
        Mission {
            mission_id: "mission-uniswap-v3-lp".to_owned(),
            goal: "owliabot:uniswap_v3_lp".to_owned(),
            allowed_chains: vec!["11155111".to_owned()],
            budget: MissionBudget::default(),
            policy: MissionPolicy {
                policy_mode: Some("guarded".to_owned()),
                allow_raw_envelopes: true,
                require_effect_contract_for_writes: true,
            },
            constraints: BTreeMap::from([
                ("owliabot_action_family".to_owned(), json!("uniswap_v3_lp")),
                ("owliabot_submission".to_owned(), submission),
            ]),
            metadata: BTreeMap::new(),
        }
    }

    fn sample_native_transfer_mission(submission: serde_json::Value) -> Mission {
        Mission {
            mission_id: "mission-transfer".to_owned(),
            goal: "owliabot:native_transfer".to_owned(),
            allowed_chains: vec!["11155111".to_owned()],
            budget: MissionBudget::default(),
            policy: MissionPolicy {
                policy_mode: Some("guarded".to_owned()),
                allow_raw_envelopes: true,
                require_effect_contract_for_writes: true,
            },
            constraints: BTreeMap::from([
                (
                    "owliabot_action_family".to_owned(),
                    json!("native_transfer"),
                ),
                ("owliabot_submission".to_owned(), submission),
            ]),
            metadata: BTreeMap::new(),
        }
    }

    fn sample_erc20_transfer_mission(submission: serde_json::Value) -> Mission {
        Mission {
            mission_id: "mission-transfer".to_owned(),
            goal: "owliabot:erc20_transfer".to_owned(),
            allowed_chains: vec!["11155111".to_owned()],
            budget: MissionBudget::default(),
            policy: MissionPolicy {
                policy_mode: Some("guarded".to_owned()),
                allow_raw_envelopes: true,
                require_effect_contract_for_writes: true,
            },
            constraints: BTreeMap::from([
                ("owliabot_action_family".to_owned(), json!("erc20_transfer")),
                ("owliabot_submission".to_owned(), submission),
            ]),
            metadata: BTreeMap::new(),
        }
    }

    fn sample_checkpoint() -> CheckpointSnapshot {
        let mut lifecycle = RunLifecycleState::new(
            ais_agent_control::ids::RunId("run-1".to_owned()),
            "mission-transfer".to_owned(),
        );
        lifecycle.mark_running(RunPhase::MissionAccepted);
        CheckpointSnapshot {
            run_id: "run-1".to_owned(),
            mission_id: "mission-transfer".to_owned(),
            checkpoint_seq: 0,
            plan_epoch: 0,
            lifecycle,
            action_graph: ActionGraph::default(),
            evidence_graph: EvidenceGraph::default(),
            effect_contracts: BTreeMap::new(),
            pending_requests: PendingRequestsSnapshot::default(),
            last_completed_node_id: None,
            actuation_records: Vec::new(),
        }
    }

    fn sample_receipt(tx_hash: TxHash, block_number: u64) -> Option<TransactionReceipt> {
        Some(TransactionReceipt {
            inner: ReceiptEnvelope::Eip1559(
                Receipt {
                    status: true.into(),
                    cumulative_gas_used: 21_000,
                    logs: Vec::new(),
                }
                .with_bloom(),
            ),
            transaction_hash: tx_hash,
            transaction_index: Some(0),
            block_hash: Some(b256!(
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            )),
            block_number: Some(block_number),
            gas_used: 21_000,
            effective_gas_price: 1,
            blob_gas_used: None,
            blob_gas_price: None,
            from: address!("1111111111111111111111111111111111111111"),
            to: Some(address!("2222222222222222222222222222222222222222")),
            contract_address: None,
        })
    }

    fn encode_u256_return(value: u64) -> Bytes {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&value.to_be_bytes());
        Bytes::from(word.to_vec())
    }
}
