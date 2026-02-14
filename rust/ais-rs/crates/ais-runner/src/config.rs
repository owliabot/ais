use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use ais_engine::RouterExecutor;
use ais_evm_executor::{
    EvmCallExecutionConfig, EvmExecutor, EvmProviderRegistry, EvmRpcEndpoint,
    LocalPrivateKeySigner as EvmLocalPrivateKeySigner,
};
use ais_sdk::PlanDocument;
use ais_solana_executor::{
    CommitmentLevel, LocalPrivateKeySigner as SolanaLocalPrivateKeySigner, ProviderError as SolanaProviderError,
    SolanaExecutor, SolanaInstructionExecutionConfig, SolanaProviderRegistry, SolanaRpcClient,
    SolanaRpcClientFactory, SolanaRpcEndpoint,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerConfig {
    #[serde(default = "default_runner_schema")]
    pub schema: String,
    #[serde(default)]
    pub engine: RunnerEngineConfig,
    #[serde(default)]
    pub chains: BTreeMap<String, ChainConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunnerEngineConfig {
    #[serde(default)]
    pub max_concurrency: Option<u32>,
    #[serde(default)]
    pub per_chain: BTreeMap<String, ChainConcurrency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ChainConcurrency {
    #[serde(default)]
    pub max_read_concurrency: Option<u32>,
    #[serde(default)]
    pub max_write_concurrency: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainConfig {
    pub rpc_url: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub wait_for_receipt: Option<bool>,
    #[serde(default)]
    pub receipt_poll: Option<PollConfig>,
    #[serde(default)]
    pub commitment: Option<CommitmentLevel>,
    #[serde(default)]
    pub wait_for_confirmation: Option<bool>,
    #[serde(default)]
    pub confirmation_poll: Option<PollConfig>,
    #[serde(default)]
    pub signer: Option<SignerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollConfig {
    pub interval_ms: u64,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignerConfig {
    EvmPrivateKey { private_key: String },
    SolanaPrivateKey { private_key: String },
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerConfigError {
    #[error("read runner config failed `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("runner config parse failed: {0}")]
    Parse(String),
    #[error("runner config validation failed: {0:?}")]
    Validation(Vec<StructuredIssue>),
}

pub fn load_runner_config(path: &Path) -> Result<RunnerConfig, RunnerConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| RunnerConfigError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let expanded = expand_env_placeholders(raw.as_str()).map_err(RunnerConfigError::Parse)?;
    let config: RunnerConfig = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str(expanded.as_str()).map_err(|error| {
            RunnerConfigError::Parse(format!("json decode error: {error}"))
        })?,
        Some("yaml") | Some("yml") => serde_yaml::from_str(expanded.as_str()).map_err(|error| {
            RunnerConfigError::Parse(format!("yaml decode error: {error}"))
        })?,
        _ => serde_yaml::from_str(expanded.as_str())
            .or_else(|_| serde_json::from_str(expanded.as_str()))
            .map_err(|error| RunnerConfigError::Parse(error.to_string()))?,
    };

    let mut issues = validate_runner_config(&config);
    StructuredIssue::sort_stable(&mut issues);
    if !issues.is_empty() {
        return Err(RunnerConfigError::Validation(issues));
    }
    Ok(config)
}

pub fn validate_runner_config(config: &RunnerConfig) -> Vec<StructuredIssue> {
    let mut issues = Vec::<StructuredIssue>::new();
    if config.schema != default_runner_schema() {
        issues.push(config_issue(
            "runner.config.schema",
            vec![FieldPathSegment::Key("schema".to_string())],
            format!(
                "unsupported runner config schema `{}` (expected `{}`)",
                config.schema,
                default_runner_schema()
            ),
        ));
    }
    if config.chains.is_empty() {
        issues.push(config_issue(
            "runner.config.chains.non_empty",
            vec![FieldPathSegment::Key("chains".to_string())],
            "runner config must define at least one chain".to_string(),
        ));
    }

    let configured_chains = config.chains.keys().cloned().collect::<BTreeSet<_>>();
    for (chain, chain_config) in &config.chains {
        let path = chain_path_segments(chain);
        if !(chain.starts_with("eip155:") || chain.starts_with("solana:")) {
            issues.push(config_issue(
                "runner.config.chain.unsupported",
                path.clone(),
                format!("unsupported chain id `{chain}`"),
            ));
        }
        if !is_supported_rpc_url(chain_config.rpc_url.as_str()) {
            let mut field = path.clone();
            field.push(FieldPathSegment::Key("rpc_url".to_string()));
            issues.push(config_issue(
                "runner.config.rpc_url",
                field,
                format!("rpc_url for `{chain}` must be http(s) or ws(s)"),
            ));
        }
        if matches!(chain_config.timeout_ms, Some(0)) {
            let mut field = path.clone();
            field.push(FieldPathSegment::Key("timeout_ms".to_string()));
            issues.push(config_issue(
                "runner.config.timeout",
                field,
                format!("timeout_ms for `{chain}` must be > 0"),
            ));
        }
        if let Some(signer) = &chain_config.signer {
            let signer_ok = matches!(
                (chain.starts_with("eip155:"), chain.starts_with("solana:"), signer),
                (true, _, SignerConfig::EvmPrivateKey { .. })
                    | (_, true, SignerConfig::SolanaPrivateKey { .. })
            );
            if !signer_ok {
                let mut field = path.clone();
                field.push(FieldPathSegment::Key("signer".to_string()));
                issues.push(config_issue(
                    "runner.config.signer.type_mismatch",
                    field,
                    format!("signer type does not match chain `{chain}`"),
                ));
            }
        }
    }

    for chain in config.engine.per_chain.keys() {
        if !configured_chains.contains(chain) {
            issues.push(config_issue(
                "runner.config.engine.per_chain.unknown_chain",
                vec![
                    FieldPathSegment::Key("engine".to_string()),
                    FieldPathSegment::Key("per_chain".to_string()),
                    FieldPathSegment::Key(chain.clone()),
                ],
                format!("engine.per_chain entry `{chain}` has no matching chains config"),
            ));
        }
    }

    issues
}

pub fn build_router_executor(config: &RunnerConfig) -> Result<RouterExecutor, Vec<StructuredIssue>> {
    let mut issues = validate_runner_config(config);
    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return Err(issues);
    }

    let mut router = RouterExecutor::new();
    for (chain, chain_config) in &config.chains {
        if chain.starts_with("eip155:") {
            match build_evm_executor(chain, chain_config) {
                Ok(executor) => router.register(format!("evm:{chain}"), chain.clone(), Box::new(executor)),
                Err(issue) => issues.push(issue),
            }
            continue;
        }
        if chain.starts_with("solana:") {
            match build_solana_executor(chain, chain_config) {
                Ok(executor) => router.register(format!("solana:{chain}"), chain.clone(), Box::new(executor)),
                Err(issue) => issues.push(issue),
            }
        }
    }
    if issues.is_empty() {
        Ok(router)
    } else {
        StructuredIssue::sort_stable(&mut issues);
        Err(issues)
    }
}

pub fn build_router_executor_for_plan(
    plan: &PlanDocument,
    config: &RunnerConfig,
) -> Result<RouterExecutor, Vec<StructuredIssue>> {
    let mut issues = Vec::<StructuredIssue>::new();
    for (index, node) in plan.nodes.iter().enumerate() {
        if let Some(chain) = node
            .as_object()
            .and_then(|object| object.get("chain"))
            .and_then(Value::as_str)
        {
            if !config.chains.contains_key(chain) {
                issues.push(config_issue(
                    "runner.config.chain_missing",
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(index),
                        FieldPathSegment::Key("chain".to_string()),
                    ],
                    format!("plan node chain `{chain}` is not configured in runner config"),
                ));
            }
        }
    }
    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return Err(issues);
    }
    build_router_executor(config)
}

fn build_evm_executor(chain: &str, chain_config: &ChainConfig) -> Result<EvmExecutor, StructuredIssue> {
    let mut endpoint = EvmRpcEndpoint::new(chain.to_string(), chain_config.rpc_url.clone()).map_err(|error| {
        chain_issue(chain, "runner.config.evm.endpoint", format!("invalid evm endpoint: {error}"))
    })?;
    if let Some(timeout_ms) = chain_config.timeout_ms {
        endpoint = endpoint.with_timeout_ms(timeout_ms).map_err(|error| {
            chain_issue(chain, "runner.config.evm.endpoint", format!("invalid evm timeout: {error}"))
        })?;
    }
    let registry = EvmProviderRegistry::from_endpoints(vec![endpoint]).map_err(|error| {
        chain_issue(chain, "runner.config.evm.registry", format!("build evm provider registry failed: {error}"))
    })?;
    let mut executor = EvmExecutor::new(registry);
    if let Some(signer_config) = &chain_config.signer {
        let signer = match signer_config {
            SignerConfig::EvmPrivateKey { private_key } => {
                EvmLocalPrivateKeySigner::from_hex(private_key.as_str()).map_err(|error| {
                    chain_issue(chain, "runner.config.evm.signer", format!("invalid evm signer key: {error}"))
                })?
            }
            SignerConfig::SolanaPrivateKey { .. } => {
                return Err(chain_issue(
                    chain,
                    "runner.config.evm.signer",
                    "evm chain must use evm_private_key signer".to_string(),
                ))
            }
        };
        executor = executor.with_signer(Box::new(signer));
    }
    let mut call_config = EvmCallExecutionConfig::default();
    if let Some(wait_for_receipt) = chain_config.wait_for_receipt {
        call_config.wait_for_receipt = wait_for_receipt;
    }
    if let Some(poll) = &chain_config.receipt_poll {
        call_config.poll_interval_ms = poll.interval_ms;
        call_config.max_poll_attempts = poll.max_attempts;
    }
    executor = executor.with_call_config(call_config);
    Ok(executor)
}

fn build_solana_executor(
    chain: &str,
    chain_config: &ChainConfig,
) -> Result<SolanaExecutor, StructuredIssue> {
    let mut endpoint =
        SolanaRpcEndpoint::new(chain.to_string(), chain_config.rpc_url.clone()).map_err(|error| {
            chain_issue(
                chain,
                "runner.config.solana.endpoint",
                format!("invalid solana endpoint: {error}"),
            )
        })?;
    endpoint.commitment = chain_config.commitment.unwrap_or_default();
    if let Some(timeout_ms) = chain_config.timeout_ms {
        endpoint = endpoint.with_timeout_ms(timeout_ms).map_err(|error| {
            chain_issue(
                chain,
                "runner.config.solana.endpoint",
                format!("invalid solana timeout: {error}"),
            )
        })?;
    }
    let registry = SolanaProviderRegistry::from_endpoints(vec![endpoint]).map_err(|error| {
        chain_issue(
            chain,
            "runner.config.solana.registry",
            format!("build solana provider registry failed: {error}"),
        )
    })?;
    let mut executor = SolanaExecutor::new(registry, Box::new(UnwiredSolanaRpcClientFactory));
    if let Some(signer_config) = &chain_config.signer {
        let signer = match signer_config {
            SignerConfig::SolanaPrivateKey { private_key } => {
                SolanaLocalPrivateKeySigner::from_config(private_key.clone()).map_err(|error| {
                    chain_issue(
                        chain,
                        "runner.config.solana.signer",
                        format!("invalid solana signer key: {error}"),
                    )
                })?
            }
            SignerConfig::EvmPrivateKey { .. } => {
                return Err(chain_issue(
                    chain,
                    "runner.config.solana.signer",
                    "solana chain must use solana_private_key signer".to_string(),
                ))
            }
        };
        executor = executor.with_signer(Box::new(signer));
    }
    let mut instruction_config = SolanaInstructionExecutionConfig::default();
    if let Some(wait_for_confirmation) = chain_config.wait_for_confirmation {
        instruction_config.wait_for_confirmation = wait_for_confirmation;
    }
    if let Some(poll) = &chain_config.confirmation_poll {
        instruction_config.poll_interval_ms = poll.interval_ms;
        instruction_config.max_poll_attempts = poll.max_attempts;
    }
    executor = executor.with_instruction_config(instruction_config);
    Ok(executor)
}

fn chain_path_segments(chain: &str) -> Vec<FieldPathSegment> {
    vec![
        FieldPathSegment::Key("chains".to_string()),
        FieldPathSegment::Key(chain.to_string()),
    ]
}

fn chain_issue(chain: &str, reference: &str, message: String) -> StructuredIssue {
    config_issue(reference, chain_path_segments(chain), message)
}

fn config_issue(reference: &str, path: Vec<FieldPathSegment>, message: String) -> StructuredIssue {
    StructuredIssue {
        kind: "runner_config_error".to_string(),
        severity: IssueSeverity::Error,
        node_id: None,
        field_path: FieldPath::from_segments(path),
        message,
        reference: Some(reference.to_string()),
        related: None,
    }
}

fn default_runner_schema() -> String {
    "ais-runner/0.0.1".to_string()
}

fn is_supported_rpc_url(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("ws://")
        || value.starts_with("wss://")
}

fn expand_env_placeholders(input: &str) -> Result<String, String> {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(start_offset) = input[cursor..].find("${") {
        let start = cursor + start_offset;
        out.push_str(&input[cursor..start]);
        let var_start = start + 2;
        let Some(end_offset) = input[var_start..].find('}') else {
            return Err("unterminated env placeholder `${...`".to_string());
        };
        let end = var_start + end_offset;
        let key = &input[var_start..end];
        if key.is_empty() {
            return Err("empty env placeholder `${}`".to_string());
        }
        let value = std::env::var(key)
            .map_err(|_| format!("missing env var for placeholder `${{{key}}}`"))?;
        out.push_str(value.as_str());
        cursor = end + 1;
    }
    out.push_str(&input[cursor..]);
    Ok(out)
}

struct UnwiredSolanaRpcClientFactory;

impl SolanaRpcClientFactory for UnwiredSolanaRpcClientFactory {
    fn build_client(
        &self,
        _endpoint: &SolanaRpcEndpoint,
    ) -> Result<Box<dyn SolanaRpcClient>, SolanaProviderError> {
        Err(SolanaProviderError::Transport(
            "runner solana rpc client is not wired yet".to_string(),
        ))
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
