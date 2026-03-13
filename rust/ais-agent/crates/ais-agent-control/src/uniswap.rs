use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniswapSwapAmountMode {
    ExactIn,
    ExactOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniswapLpOperationKind {
    Mint,
    IncreaseLiquidity,
    DecreaseLiquidity,
    Collect,
    ClosePosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3SwapRequest {
    pub chain: String,
    pub token_in_address: String,
    #[serde(default)]
    pub token_in_symbol: Option<String>,
    pub token_out_address: String,
    #[serde(default)]
    pub token_out_symbol: Option<String>,
    pub fee_tier: u32,
    pub requested_amount: String,
    pub amount_mode: UniswapSwapAmountMode,
    pub slippage_bps: u16,
    pub deadline_seconds: u64,
    pub router_address: String,
    #[serde(default)]
    pub recipient_address: Option<String>,
    #[serde(default)]
    pub sender_address_hint: Option<String>,
    #[serde(default)]
    pub unwrap_native_out: bool,
}

impl UniswapV3SwapRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("uniswap_v3_swap_request.chain must not be empty");
        }
        if self.token_in_address.trim().is_empty() {
            return Err("uniswap_v3_swap_request.token_in_address must not be empty");
        }
        if self.token_out_address.trim().is_empty() {
            return Err("uniswap_v3_swap_request.token_out_address must not be empty");
        }
        if self
            .token_in_address
            .eq_ignore_ascii_case(&self.token_out_address)
        {
            return Err(
                "uniswap_v3_swap_request.token_in_address must differ from token_out_address",
            );
        }
        if self.fee_tier == 0 {
            return Err("uniswap_v3_swap_request.fee_tier must be greater than zero");
        }
        if self.requested_amount.trim().is_empty() {
            return Err("uniswap_v3_swap_request.requested_amount must not be empty");
        }
        if self.router_address.trim().is_empty() {
            return Err("uniswap_v3_swap_request.router_address must not be empty");
        }
        if self.deadline_seconds == 0 {
            return Err("uniswap_v3_swap_request.deadline_seconds must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3LpRequest {
    pub chain: String,
    pub operation: UniswapLpOperationKind,
    pub token0_address: String,
    #[serde(default)]
    pub token0_symbol: Option<String>,
    pub token1_address: String,
    #[serde(default)]
    pub token1_symbol: Option<String>,
    pub fee_tier: u32,
    #[serde(default)]
    pub desired_amount0: Option<String>,
    #[serde(default)]
    pub desired_amount1: Option<String>,
    #[serde(default)]
    pub tick_lower: Option<i32>,
    #[serde(default)]
    pub tick_upper: Option<i32>,
    pub position_manager_address: String,
    #[serde(default)]
    pub position_token_id: Option<String>,
    #[serde(default)]
    pub deadline_seconds: Option<u64>,
    #[serde(default)]
    pub sender_address_hint: Option<String>,
}

impl UniswapV3LpRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("uniswap_v3_lp_request.chain must not be empty");
        }
        if self.token0_address.trim().is_empty() {
            return Err("uniswap_v3_lp_request.token0_address must not be empty");
        }
        if self.token1_address.trim().is_empty() {
            return Err("uniswap_v3_lp_request.token1_address must not be empty");
        }
        if self
            .token0_address
            .eq_ignore_ascii_case(&self.token1_address)
        {
            return Err("uniswap_v3_lp_request.token0_address must differ from token1_address");
        }
        if self.fee_tier == 0 {
            return Err("uniswap_v3_lp_request.fee_tier must be greater than zero");
        }
        if self.position_manager_address.trim().is_empty() {
            return Err("uniswap_v3_lp_request.position_manager_address must not be empty");
        }
        if let (Some(tick_lower), Some(tick_upper)) = (self.tick_lower, self.tick_upper) {
            if tick_lower >= tick_upper {
                return Err("uniswap_v3_lp_request.tick_lower must be less than tick_upper");
            }
        }
        if let Some(deadline_seconds) = self.deadline_seconds {
            if deadline_seconds == 0 {
                return Err("uniswap_v3_lp_request.deadline_seconds must be greater than zero");
            }
        }

        match self.operation {
            UniswapLpOperationKind::Mint => {
                if self.tick_lower.is_none() || self.tick_upper.is_none() {
                    return Err("uniswap_v3_lp_request.mint requires tick_lower and tick_upper");
                }
                if self
                    .desired_amount0
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self
                        .desired_amount1
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(
                        "uniswap_v3_lp_request.mint requires desired_amount0 or desired_amount1",
                    );
                }
            }
            UniswapLpOperationKind::IncreaseLiquidity => {
                if self
                    .position_token_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err(
                        "uniswap_v3_lp_request.increase_liquidity requires position_token_id",
                    );
                }
                if self
                    .desired_amount0
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self
                        .desired_amount1
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(
                        "uniswap_v3_lp_request.increase_liquidity requires desired_amount0 or desired_amount1",
                    );
                }
            }
            UniswapLpOperationKind::DecreaseLiquidity
            | UniswapLpOperationKind::Collect
            | UniswapLpOperationKind::ClosePosition => {
                if self
                    .position_token_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err(
                        "uniswap_v3_lp_request.position operation requires position_token_id",
                    );
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapTokenEvidence {
    pub token_address: String,
    #[serde(default)]
    pub token_symbol: Option<String>,
    pub decimals: u8,
    pub resolution_source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapQuoteEvidence {
    pub source: String,
    pub quoted_at_ms: u64,
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
    #[serde(default)]
    pub route_summary: Option<String>,
    #[serde(default)]
    pub amount_in_atomic: Option<String>,
    #[serde(default)]
    pub amount_out_atomic: Option<String>,
    #[serde(default)]
    pub min_amount_out_atomic: Option<String>,
    #[serde(default)]
    pub max_amount_in_atomic: Option<String>,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapRouterEvidence {
    pub router_address: String,
    #[serde(default)]
    pub approval_target_address: Option<String>,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub quoter_address: Option<String>,
    pub resolution_source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapPoolEvidence {
    pub pool_address: String,
    pub token0_address: String,
    pub token1_address: String,
    pub fee_tier: u32,
    #[serde(default)]
    pub tick_spacing: Option<i32>,
    #[serde(default)]
    pub slot0_sqrt_price_x96: Option<String>,
    #[serde(default)]
    pub slot0_tick: Option<i32>,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
    pub resolution_source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapDeadlineEvidence {
    pub deadline_unix_seconds: u64,
    pub source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapLpPositionEvidence {
    pub position_token_id: String,
    #[serde(default)]
    pub liquidity: Option<String>,
    #[serde(default)]
    pub tick_lower: Option<i32>,
    #[serde(default)]
    pub tick_upper: Option<i32>,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3SwapEvidencePackage {
    pub token_in: UniswapTokenEvidence,
    pub token_out: UniswapTokenEvidence,
    pub quote: UniswapQuoteEvidence,
    pub router: UniswapRouterEvidence,
    pub deadline: UniswapDeadlineEvidence,
}

impl UniswapV3SwapEvidencePackage {
    pub fn validate_for(&self, request: &UniswapV3SwapRequest) -> Result<(), &'static str> {
        validate_token_evidence(&self.token_in, "uniswap_v3_swap_evidence.token_in")?;
        validate_token_evidence(&self.token_out, "uniswap_v3_swap_evidence.token_out")?;
        if self
            .token_in
            .token_address
            .eq_ignore_ascii_case(&self.token_out.token_address)
        {
            return Err("uniswap_v3_swap_evidence.token_in must differ from token_out");
        }
        if self.quote.source.trim().is_empty() {
            return Err("uniswap_v3_swap_evidence.quote.source must not be empty");
        }
        if self.router.router_address.trim().is_empty() {
            return Err("uniswap_v3_swap_evidence.router.router_address must not be empty");
        }
        if self.router.resolution_source.trim().is_empty() {
            return Err("uniswap_v3_swap_evidence.router.resolution_source must not be empty");
        }
        if self.router.approval_required
            && self
                .router
                .approval_target_address
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(
                "uniswap_v3_swap_evidence.router.approval_target_address must be present when approval_required is true",
            );
        }
        if self.deadline.deadline_unix_seconds == 0 {
            return Err(
                "uniswap_v3_swap_evidence.deadline.deadline_unix_seconds must be greater than zero",
            );
        }
        if self.deadline.source.trim().is_empty() {
            return Err("uniswap_v3_swap_evidence.deadline.source must not be empty");
        }

        match request.amount_mode {
            UniswapSwapAmountMode::ExactIn => {
                if self
                    .quote
                    .min_amount_out_atomic
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self
                        .quote
                        .amount_out_atomic
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(
                        "uniswap_v3_swap exact_in requires quote.min_amount_out_atomic or amount_out_atomic",
                    );
                }
            }
            UniswapSwapAmountMode::ExactOut => {
                if self
                    .quote
                    .max_amount_in_atomic
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self
                        .quote
                        .amount_in_atomic
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(
                        "uniswap_v3_swap exact_out requires quote.max_amount_in_atomic or amount_in_atomic",
                    );
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3LpEvidencePackage {
    pub token0: UniswapTokenEvidence,
    pub token1: UniswapTokenEvidence,
    pub pool: UniswapPoolEvidence,
    #[serde(default)]
    pub router: Option<UniswapRouterEvidence>,
    #[serde(default)]
    pub deadline: Option<UniswapDeadlineEvidence>,
    #[serde(default)]
    pub position: Option<UniswapLpPositionEvidence>,
}

impl UniswapV3LpEvidencePackage {
    pub fn validate_for(&self, request: &UniswapV3LpRequest) -> Result<(), &'static str> {
        validate_token_evidence(&self.token0, "uniswap_v3_lp_evidence.token0")?;
        validate_token_evidence(&self.token1, "uniswap_v3_lp_evidence.token1")?;
        if self
            .token0
            .token_address
            .eq_ignore_ascii_case(&self.token1.token_address)
        {
            return Err("uniswap_v3_lp_evidence.token0 must differ from token1");
        }
        if self.pool.pool_address.trim().is_empty() {
            return Err("uniswap_v3_lp_evidence.pool.pool_address must not be empty");
        }
        if self.pool.token0_address.trim().is_empty() || self.pool.token1_address.trim().is_empty()
        {
            return Err("uniswap_v3_lp_evidence.pool token addresses must not be empty");
        }
        if self.pool.fee_tier == 0 {
            return Err("uniswap_v3_lp_evidence.pool.fee_tier must be greater than zero");
        }
        if self.pool.resolution_source.trim().is_empty() {
            return Err("uniswap_v3_lp_evidence.pool.resolution_source must not be empty");
        }
        if let Some(deadline) = &self.deadline {
            if deadline.deadline_unix_seconds == 0 {
                return Err(
                    "uniswap_v3_lp_evidence.deadline.deadline_unix_seconds must be greater than zero",
                );
            }
        }

        match request.operation {
            UniswapLpOperationKind::Mint => {}
            UniswapLpOperationKind::IncreaseLiquidity
            | UniswapLpOperationKind::DecreaseLiquidity
            | UniswapLpOperationKind::Collect
            | UniswapLpOperationKind::ClosePosition => {
                let position = self.position.as_ref().ok_or(
                    "uniswap_v3_lp position operations require uniswap_v3_lp_evidence.position",
                )?;
                if position.position_token_id.trim().is_empty() {
                    return Err(
                        "uniswap_v3_lp_evidence.position.position_token_id must not be empty",
                    );
                }
                if position.source.trim().is_empty() {
                    return Err("uniswap_v3_lp_evidence.position.source must not be empty");
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3SwapVerificationContract {
    pub chain: String,
    pub token_in_address: String,
    pub token_out_address: String,
    pub fee_tier: u32,
    pub recipient_address: String,
    pub amount_mode: UniswapSwapAmountMode,
    #[serde(default)]
    pub quoted_amount_in_atomic: Option<String>,
    #[serde(default)]
    pub quoted_amount_out_atomic: Option<String>,
    #[serde(default)]
    pub min_amount_out_atomic: Option<String>,
    #[serde(default)]
    pub max_amount_in_atomic: Option<String>,
    pub router_address: String,
    pub deadline_unix_seconds: u64,
    #[serde(default)]
    pub sender_address_hint: Option<String>,
    #[serde(default)]
    pub require_recipient_out_delta: bool,
}

impl UniswapV3SwapVerificationContract {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("uniswap_v3_swap_verification.chain must not be empty");
        }
        if self.token_in_address.trim().is_empty() {
            return Err("uniswap_v3_swap_verification.token_in_address must not be empty");
        }
        if self.token_out_address.trim().is_empty() {
            return Err("uniswap_v3_swap_verification.token_out_address must not be empty");
        }
        if self
            .token_in_address
            .eq_ignore_ascii_case(&self.token_out_address)
        {
            return Err(
                "uniswap_v3_swap_verification.token_in_address must differ from token_out_address",
            );
        }
        if self.fee_tier == 0 {
            return Err("uniswap_v3_swap_verification.fee_tier must be greater than zero");
        }
        if self.recipient_address.trim().is_empty() {
            return Err("uniswap_v3_swap_verification.recipient_address must not be empty");
        }
        if self.router_address.trim().is_empty() {
            return Err("uniswap_v3_swap_verification.router_address must not be empty");
        }
        if self.deadline_unix_seconds == 0 {
            return Err(
                "uniswap_v3_swap_verification.deadline_unix_seconds must be greater than zero",
            );
        }
        match self.amount_mode {
            UniswapSwapAmountMode::ExactIn => {
                if self
                    .min_amount_out_atomic
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self
                        .quoted_amount_out_atomic
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(
                        "uniswap_v3_swap_verification.exact_in requires min_amount_out_atomic or quoted_amount_out_atomic",
                    );
                }
            }
            UniswapSwapAmountMode::ExactOut => {
                if self
                    .max_amount_in_atomic
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && self
                        .quoted_amount_in_atomic
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(
                        "uniswap_v3_swap_verification.exact_out requires max_amount_in_atomic or quoted_amount_in_atomic",
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniswapV3LpVerificationContract {
    pub chain: String,
    pub operation: UniswapLpOperationKind,
    pub position_manager_address: String,
    pub pool_address: String,
    pub token0_address: String,
    pub token1_address: String,
    pub fee_tier: u32,
    #[serde(default)]
    pub position_token_id: Option<String>,
    #[serde(default)]
    pub expected_liquidity_delta: Option<String>,
    #[serde(default)]
    pub expected_amount0_max: Option<String>,
    #[serde(default)]
    pub expected_amount1_max: Option<String>,
    #[serde(default)]
    pub tick_lower: Option<i32>,
    #[serde(default)]
    pub tick_upper: Option<i32>,
}

impl UniswapV3LpVerificationContract {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("uniswap_v3_lp_verification.chain must not be empty");
        }
        if self.position_manager_address.trim().is_empty() {
            return Err("uniswap_v3_lp_verification.position_manager_address must not be empty");
        }
        if self.pool_address.trim().is_empty() {
            return Err("uniswap_v3_lp_verification.pool_address must not be empty");
        }
        if self.token0_address.trim().is_empty() || self.token1_address.trim().is_empty() {
            return Err("uniswap_v3_lp_verification token addresses must not be empty");
        }
        if self
            .token0_address
            .eq_ignore_ascii_case(&self.token1_address)
        {
            return Err(
                "uniswap_v3_lp_verification.token0_address must differ from token1_address",
            );
        }
        if self.fee_tier == 0 {
            return Err("uniswap_v3_lp_verification.fee_tier must be greater than zero");
        }
        if let (Some(tick_lower), Some(tick_upper)) = (self.tick_lower, self.tick_upper) {
            if tick_lower >= tick_upper {
                return Err("uniswap_v3_lp_verification.tick_lower must be less than tick_upper");
            }
        }
        match self.operation {
            UniswapLpOperationKind::Mint => {
                if self.tick_lower.is_none() || self.tick_upper.is_none() {
                    return Err(
                        "uniswap_v3_lp_verification.mint requires tick_lower and tick_upper",
                    );
                }
            }
            UniswapLpOperationKind::IncreaseLiquidity
            | UniswapLpOperationKind::DecreaseLiquidity
            | UniswapLpOperationKind::Collect
            | UniswapLpOperationKind::ClosePosition => {
                if self
                    .position_token_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err(
                        "uniswap_v3_lp_verification.position operation requires position_token_id",
                    );
                }
            }
        }
        Ok(())
    }
}

fn validate_token_evidence(
    evidence: &UniswapTokenEvidence,
    field_prefix: &str,
) -> Result<(), &'static str> {
    if evidence.token_address.trim().is_empty() {
        return Err("uniswap token evidence token_address must not be empty");
    }
    if evidence.resolution_source.trim().is_empty() {
        return Err("uniswap token evidence resolution_source must not be empty");
    }
    let _ = field_prefix;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        UniswapDeadlineEvidence, UniswapLpOperationKind, UniswapPoolEvidence, UniswapQuoteEvidence,
        UniswapRouterEvidence, UniswapSwapAmountMode, UniswapTokenEvidence,
        UniswapV3LpEvidencePackage, UniswapV3LpRequest, UniswapV3LpVerificationContract,
        UniswapV3SwapEvidencePackage, UniswapV3SwapRequest, UniswapV3SwapVerificationContract,
    };

    #[test]
    fn uniswap_swap_request_and_evidence_validate() {
        let request = UniswapV3SwapRequest {
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
        };
        request.validate().expect("swap request should validate");

        UniswapV3SwapEvidencePackage {
            token_in: UniswapTokenEvidence {
                token_address: "0x1111".to_owned(),
                token_symbol: Some("WETH".to_owned()),
                decimals: 18,
                resolution_source: "wallet".to_owned(),
                user_confirmed: true,
            },
            token_out: UniswapTokenEvidence {
                token_address: "0x2222".to_owned(),
                token_symbol: Some("USDC".to_owned()),
                decimals: 6,
                resolution_source: "wallet".to_owned(),
                user_confirmed: true,
            },
            quote: UniswapQuoteEvidence {
                source: "quoter".to_owned(),
                quoted_at_ms: 1,
                expires_at_ms: Some(2),
                route_summary: Some("WETH->USDC".to_owned()),
                amount_in_atomic: Some("500000000000000000".to_owned()),
                amount_out_atomic: Some("1500000000".to_owned()),
                min_amount_out_atomic: Some("1490000000".to_owned()),
                max_amount_in_atomic: None,
                user_confirmed: true,
            },
            router: UniswapRouterEvidence {
                router_address: "0xe592".to_owned(),
                approval_target_address: Some("0xe592".to_owned()),
                approval_required: false,
                quoter_address: Some("0xb273".to_owned()),
                resolution_source: "sepolia_registry".to_owned(),
                user_confirmed: true,
            },
            deadline: UniswapDeadlineEvidence {
                deadline_unix_seconds: 1_700_000_000,
                source: "policy".to_owned(),
                user_confirmed: true,
            },
        }
        .validate_for(&request)
        .expect("swap evidence should validate");
    }

    #[test]
    fn uniswap_swap_verification_requires_outbound_bound() {
        let err = UniswapV3SwapVerificationContract {
            chain: "11155111".to_owned(),
            token_in_address: "0x1111".to_owned(),
            token_out_address: "0x2222".to_owned(),
            fee_tier: 3000,
            recipient_address: "0xaaaa".to_owned(),
            amount_mode: UniswapSwapAmountMode::ExactIn,
            quoted_amount_in_atomic: Some("1".to_owned()),
            quoted_amount_out_atomic: None,
            min_amount_out_atomic: None,
            max_amount_in_atomic: None,
            router_address: "0xe592".to_owned(),
            deadline_unix_seconds: 1_700_000_000,
            sender_address_hint: None,
            require_recipient_out_delta: true,
        }
        .validate()
        .expect_err("exact_in verification needs an out bound");

        assert!(err.contains("exact_in"));
    }

    #[test]
    fn uniswap_lp_request_and_evidence_validate() {
        let request = UniswapV3LpRequest {
            chain: "11155111".to_owned(),
            operation: UniswapLpOperationKind::Mint,
            token0_address: "0x1111".to_owned(),
            token0_symbol: Some("USDC".to_owned()),
            token1_address: "0x2222".to_owned(),
            token1_symbol: Some("WETH".to_owned()),
            fee_tier: 3000,
            desired_amount0: Some("1000".to_owned()),
            desired_amount1: Some("0.5".to_owned()),
            tick_lower: Some(-887220),
            tick_upper: Some(887220),
            position_manager_address: "0x1234".to_owned(),
            position_token_id: None,
            deadline_seconds: Some(900),
            sender_address_hint: Some("0xaaaa".to_owned()),
        };
        request.validate().expect("lp request should validate");

        UniswapV3LpEvidencePackage {
            token0: UniswapTokenEvidence {
                token_address: "0x1111".to_owned(),
                token_symbol: Some("USDC".to_owned()),
                decimals: 6,
                resolution_source: "wallet".to_owned(),
                user_confirmed: true,
            },
            token1: UniswapTokenEvidence {
                token_address: "0x2222".to_owned(),
                token_symbol: Some("WETH".to_owned()),
                decimals: 18,
                resolution_source: "wallet".to_owned(),
                user_confirmed: true,
            },
            pool: UniswapPoolEvidence {
                pool_address: "0xpool".to_owned(),
                token0_address: "0x1111".to_owned(),
                token1_address: "0x2222".to_owned(),
                fee_tier: 3000,
                tick_spacing: Some(60),
                slot0_sqrt_price_x96: Some("1".to_owned()),
                slot0_tick: Some(0),
                observed_at_ms: Some(1),
                resolution_source: "pool_lookup".to_owned(),
                user_confirmed: true,
            },
            router: Some(UniswapRouterEvidence {
                router_address: "0x1234".to_owned(),
                approval_target_address: Some("0x1234".to_owned()),
                approval_required: false,
                quoter_address: None,
                resolution_source: "sepolia_registry".to_owned(),
                user_confirmed: true,
            }),
            deadline: Some(UniswapDeadlineEvidence {
                deadline_unix_seconds: 1_700_000_000,
                source: "policy".to_owned(),
                user_confirmed: true,
            }),
            position: None,
        }
        .validate_for(&request)
        .expect("lp evidence should validate");
    }

    #[test]
    fn uniswap_lp_position_operation_requires_position_id() {
        let err = UniswapV3LpRequest {
            chain: "11155111".to_owned(),
            operation: UniswapLpOperationKind::Collect,
            token0_address: "0x1111".to_owned(),
            token0_symbol: Some("USDC".to_owned()),
            token1_address: "0x2222".to_owned(),
            token1_symbol: Some("WETH".to_owned()),
            fee_tier: 3000,
            desired_amount0: None,
            desired_amount1: None,
            tick_lower: None,
            tick_upper: None,
            position_manager_address: "0x1234".to_owned(),
            position_token_id: None,
            deadline_seconds: None,
            sender_address_hint: None,
        }
        .validate()
        .expect_err("collect requires position id");

        assert!(err.contains("position_token_id"));
    }

    #[test]
    fn uniswap_lp_verification_requires_tick_range_for_mint() {
        let err = UniswapV3LpVerificationContract {
            chain: "11155111".to_owned(),
            operation: UniswapLpOperationKind::Mint,
            position_manager_address: "0x1234".to_owned(),
            pool_address: "0xpool".to_owned(),
            token0_address: "0x1111".to_owned(),
            token1_address: "0x2222".to_owned(),
            fee_tier: 3000,
            position_token_id: None,
            expected_liquidity_delta: Some("100".to_owned()),
            expected_amount0_max: None,
            expected_amount1_max: None,
            tick_lower: None,
            tick_upper: None,
        }
        .validate()
        .expect_err("mint verification requires range");

        assert!(err.contains("tick_lower"));
    }
}
