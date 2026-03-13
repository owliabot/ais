use ais_agent_control::transfer::{
    Erc20TransferRequest, NativeTransferRequest, TransferActionFamily, TransferVerificationContract,
};
use serde::{Deserialize, Serialize};

use crate::effect::{EffectAssertion, EffectContract, EffectContractKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum TransferRequest {
    Native(NativeTransferRequest),
    Erc20(Erc20TransferRequest),
}

impl TransferRequest {
    pub fn family(&self) -> TransferActionFamily {
        match self {
            Self::Native(_) => TransferActionFamily::NativeTransfer,
            Self::Erc20(_) => TransferActionFamily::Erc20Transfer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum TransferEffectTemplate {
    Native(NativeTransferEffectTemplate),
    Erc20(Erc20TransferEffectTemplate),
}

impl TransferEffectTemplate {
    pub fn verification(&self) -> &TransferVerificationContract {
        match self {
            Self::Native(template) => &template.verification,
            Self::Erc20(template) => &template.verification,
        }
    }

    pub fn to_effect_contract(&self, effect_id: impl Into<String>) -> EffectContract {
        let effect_id = effect_id.into();
        match self {
            Self::Native(_template) => EffectContract {
                effect_id,
                kind: EffectContractKind::AssetDelta,
                assertions: vec![
                    EffectAssertion {
                        expression: "receipt.status == true".to_owned(),
                        description: "native transfer receipt must succeed".to_owned(),
                    },
                    EffectAssertion {
                        expression: "post.decoded_u256 != pre.decoded_u256".to_owned(),
                        description: "recipient native balance should change after the transfer"
                            .to_owned(),
                    },
                ],
                tolerance_hint: Some(
                    "native transfer verification may tolerate fee-side sender deltas".to_owned(),
                ),
            },
            Self::Erc20(_template) => EffectContract {
                effect_id,
                kind: EffectContractKind::AssetDelta,
                assertions: vec![
                    EffectAssertion {
                        expression: "receipt.status == true".to_owned(),
                        description: "ERC20 transfer receipt must succeed".to_owned(),
                    },
                    EffectAssertion {
                        expression: "post.decoded_u256 != pre.decoded_u256".to_owned(),
                        description: "recipient ERC20 balance should change after the transfer"
                            .to_owned(),
                    },
                ],
                tolerance_hint: Some(
                    "ERC20 transfer verification should compare token-specific deltas".to_owned(),
                ),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeTransferEffectTemplate {
    pub request: NativeTransferRequest,
    pub verification: TransferVerificationContract,
}

impl NativeTransferEffectTemplate {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        self.verification
            .validate(TransferActionFamily::NativeTransfer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc20TransferEffectTemplate {
    pub request: Erc20TransferRequest,
    pub verification: TransferVerificationContract,
}

impl Erc20TransferEffectTemplate {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        self.verification
            .validate(TransferActionFamily::Erc20Transfer)
    }
}

#[cfg(test)]
mod tests {
    use ais_agent_control::transfer::{
        Erc20TransferRequest, NativeTransferRequest, TransferVerificationContract,
    };

    use super::{
        Erc20TransferEffectTemplate, NativeTransferEffectTemplate, TransferEffectTemplate,
    };

    #[test]
    fn native_transfer_effect_contract_contains_recipient_assertion() {
        let template = TransferEffectTemplate::Native(NativeTransferEffectTemplate {
            request: NativeTransferRequest {
                chain: "11155111".to_owned(),
                recipient: "0xabc".to_owned(),
                requested_amount: "0.5".to_owned(),
                asset_symbol: Some("ETH".to_owned()),
                sender_address_hint: Some("0xsender".to_owned()),
            },
            verification: TransferVerificationContract {
                chain: "11155111".to_owned(),
                token_address: None,
                recipient_address: "0xabc".to_owned(),
                expected_amount_atomic: "500000000000000000".to_owned(),
                sender_address_hint: Some("0xsender".to_owned()),
                require_recipient_delta: true,
                require_sender_delta: true,
            },
        });

        let effect = template.to_effect_contract("effect-native");
        assert_eq!(effect.kind, crate::effect::EffectContractKind::AssetDelta);
        assert!(
            effect.assertions[1]
                .expression
                .contains("post.decoded_u256 != pre.decoded_u256"),
            "native transfer should describe a recipient-balance change"
        );
    }

    #[test]
    fn erc20_transfer_effect_template_requires_token_address() {
        let err = Erc20TransferEffectTemplate {
            request: Erc20TransferRequest {
                chain: "11155111".to_owned(),
                token_address: "0x1c7d".to_owned(),
                token_symbol: Some("USDC".to_owned()),
                recipient: "0xabc".to_owned(),
                requested_amount: "10".to_owned(),
                sender_address_hint: Some("0xsender".to_owned()),
            },
            verification: TransferVerificationContract {
                chain: "11155111".to_owned(),
                token_address: None,
                recipient_address: "0xabc".to_owned(),
                expected_amount_atomic: "10000000".to_owned(),
                sender_address_hint: None,
                require_recipient_delta: true,
                require_sender_delta: false,
            },
        }
        .validate()
        .expect_err("missing token verification address should fail");

        assert!(err.contains("token_address"));
    }

    #[test]
    fn erc20_transfer_effect_contract_contains_token_assertion() {
        let template = TransferEffectTemplate::Erc20(Erc20TransferEffectTemplate {
            request: Erc20TransferRequest {
                chain: "11155111".to_owned(),
                token_address: "0x1c7d".to_owned(),
                token_symbol: Some("USDC".to_owned()),
                recipient: "0xabc".to_owned(),
                requested_amount: "10".to_owned(),
                sender_address_hint: Some("0xsender".to_owned()),
            },
            verification: TransferVerificationContract {
                chain: "11155111".to_owned(),
                token_address: Some("0x1c7d".to_owned()),
                recipient_address: "0xabc".to_owned(),
                expected_amount_atomic: "10000000".to_owned(),
                sender_address_hint: Some("0xsender".to_owned()),
                require_recipient_delta: true,
                require_sender_delta: true,
            },
        });

        let effect = template.to_effect_contract("effect-erc20");
        assert!(
            effect.assertions[1]
                .expression
                .contains("post.decoded_u256 != pre.decoded_u256"),
            "erc20 transfer should describe token-specific recipient balance change"
        );
    }
}
