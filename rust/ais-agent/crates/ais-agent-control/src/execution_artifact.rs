use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! execution_artifact_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

execution_artifact_id!(ExecutionStageId);
execution_artifact_id!(ExecutionCandidateId);
execution_artifact_id!(ExecutionOutputKey);
execution_artifact_id!(ExecutionPackageEntry);

/// Supported runtime families for artifact planning and execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionChainFamily {
    #[default]
    Evm,
    Solana,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionArtifactActor {
    #[serde(default)]
    pub sender_address_hint: Option<String>,
    #[serde(default)]
    pub recipient_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmTransactionCandidate {
    pub candidate_id: ExecutionCandidateId,
    pub to: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub calldata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaInstructionAccount {
    pub address: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaInstructionCandidate {
    pub program_id: String,
    #[serde(default)]
    pub accounts: Vec<SolanaInstructionAccount>,
    #[serde(default)]
    pub data_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolanaTransactionCandidate {
    pub candidate_id: ExecutionCandidateId,
    #[serde(default)]
    pub instructions: Vec<SolanaInstructionCandidate>,
}

/// Chain-specific transaction payloads that runtime stages can reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionTransactionCandidate {
    EvmTransaction(EvmTransactionCandidate),
    SolanaTransaction(SolanaTransactionCandidate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatusExpectation {
    Success,
}

/// Generic expression/value lookup surface for runtime-visible data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValueRef {
    Literal {
        value: Value,
    },
    Ref {
        #[serde(rename = "ref")]
        reference: String,
    },
    Cel {
        expression: String,
    },
}

/// Runtime predicates for branching and assertion guards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PredicateSpec {
    Comparison {
        left: ValueRef,
        op: ComparisonOperator,
        right: ValueRef,
    },
    Cel {
        expression: String,
    },
    Freshness {
        evidence_ref: String,
        max_age_ms: u64,
    },
    ReceiptStatus {
        receipt_ref: String,
        expected: ReceiptStatusExpectation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BranchTarget {
    GotoStage {
        stage_id: ExecutionStageId,
    },
    Assert {
        failure_code: String,
        message: String,
    },
}

/// Execute the referenced transaction candidate, then optionally continue to another stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionStage {
    pub stage_id: ExecutionStageId,
    pub candidate_ref: ExecutionCandidateId,
    #[serde(default)]
    pub exports: Vec<OutputExportSpec>,
    #[serde(default)]
    pub next_stage_id: Option<ExecutionStageId>,
}

/// Execute the referenced observation, export values, then optionally continue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObserveStage {
    pub stage_id: ExecutionStageId,
    pub observation_ref: String,
    #[serde(default)]
    pub exports: Vec<OutputExportSpec>,
    #[serde(default)]
    pub next_stage_id: Option<ExecutionStageId>,
}

/// Evaluate a generic predicate and jump to the next stage or fail closed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchStage {
    pub stage_id: ExecutionStageId,
    pub predicate: PredicateSpec,
    pub if_true: BranchTarget,
    pub if_false: BranchTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationSpec {
    pub observation_id: String,
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectSpec {
    pub effect_id: String,
    pub stage_id: ExecutionStageId,
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputExportSpec {
    pub output_key: ExecutionOutputKey,
    pub source: ValueRef,
}

/// Pause execution at a package-owned continuation boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationStage {
    pub stage_id: ExecutionStageId,
    #[serde(default)]
    pub required_outputs: Vec<ExecutionOutputKey>,
    pub package_entry: ExecutionPackageEntry,
    #[serde(default)]
    pub next_stage_id: Option<ExecutionStageId>,
}

/// Directed stage graph owned by the package but executed generically by runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionStage {
    Transaction(TransactionStage),
    Observe(ObserveStage),
    Branch(BranchStage),
    Continuation(ContinuationStage),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    #[serde(default)]
    pub quote_max_age_ms: Option<u64>,
    #[serde(default)]
    pub max_steps: Option<u32>,
    #[serde(default)]
    pub max_signer_requests: Option<u32>,
    #[serde(default)]
    pub max_continuations: Option<u32>,
    #[serde(default)]
    pub require_signer: Option<bool>,
    #[serde(default)]
    pub allow_recovery_patch: Option<bool>,
}

/// Stable artifact-first launch contract for the generic runtime cutover.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionArtifactLaunchSpec {
    pub protocol_package_id: String,
    pub action_key: String,
    pub chain_family: ExecutionChainFamily,
    #[serde(default)]
    pub allowed_chains: Vec<String>,
    pub entry_stage_id: ExecutionStageId,
    #[serde(default)]
    pub actor: Option<ExecutionArtifactActor>,
    #[serde(default)]
    pub transactions: Vec<ExecutionTransactionCandidate>,
    #[serde(default)]
    pub stages: Vec<ExecutionStage>,
    #[serde(default)]
    pub observations: Vec<ObservationSpec>,
    #[serde(default)]
    pub preconditions: Vec<ObservationSpec>,
    #[serde(default)]
    pub postconditions: Vec<ObservationSpec>,
    #[serde(default)]
    pub expected_effects: Vec<EffectSpec>,
    #[serde(default)]
    pub execution_policy: Option<ExecutionPolicy>,
    #[serde(default)]
    pub risk_class: Option<String>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    #[serde(default)]
    pub decoded_intent: Option<Value>,
    #[serde(default)]
    pub candidate_envelopes: Vec<Value>,
    #[serde(default)]
    pub decode_spec: Option<Value>,
    #[serde(default)]
    pub validation_plan: Option<Value>,
    #[serde(default)]
    pub evidence: Value,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl ExecutionChainFamily {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evm => "evm",
            Self::Solana => "solana",
        }
    }
}

impl ExecutionArtifactActor {
    #[must_use]
    pub fn sender_address_hint(&self) -> Option<&str> {
        self.sender_address_hint.as_deref()
    }

    #[must_use]
    pub fn recipient_address(&self) -> Option<&str> {
        self.recipient_address.as_deref()
    }
}

impl ExecutionTransactionCandidate {
    #[must_use]
    pub fn candidate_id(&self) -> &ExecutionCandidateId {
        match self {
            Self::EvmTransaction(candidate) => &candidate.candidate_id,
            Self::SolanaTransaction(candidate) => &candidate.candidate_id,
        }
    }

    #[must_use]
    pub fn chain_family(&self) -> ExecutionChainFamily {
        match self {
            Self::EvmTransaction(_) => ExecutionChainFamily::Evm,
            Self::SolanaTransaction(_) => ExecutionChainFamily::Solana,
        }
    }

    #[must_use]
    pub fn as_evm_transaction(&self) -> Option<&EvmTransactionCandidate> {
        match self {
            Self::EvmTransaction(candidate) => Some(candidate),
            Self::SolanaTransaction(_) => None,
        }
    }

    #[must_use]
    pub fn as_solana_transaction(&self) -> Option<&SolanaTransactionCandidate> {
        match self {
            Self::EvmTransaction(_) => None,
            Self::SolanaTransaction(candidate) => Some(candidate),
        }
    }
}

impl BranchTarget {
    #[must_use]
    pub fn goto_stage_id(&self) -> Option<&ExecutionStageId> {
        match self {
            Self::GotoStage { stage_id } => Some(stage_id),
            Self::Assert { .. } => None,
        }
    }
}

impl ExecutionStage {
    #[must_use]
    pub fn stage_id(&self) -> &ExecutionStageId {
        match self {
            Self::Transaction(stage) => &stage.stage_id,
            Self::Observe(stage) => &stage.stage_id,
            Self::Branch(stage) => &stage.stage_id,
            Self::Continuation(stage) => &stage.stage_id,
        }
    }

    #[must_use]
    pub fn next_stage_id(&self) -> Option<&ExecutionStageId> {
        match self {
            Self::Transaction(stage) => stage.next_stage_id.as_ref(),
            Self::Observe(stage) => stage.next_stage_id.as_ref(),
            Self::Branch(_) => None,
            Self::Continuation(stage) => stage.next_stage_id.as_ref(),
        }
    }

    #[must_use]
    pub fn as_transaction(&self) -> Option<&TransactionStage> {
        match self {
            Self::Transaction(stage) => Some(stage),
            Self::Observe(_) | Self::Branch(_) | Self::Continuation(_) => None,
        }
    }

    #[must_use]
    pub fn as_observe(&self) -> Option<&ObserveStage> {
        match self {
            Self::Observe(stage) => Some(stage),
            Self::Transaction(_) | Self::Branch(_) | Self::Continuation(_) => None,
        }
    }

    #[must_use]
    pub fn as_branch(&self) -> Option<&BranchStage> {
        match self {
            Self::Transaction(_) | Self::Observe(_) | Self::Continuation(_) => None,
            Self::Branch(stage) => Some(stage),
        }
    }

    #[must_use]
    pub fn as_continuation(&self) -> Option<&ContinuationStage> {
        match self {
            Self::Transaction(_) | Self::Observe(_) | Self::Branch(_) => None,
            Self::Continuation(stage) => Some(stage),
        }
    }
}

impl ExecutionArtifactLaunchSpec {
    #[must_use]
    pub fn chain_scope(&self) -> Option<&str> {
        match self.allowed_chains.as_slice() {
            [chain_scope] => Some(chain_scope.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn entry_stage(&self) -> Option<&ExecutionStage> {
        self.stage(self.entry_stage_id.as_str())
    }

    #[must_use]
    pub fn stage(&self, stage_id: &str) -> Option<&ExecutionStage> {
        self.stages
            .iter()
            .find(|stage| stage.stage_id().as_str() == stage_id)
    }

    #[must_use]
    pub fn transaction_candidate(
        &self,
        candidate_id: &str,
    ) -> Option<&ExecutionTransactionCandidate> {
        self.transactions
            .iter()
            .find(|candidate| candidate.candidate_id().as_str() == candidate_id)
    }

    #[must_use]
    pub fn semantic_contract_active(&self) -> bool {
        self.risk_class.is_some()
            || !self.risk_tags.is_empty()
            || self.decoded_intent.is_some()
            || !self.candidate_envelopes.is_empty()
            || self.decode_spec.is_some()
            || self.validation_plan.is_some()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn execution_transaction_candidate_helpers_expose_runtime_shape() {
        let candidate = ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
            candidate_id: "tx.swap".into(),
            to: "0x1111111111111111111111111111111111111111".to_owned(),
            value: Some("0".to_owned()),
            calldata: Some("0xdeadbeef".to_owned()),
        });

        assert_eq!(candidate.candidate_id().as_str(), "tx.swap");
        assert_eq!(candidate.chain_family(), ExecutionChainFamily::Evm);
        assert!(candidate.as_evm_transaction().is_some());
        assert!(candidate.as_solana_transaction().is_none());
    }

    #[test]
    fn execution_artifact_lookup_helpers_follow_stage_graph_ids() {
        let artifact = ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "swap".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["8453".to_owned()],
            entry_stage_id: "stage.swap".into(),
            actor: None,
            transactions: vec![ExecutionTransactionCandidate::EvmTransaction(
                EvmTransactionCandidate {
                    candidate_id: "tx.swap".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0xdeadbeef".to_owned()),
                },
            )],
            stages: vec![ExecutionStage::Transaction(TransactionStage {
                stage_id: "stage.swap".into(),
                candidate_ref: "tx.swap".into(),
                exports: vec![OutputExportSpec {
                    output_key: "swap.received_atomic".into(),
                    source: ValueRef::Literal {
                        value: json!("100"),
                    },
                }],
                next_stage_id: None,
            })],
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            risk_class: None,
            risk_tags: Vec::new(),
            decoded_intent: None,
            candidate_envelopes: Vec::new(),
            decode_spec: None,
            validation_plan: None,
            evidence: Value::Null,
            metadata: BTreeMap::new(),
        };

        let stage = artifact.entry_stage().expect("entry stage");
        let tx_stage = stage.as_transaction().expect("transaction stage");

        assert_eq!(stage.stage_id().as_str(), "stage.swap");
        assert_eq!(tx_stage.candidate_ref.as_str(), "tx.swap");
        assert_eq!(
            artifact
                .transaction_candidate("tx.swap")
                .expect("transaction candidate")
                .candidate_id()
                .as_str(),
            "tx.swap"
        );
        assert_eq!(artifact.chain_scope(), Some("8453"));
    }

    #[test]
    fn execution_artifact_chain_scope_helper_requires_exactly_one_scope() {
        let mut artifact = ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.transfer".to_owned(),
            action_key: "native_transfer".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["eip155:1".to_owned()],
            entry_stage_id: "stage.transfer".into(),
            actor: None,
            transactions: Vec::new(),
            stages: Vec::new(),
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            risk_class: None,
            risk_tags: Vec::new(),
            decoded_intent: None,
            candidate_envelopes: Vec::new(),
            decode_spec: None,
            validation_plan: None,
            evidence: Value::Null,
            metadata: BTreeMap::new(),
        };

        assert_eq!(artifact.chain_scope(), Some("eip155:1"));

        artifact.allowed_chains = Vec::new();
        assert_eq!(artifact.chain_scope(), None);

        artifact.allowed_chains = vec!["eip155:1".to_owned(), "eip155:8453".to_owned()];
        assert_eq!(artifact.chain_scope(), None);
    }

    #[test]
    fn execution_artifact_semantic_contract_helper_tracks_presence_of_semantic_fields() {
        let mut artifact = ExecutionArtifactLaunchSpec::default();
        assert!(!artifact.semantic_contract_active());

        artifact.validation_plan = Some(json!({ "kind": "static" }));
        assert!(artifact.semantic_contract_active());
    }
}
