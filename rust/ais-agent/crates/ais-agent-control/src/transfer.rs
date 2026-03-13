use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferActionFamily {
    NativeTransfer,
    Erc20Transfer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeTransferRequest {
    pub chain: String,
    pub recipient: String,
    pub requested_amount: String,
    #[serde(default)]
    pub asset_symbol: Option<String>,
    #[serde(default)]
    pub sender_address_hint: Option<String>,
}

impl NativeTransferRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("native_transfer_request.chain must not be empty");
        }
        if self.recipient.trim().is_empty() {
            return Err("native_transfer_request.recipient must not be empty");
        }
        if self.requested_amount.trim().is_empty() {
            return Err("native_transfer_request.requested_amount must not be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc20TransferRequest {
    pub chain: String,
    pub token_address: String,
    #[serde(default)]
    pub token_symbol: Option<String>,
    pub recipient: String,
    pub requested_amount: String,
    #[serde(default)]
    pub sender_address_hint: Option<String>,
}

impl Erc20TransferRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("erc20_transfer_request.chain must not be empty");
        }
        if self.token_address.trim().is_empty() {
            return Err("erc20_transfer_request.token_address must not be empty");
        }
        if self.recipient.trim().is_empty() {
            return Err("erc20_transfer_request.recipient must not be empty");
        }
        if self.requested_amount.trim().is_empty() {
            return Err("erc20_transfer_request.requested_amount must not be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRecipientEvidence {
    pub user_input: String,
    pub normalized_address: String,
    pub source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferAmountEvidence {
    pub user_input: String,
    pub normalized_amount: String,
    #[serde(default)]
    pub atomic_amount: Option<String>,
    #[serde(default)]
    pub decimals: Option<u8>,
    pub source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferTokenEvidence {
    pub token_address: String,
    #[serde(default)]
    pub token_symbol: Option<String>,
    pub decimals: u8,
    pub resolution_source: String,
    #[serde(default)]
    pub user_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferBalanceEvidence {
    pub owner: String,
    pub balance_atomic: String,
    #[serde(default)]
    pub decimals: Option<u8>,
    pub observed_at_ms: u64,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferEvidencePackage {
    pub recipient: TransferRecipientEvidence,
    pub amount: TransferAmountEvidence,
    #[serde(default)]
    pub token: Option<TransferTokenEvidence>,
    #[serde(default)]
    pub sender_balance: Option<TransferBalanceEvidence>,
}

impl TransferEvidencePackage {
    pub fn validate_for(&self, family: TransferActionFamily) -> Result<(), &'static str> {
        if self.recipient.user_input.trim().is_empty() {
            return Err("transfer_evidence.recipient.user_input must not be empty");
        }
        if self.recipient.normalized_address.trim().is_empty() {
            return Err("transfer_evidence.recipient.normalized_address must not be empty");
        }
        if self.recipient.source.trim().is_empty() {
            return Err("transfer_evidence.recipient.source must not be empty");
        }
        if self.amount.user_input.trim().is_empty() {
            return Err("transfer_evidence.amount.user_input must not be empty");
        }
        if self.amount.normalized_amount.trim().is_empty() {
            return Err("transfer_evidence.amount.normalized_amount must not be empty");
        }
        if self.amount.source.trim().is_empty() {
            return Err("transfer_evidence.amount.source must not be empty");
        }

        if matches!(family, TransferActionFamily::Erc20Transfer) {
            let token = self
                .token
                .as_ref()
                .ok_or("erc20_transfer requires transfer_evidence.token")?;
            if token.token_address.trim().is_empty() {
                return Err("transfer_evidence.token.token_address must not be empty");
            }
            if token.resolution_source.trim().is_empty() {
                return Err("transfer_evidence.token.resolution_source must not be empty");
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferVerificationContract {
    pub chain: String,
    #[serde(default)]
    pub token_address: Option<String>,
    pub recipient_address: String,
    pub expected_amount_atomic: String,
    #[serde(default)]
    pub sender_address_hint: Option<String>,
    #[serde(default)]
    pub require_recipient_delta: bool,
    #[serde(default)]
    pub require_sender_delta: bool,
}

impl TransferVerificationContract {
    pub fn validate(&self, family: TransferActionFamily) -> Result<(), &'static str> {
        if self.chain.trim().is_empty() {
            return Err("transfer_verification_contract.chain must not be empty");
        }
        if self.recipient_address.trim().is_empty() {
            return Err("transfer_verification_contract.recipient_address must not be empty");
        }
        if self.expected_amount_atomic.trim().is_empty() {
            return Err("transfer_verification_contract.expected_amount_atomic must not be empty");
        }
        if matches!(family, TransferActionFamily::Erc20Transfer)
            && self
                .token_address
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err("erc20_transfer verification requires token_address");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Erc20TransferRequest, NativeTransferRequest, TransferActionFamily, TransferAmountEvidence,
        TransferEvidencePackage, TransferRecipientEvidence, TransferTokenEvidence,
        TransferVerificationContract,
    };

    #[test]
    fn native_transfer_request_requires_non_empty_fields() {
        let err = NativeTransferRequest {
            chain: String::new(),
            recipient: "0xabc".to_owned(),
            requested_amount: "1".to_owned(),
            asset_symbol: None,
            sender_address_hint: None,
        }
        .validate()
        .expect_err("empty chain should fail");
        assert!(err.contains("chain"));
    }

    #[test]
    fn erc20_transfer_requires_token_evidence() {
        let err = TransferEvidencePackage {
            recipient: TransferRecipientEvidence {
                user_input: "alice".to_owned(),
                normalized_address: "0xabc".to_owned(),
                source: "user".to_owned(),
                user_confirmed: true,
            },
            amount: TransferAmountEvidence {
                user_input: "10".to_owned(),
                normalized_amount: "10".to_owned(),
                atomic_amount: Some("10000000".to_owned()),
                decimals: Some(6),
                source: "user".to_owned(),
                user_confirmed: true,
            },
            token: None,
            sender_balance: None,
        }
        .validate_for(TransferActionFamily::Erc20Transfer)
        .expect_err("missing token evidence should fail");
        assert!(err.contains("requires transfer_evidence.token"));
    }

    #[test]
    fn erc20_transfer_request_and_evidence_validate() {
        Erc20TransferRequest {
            chain: "11155111".to_owned(),
            token_address: "0x1c7d".to_owned(),
            token_symbol: Some("USDC".to_owned()),
            recipient: "0xabc".to_owned(),
            requested_amount: "10".to_owned(),
            sender_address_hint: Some("0xsender".to_owned()),
        }
        .validate()
        .expect("request should validate");

        TransferEvidencePackage {
            recipient: TransferRecipientEvidence {
                user_input: "alice".to_owned(),
                normalized_address: "0xabc".to_owned(),
                source: "user".to_owned(),
                user_confirmed: true,
            },
            amount: TransferAmountEvidence {
                user_input: "10".to_owned(),
                normalized_amount: "10".to_owned(),
                atomic_amount: Some("10000000".to_owned()),
                decimals: Some(6),
                source: "token_metadata".to_owned(),
                user_confirmed: true,
            },
            token: Some(TransferTokenEvidence {
                token_address: "0x1c7d".to_owned(),
                token_symbol: Some("USDC".to_owned()),
                decimals: 6,
                resolution_source: "token_registry".to_owned(),
                user_confirmed: true,
            }),
            sender_balance: None,
        }
        .validate_for(TransferActionFamily::Erc20Transfer)
        .expect("evidence should validate");
    }

    #[test]
    fn verification_contract_requires_token_for_erc20() {
        let err = TransferVerificationContract {
            chain: "11155111".to_owned(),
            token_address: None,
            recipient_address: "0xabc".to_owned(),
            expected_amount_atomic: "100".to_owned(),
            sender_address_hint: None,
            require_recipient_delta: true,
            require_sender_delta: false,
        }
        .validate(TransferActionFamily::Erc20Transfer)
        .expect_err("erc20 verification requires token");
        assert!(err.contains("token_address"));
    }
}
