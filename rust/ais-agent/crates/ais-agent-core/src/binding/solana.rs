use serde::{Deserialize, Serialize};
use solana_sdk::{
    hash::Hash, instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signature::Signature, transaction::VersionedTransaction,
};

mod serde_address_lookup_table_accounts {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use solana_sdk::{message::AddressLookupTableAccount, pubkey::Pubkey};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct AddressLookupTableAccountWire {
        key: Pubkey,
        addresses: Vec<Pubkey>,
    }

    pub fn serialize<S>(
        value: &Vec<AddressLookupTableAccount>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire: Vec<AddressLookupTableAccountWire> = value
            .iter()
            .map(|account| AddressLookupTableAccountWire {
                key: account.key,
                addresses: account.addresses.clone(),
            })
            .collect();
        wire.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<AddressLookupTableAccount>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = Vec::<AddressLookupTableAccountWire>::deserialize(deserializer)?;
        Ok(wire
            .into_iter()
            .map(|account| AddressLookupTableAccount {
                key: account.key,
                addresses: account.addresses,
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaConnectionSpec {
    pub http_url: String,
    #[serde(default)]
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolanaObserveBinding {
    Slot,
    AccountLamports,
    SplTokenBalance,
    AccountData,
    SignatureStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SolanaObserveRequest {
    Slot,
    AccountLamports { address: Pubkey },
    SplTokenBalance { token_account: Pubkey },
    AccountData { address: Pubkey },
    SignatureStatus { signature: Signature },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version", rename_all = "snake_case")]
pub enum SolanaTransactionRequest {
    Legacy {
        #[serde(default)]
        recent_blockhash: Option<Hash>,
        #[serde(default)]
        payer: Option<Pubkey>,
        instructions: Vec<Instruction>,
    },
    V0 {
        #[serde(default)]
        recent_blockhash: Option<Hash>,
        #[serde(default)]
        payer: Option<Pubkey>,
        instructions: Vec<Instruction>,
        #[serde(default)]
        #[serde(with = "serde_address_lookup_table_accounts")]
        address_lookup_tables: Vec<AddressLookupTableAccount>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolanaSimulateBinding {
    SimulateTransaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolanaActuateBinding {
    BroadcastSignedTransaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolanaVerifyBinding {
    SignatureStatus,
    EffectContractFromSignatureStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaSignedEnvelope {
    pub transaction: VersionedTransaction,
}
