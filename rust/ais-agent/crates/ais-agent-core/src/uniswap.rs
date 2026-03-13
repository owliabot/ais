use ais_agent_control::uniswap::{
    UniswapLpOperationKind, UniswapV3LpRequest, UniswapV3LpVerificationContract,
    UniswapV3SwapRequest, UniswapV3SwapVerificationContract,
};
use serde::{Deserialize, Serialize};

use crate::effect::{EffectAssertion, EffectContract, EffectContractKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum UniswapV3Request {
    Swap(UniswapV3SwapRequest),
    Lp(UniswapV3LpRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum UniswapV3EffectTemplate {
    Swap(UniswapV3SwapEffectTemplate),
    Lp(UniswapV3LpEffectTemplate),
}

impl UniswapV3EffectTemplate {
    pub fn to_effect_contract(&self, effect_id: impl Into<String>) -> EffectContract {
        let effect_id = effect_id.into();
        match self {
            Self::Swap(_template) => EffectContract {
                effect_id,
                kind: EffectContractKind::AssetDelta,
                assertions: vec![
                    EffectAssertion {
                        expression: "receipt.status == true".to_owned(),
                        description: "Uniswap V3 swap receipt must succeed".to_owned(),
                    },
                    EffectAssertion {
                        expression: "post.decoded_u256 != pre.decoded_u256".to_owned(),
                        description: "swap recipient output balance should change".to_owned(),
                    },
                ],
                tolerance_hint: Some(
                    "swap verification should later compare quote/slippage bounds and exact output deltas"
                        .to_owned(),
                ),
            },
            Self::Lp(template) => EffectContract {
                effect_id,
                kind: EffectContractKind::StateTransition,
                assertions: vec![
                    EffectAssertion {
                        expression: "receipt.status == true".to_owned(),
                        description: "Uniswap V3 LP transaction receipt must succeed".to_owned(),
                    },
                    EffectAssertion {
                        expression: lp_state_transition_expression(template.verification.operation.clone()),
                        description: "position state should transition after the LP operation"
                            .to_owned(),
                    },
                ],
                tolerance_hint: Some(
                    "lp verification should later compare structured position state, liquidity deltas, and collected amounts"
                        .to_owned(),
                ),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3SwapEffectTemplate {
    pub request: UniswapV3SwapRequest,
    pub verification: UniswapV3SwapVerificationContract,
}

impl UniswapV3SwapEffectTemplate {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        self.verification.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3LpEffectTemplate {
    pub request: UniswapV3LpRequest,
    pub verification: UniswapV3LpVerificationContract,
}

impl UniswapV3LpEffectTemplate {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        self.verification.validate()
    }
}

fn lp_state_transition_expression(operation: UniswapLpOperationKind) -> String {
    match operation {
        UniswapLpOperationKind::Mint => "post.decoded_u256 != pre.decoded_u256".to_owned(),
        UniswapLpOperationKind::IncreaseLiquidity
        | UniswapLpOperationKind::DecreaseLiquidity
        | UniswapLpOperationKind::ClosePosition => {
            "post_position.liquidity != pre_position.liquidity".to_owned()
        }
        UniswapLpOperationKind::Collect => {
            "post_position.tokens_owed != pre_position.tokens_owed".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use ais_agent_control::uniswap::{
        UniswapLpOperationKind, UniswapSwapAmountMode, UniswapV3LpRequest,
        UniswapV3LpVerificationContract, UniswapV3SwapRequest, UniswapV3SwapVerificationContract,
    };

    use super::{UniswapV3EffectTemplate, UniswapV3LpEffectTemplate, UniswapV3SwapEffectTemplate};

    #[test]
    fn uniswap_swap_effect_contract_contains_output_balance_assertion() {
        let template = UniswapV3EffectTemplate::Swap(UniswapV3SwapEffectTemplate {
            request: UniswapV3SwapRequest {
                chain: "11155111".to_owned(),
                token_in_address: "0x1111".to_owned(),
                token_in_symbol: Some("WETH".to_owned()),
                token_out_address: "0x2222".to_owned(),
                token_out_symbol: Some("USDC".to_owned()),
                fee_tier: 3000,
                requested_amount: "0.5".to_owned(),
                amount_mode: UniswapSwapAmountMode::ExactIn,
                slippage_bps: 50,
                deadline_seconds: 900,
                router_address: "0xe592".to_owned(),
                recipient_address: Some("0xaaaa".to_owned()),
                sender_address_hint: Some("0xbbbb".to_owned()),
                unwrap_native_out: false,
            },
            verification: UniswapV3SwapVerificationContract {
                chain: "11155111".to_owned(),
                token_in_address: "0x1111".to_owned(),
                token_out_address: "0x2222".to_owned(),
                fee_tier: 3000,
                recipient_address: "0xaaaa".to_owned(),
                amount_mode: UniswapSwapAmountMode::ExactIn,
                quoted_amount_in_atomic: Some("500000000000000000".to_owned()),
                quoted_amount_out_atomic: Some("1500000000".to_owned()),
                min_amount_out_atomic: Some("1490000000".to_owned()),
                max_amount_in_atomic: None,
                router_address: "0xe592".to_owned(),
                deadline_unix_seconds: 1_700_000_000,
                sender_address_hint: Some("0xbbbb".to_owned()),
                require_recipient_out_delta: true,
            },
        });

        let effect = template.to_effect_contract("effect.swap.uniswap_v3");
        assert_eq!(effect.kind, crate::effect::EffectContractKind::AssetDelta);
        assert!(
            effect.assertions[1]
                .expression
                .contains("post.decoded_u256 != pre.decoded_u256"),
            "swap effect should describe output-balance change"
        );
    }

    #[test]
    fn uniswap_lp_effect_contract_uses_state_transition_kind() {
        let template = UniswapV3EffectTemplate::Lp(UniswapV3LpEffectTemplate {
            request: UniswapV3LpRequest {
                chain: "11155111".to_owned(),
                operation: UniswapLpOperationKind::Mint,
                token0_address: "0x1111".to_owned(),
                token0_symbol: Some("USDC".to_owned()),
                token1_address: "0x2222".to_owned(),
                token1_symbol: Some("WETH".to_owned()),
                fee_tier: 3000,
                desired_amount0: Some("1000".to_owned()),
                desired_amount1: Some("0.5".to_owned()),
                tick_lower: Some(-600),
                tick_upper: Some(600),
                position_manager_address: "0xc364".to_owned(),
                position_token_id: None,
                deadline_seconds: Some(900),
                sender_address_hint: Some("0xaaaa".to_owned()),
            },
            verification: UniswapV3LpVerificationContract {
                chain: "11155111".to_owned(),
                operation: UniswapLpOperationKind::Mint,
                position_manager_address: "0xc364".to_owned(),
                pool_address: "0xpool".to_owned(),
                token0_address: "0x1111".to_owned(),
                token1_address: "0x2222".to_owned(),
                fee_tier: 3000,
                position_token_id: None,
                expected_liquidity_delta: Some("100".to_owned()),
                expected_amount0_max: Some("1000000000".to_owned()),
                expected_amount1_max: Some("500000000000000000".to_owned()),
                tick_lower: Some(-600),
                tick_upper: Some(600),
            },
        });

        let effect = template.to_effect_contract("effect.lp.uniswap_v3");
        assert_eq!(
            effect.kind,
            crate::effect::EffectContractKind::StateTransition
        );
        assert!(
            effect.assertions[1]
                .expression
                .contains("post.decoded_u256 != pre.decoded_u256"),
            "lp mint effect should describe position-count change"
        );
    }
}
