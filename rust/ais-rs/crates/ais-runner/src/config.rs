use ais_core::{FieldPath, FieldPathSegment, IssueSeverity, StructuredIssue};
use ais_engine::{
    execution_types_for_route_preset, ExecutionTypeRoutePreset, PluginExecutionTypeCapabilities,
    RouterExecutor, RuntimeExecutionTypeRegistry,
};
use ais_evm_executor::{
    EvmCallExecutionConfig, EvmExecutor, EvmProviderRegistry, EvmRpcEndpoint,
    LocalPrivateKeySigner as EvmLocalPrivateKeySigner,
};
use ais_offchain_executor::{OffchainApyExecutor, OffchainApyExecutorConfig};
use ais_sdk::PlanDocument;
use ais_solana_executor::{
    CommitmentLevel, LocalPrivateKeySigner as SolanaLocalPrivateKeySigner,
    ProviderError as SolanaProviderError, SolanaExecutor, SolanaInstructionExecutionConfig,
    SolanaProviderRegistry, SolanaRpcClient, SolanaRpcClientFactory, SolanaRpcEndpoint,
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
    pub llm: Option<RunnerLlmConfig>,
    #[serde(default)]
    pub chains: BTreeMap<String, ChainConfig>,
    #[serde(default)]
    pub plugins: RunnerPluginsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerLlmConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub fallback: Vec<RunnerLlmEndpointConfig>,
    #[serde(default)]
    pub max_retries_per_provider: Option<u8>,
    #[serde(default)]
    pub rotation: RunnerLlmRotationMode,
    #[serde(default)]
    pub controller_prompts_dir: Option<String>,
    #[serde(default)]
    pub operator_templates_dir: Option<String>,
    #[serde(default)]
    pub planner_context_token_budget: Option<usize>,
    #[serde(default)]
    pub max_tool_rounds: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_token_count")]
    pub context_limit_tokens: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerLlmEndpointConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    #[serde(default)]
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunnerLlmRotationMode {
    #[default]
    StickyPrimary,
    RoundRobin,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunnerPluginsConfig {
    #[serde(default)]
    pub execution: RunnerExecutionPluginsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunnerExecutionPluginsConfig {
    #[serde(default)]
    pub offchain_apy_query: Option<OffchainApyPluginConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffchainApyPluginConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub retry_backoff_ms: Option<u64>,
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
    let expanded = match expand_env_placeholders(raw.as_str()) {
        Ok(expanded) => expanded,
        Err(error) => {
            let mut issues = collect_env_placeholder_issues(path, raw.as_str());
            StructuredIssue::sort_stable(&mut issues);
            if !issues.is_empty() {
                return Err(RunnerConfigError::Validation(issues));
            }
            return Err(RunnerConfigError::Parse(error));
        }
    };
    let config: RunnerConfig = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str(expanded.as_str())
            .map_err(|error| RunnerConfigError::Parse(format!("json decode error: {error}")))?,
        Some("yaml") | Some("yml") => serde_yaml::from_str(expanded.as_str())
            .map_err(|error| RunnerConfigError::Parse(format!("yaml decode error: {error}")))?,
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
        let family = chain_family(chain);
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
            validate_signer_family(chain, family, signer, path.clone(), &mut issues);
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

    validate_offchain_plugin_config(config, &configured_chains, &mut issues);
    validate_llm_config(config, &mut issues);

    issues
}

pub fn build_router_executor(
    config: &RunnerConfig,
) -> Result<RouterExecutor, Vec<StructuredIssue>> {
    let mut issues = validate_runner_config(config);
    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return Err(issues);
    }

    let mut router = RouterExecutor::new();
    let mut runtime_execution_types = RuntimeExecutionTypeRegistry::new();
    for (chain, chain_config) in &config.chains {
        match chain_family(chain) {
            ChainFamily::Evm => {
                register_evm_routes(&mut router, chain, chain_config, &mut issues);
            }
            ChainFamily::Solana => {
                register_solana_routes(&mut router, chain, chain_config, &mut issues);
            }
            ChainFamily::External => {}
        }
    }
    register_offchain_plugin_routes(
        &mut router,
        &mut runtime_execution_types,
        config,
        &mut issues,
    );
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
    let router = build_router_executor(config)?;
    for (index, node) in plan.nodes.iter().enumerate() {
        let Some(node_obj) = node.as_object() else {
            continue;
        };
        let Some(chain) = node_obj.get("chain").and_then(Value::as_str) else {
            continue;
        };
        let execution_type = node_obj
            .get("execution")
            .and_then(Value::as_object)
            .and_then(|execution| execution.get("type"))
            .and_then(Value::as_str);
        match execution_type {
            Some(execution_type) => {
                if !router.can_route(chain, execution_type) {
                    issues.push(config_issue(
                        "runner.config.execution_type_unregistered",
                        vec![
                            FieldPathSegment::Key("nodes".to_string()),
                            FieldPathSegment::Index(index),
                            FieldPathSegment::Key("execution".to_string()),
                            FieldPathSegment::Key("type".to_string()),
                        ],
                        format!(
                            "plan node execution.type `{execution_type}` on chain `{chain}` has no registered handler"
                        ),
                    ));
                }
            }
            None => {
                issues.push(config_issue(
                    "runner.config.execution_type_missing",
                    vec![
                        FieldPathSegment::Key("nodes".to_string()),
                        FieldPathSegment::Index(index),
                        FieldPathSegment::Key("execution".to_string()),
                        FieldPathSegment::Key("type".to_string()),
                    ],
                    "plan node must define execution.type".to_string(),
                ));
            }
        }
    }
    if !issues.is_empty() {
        StructuredIssue::sort_stable(&mut issues);
        return Err(issues);
    }
    Ok(router)
}

fn register_evm_routes(
    router: &mut RouterExecutor,
    chain: &str,
    chain_config: &ChainConfig,
    issues: &mut Vec<StructuredIssue>,
) {
    match build_evm_executor(chain, chain_config) {
        Ok(executor) => router.register_core(
            format!("evm-core:{chain}"),
            chain.to_string(),
            execution_types_for_route_preset(ExecutionTypeRoutePreset::EvmCore)
                .iter()
                .copied(),
            Box::new(executor),
        ),
        Err(issue) => issues.push(issue),
    }
    match build_evm_executor(chain, chain_config) {
        Ok(executor) => router.register_plugin(
            format!("evm-plugin:{chain}"),
            chain.to_string(),
            execution_types_for_route_preset(ExecutionTypeRoutePreset::EvmPlugin)
                .iter()
                .copied(),
            Box::new(executor),
        ),
        Err(issue) => issues.push(issue),
    }
}

fn register_solana_routes(
    router: &mut RouterExecutor,
    chain: &str,
    chain_config: &ChainConfig,
    issues: &mut Vec<StructuredIssue>,
) {
    match build_solana_executor(chain, chain_config) {
        Ok(executor) => router.register_core(
            format!("solana-core:{chain}"),
            chain.to_string(),
            execution_types_for_route_preset(ExecutionTypeRoutePreset::SolanaCore)
                .iter()
                .copied(),
            Box::new(executor),
        ),
        Err(issue) => issues.push(issue),
    }
}

fn register_offchain_plugin_routes(
    router: &mut RouterExecutor,
    runtime_execution_types: &mut RuntimeExecutionTypeRegistry,
    config: &RunnerConfig,
    issues: &mut Vec<StructuredIssue>,
) {
    let Some(plugin) = config.plugins.execution.offchain_apy_query.as_ref() else {
        return;
    };
    if !plugin.enabled {
        return;
    }
    runtime_execution_types.register_plugin(
        "offchain_apy_query",
        PluginExecutionTypeCapabilities {
            is_write: false,
            requires_confirm_default: false,
            supports_side_effect_adapter: true,
        },
    );
    let plugin_execution_types = runtime_execution_types.plugin_execution_types();
    for chain in &plugin.chains {
        let Some(chain_config) = config.chains.get(chain) else {
            continue;
        };
        let offchain_config = OffchainApyExecutorConfig {
            chain: chain.clone(),
            allowed_domains: plugin.allowed_domains.clone(),
            timeout_ms: plugin
                .timeout_ms
                .unwrap_or_else(|| chain_config.timeout_ms.unwrap_or(5_000)),
            max_retries: plugin.max_retries.unwrap_or(1),
            retry_backoff_ms: plugin.retry_backoff_ms.unwrap_or(250),
        };
        match OffchainApyExecutor::new(offchain_config) {
            Ok(executor) => router.register_plugin(
                format!("offchain-apy-plugin:{chain}"),
                chain.to_string(),
                plugin_execution_types.iter().map(String::as_str),
                Box::new(executor),
            ),
            Err(error) => issues.push(config_issue(
                "runner.config.offchain_apy_query.init",
                vec![
                    FieldPathSegment::Key("plugins".to_string()),
                    FieldPathSegment::Key("execution".to_string()),
                    FieldPathSegment::Key("offchain_apy_query".to_string()),
                ],
                format!("build offchain_apy_query executor for chain `{chain}` failed: {error}"),
            )),
        }
    }
}

fn build_evm_executor(
    chain: &str,
    chain_config: &ChainConfig,
) -> Result<EvmExecutor, StructuredIssue> {
    let mut endpoint = EvmRpcEndpoint::new(chain.to_string(), chain_config.rpc_url.clone())
        .map_err(|error| {
            chain_issue(
                chain,
                "runner.config.evm.endpoint",
                format!("invalid evm endpoint: {error}"),
            )
        })?;
    if let Some(timeout_ms) = chain_config.timeout_ms {
        endpoint = endpoint.with_timeout_ms(timeout_ms).map_err(|error| {
            chain_issue(
                chain,
                "runner.config.evm.endpoint",
                format!("invalid evm timeout: {error}"),
            )
        })?;
    }
    let registry = EvmProviderRegistry::from_endpoints(vec![endpoint]).map_err(|error| {
        chain_issue(
            chain,
            "runner.config.evm.registry",
            format!("build evm provider registry failed: {error}"),
        )
    })?;
    let mut executor = EvmExecutor::new(registry);
    if let Some(signer_config) = &chain_config.signer {
        let signer = match signer_config {
            SignerConfig::EvmPrivateKey { private_key } => {
                EvmLocalPrivateKeySigner::from_hex(private_key.as_str()).map_err(|error| {
                    chain_issue(
                        chain,
                        "runner.config.evm.signer",
                        format!("invalid evm signer key: {error}"),
                    )
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
    let mut endpoint = SolanaRpcEndpoint::new(chain.to_string(), chain_config.rpc_url.clone())
        .map_err(|error| {
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

fn validate_offchain_plugin_config(
    config: &RunnerConfig,
    configured_chains: &BTreeSet<String>,
    issues: &mut Vec<StructuredIssue>,
) {
    let Some(plugin) = config.plugins.execution.offchain_apy_query.as_ref() else {
        return;
    };
    if !plugin.enabled {
        return;
    }
    if plugin.chains.is_empty() {
        issues.push(config_issue(
            "runner.config.offchain_apy_query.chains.non_empty",
            vec![
                FieldPathSegment::Key("plugins".to_string()),
                FieldPathSegment::Key("execution".to_string()),
                FieldPathSegment::Key("offchain_apy_query".to_string()),
                FieldPathSegment::Key("chains".to_string()),
            ],
            "offchain_apy_query enabled=true requires at least one chain".to_string(),
        ));
    }
    for (index, chain) in plugin.chains.iter().enumerate() {
        if !configured_chains.contains(chain) {
            issues.push(config_issue(
                "runner.config.offchain_apy_query.chain_missing",
                vec![
                    FieldPathSegment::Key("plugins".to_string()),
                    FieldPathSegment::Key("execution".to_string()),
                    FieldPathSegment::Key("offchain_apy_query".to_string()),
                    FieldPathSegment::Key("chains".to_string()),
                    FieldPathSegment::Index(index),
                ],
                format!("offchain_apy_query chain `{chain}` is not configured in chains"),
            ));
        }
    }
    if plugin.allowed_domains.is_empty() {
        issues.push(config_issue(
            "runner.config.offchain_apy_query.allowed_domains.non_empty",
            vec![
                FieldPathSegment::Key("plugins".to_string()),
                FieldPathSegment::Key("execution".to_string()),
                FieldPathSegment::Key("offchain_apy_query".to_string()),
                FieldPathSegment::Key("allowed_domains".to_string()),
            ],
            "offchain_apy_query enabled=true requires non-empty allowed_domains".to_string(),
        ));
    }
    if matches!(plugin.timeout_ms, Some(0)) {
        issues.push(config_issue(
            "runner.config.offchain_apy_query.timeout",
            vec![
                FieldPathSegment::Key("plugins".to_string()),
                FieldPathSegment::Key("execution".to_string()),
                FieldPathSegment::Key("offchain_apy_query".to_string()),
                FieldPathSegment::Key("timeout_ms".to_string()),
            ],
            "offchain_apy_query timeout_ms must be > 0".to_string(),
        ));
    }
    if matches!(plugin.retry_backoff_ms, Some(0)) {
        issues.push(config_issue(
            "runner.config.offchain_apy_query.retry_backoff",
            vec![
                FieldPathSegment::Key("plugins".to_string()),
                FieldPathSegment::Key("execution".to_string()),
                FieldPathSegment::Key("offchain_apy_query".to_string()),
                FieldPathSegment::Key("retry_backoff_ms".to_string()),
            ],
            "offchain_apy_query retry_backoff_ms must be > 0".to_string(),
        ));
    }
}

fn validate_llm_config(config: &RunnerConfig, issues: &mut Vec<StructuredIssue>) {
    let Some(llm) = config.llm.as_ref() else {
        return;
    };
    validate_llm_endpoint(
        llm.provider.as_str(),
        llm.model.as_str(),
        llm.api_key.as_str(),
        vec![FieldPathSegment::Key("llm".to_string())],
        issues,
    );
    if llm
        .controller_prompts_dir
        .as_deref()
        .is_some_and(|path| path.trim().is_empty())
    {
        issues.push(config_issue(
            "runner.config.llm.controller_prompts_dir",
            vec![
                FieldPathSegment::Key("llm".to_string()),
                FieldPathSegment::Key("controller_prompts_dir".to_string()),
            ],
            "llm.controller_prompts_dir must be non-empty when provided".to_string(),
        ));
    }
    if llm
        .operator_templates_dir
        .as_deref()
        .is_some_and(|path| path.trim().is_empty())
    {
        issues.push(config_issue(
            "runner.config.llm.operator_templates_dir",
            vec![
                FieldPathSegment::Key("llm".to_string()),
                FieldPathSegment::Key("operator_templates_dir".to_string()),
            ],
            "llm.operator_templates_dir must be non-empty when provided".to_string(),
        ));
    }
    if matches!(llm.planner_context_token_budget, Some(0)) {
        issues.push(config_issue(
            "runner.config.llm.planner_context_token_budget",
            vec![
                FieldPathSegment::Key("llm".to_string()),
                FieldPathSegment::Key("planner_context_token_budget".to_string()),
            ],
            "llm.planner_context_token_budget must be > 0 when provided".to_string(),
        ));
    }
    if matches!(llm.max_tool_rounds, Some(0)) {
        issues.push(config_issue(
            "runner.config.llm.max_tool_rounds",
            vec![
                FieldPathSegment::Key("llm".to_string()),
                FieldPathSegment::Key("max_tool_rounds".to_string()),
            ],
            "llm.max_tool_rounds must be > 0 when provided".to_string(),
        ));
    }
    if matches!(llm.context_limit_tokens, Some(0)) {
        issues.push(config_issue(
            "runner.config.llm.context_limit_tokens",
            vec![
                FieldPathSegment::Key("llm".to_string()),
                FieldPathSegment::Key("context_limit_tokens".to_string()),
            ],
            "llm.context_limit_tokens must be > 0 when provided".to_string(),
        ));
    }
    for (index, fallback) in llm.fallback.iter().enumerate() {
        validate_llm_endpoint(
            fallback.provider.as_str(),
            fallback.model.as_str(),
            fallback.api_key.as_str(),
            vec![
                FieldPathSegment::Key("llm".to_string()),
                FieldPathSegment::Key("fallback".to_string()),
                FieldPathSegment::Index(index),
            ],
            issues,
        );
    }
}

fn validate_llm_endpoint(
    provider: &str,
    model: &str,
    api_key: &str,
    mut path: Vec<FieldPathSegment>,
    issues: &mut Vec<StructuredIssue>,
) {
    if provider.trim().is_empty() {
        path.push(FieldPathSegment::Key("provider".to_string()));
        issues.push(config_issue(
            "runner.config.llm.provider",
            path.clone(),
            "llm.provider must be non-empty".to_string(),
        ));
        path.pop();
    }
    if model.trim().is_empty() {
        path.push(FieldPathSegment::Key("model".to_string()));
        issues.push(config_issue(
            "runner.config.llm.model",
            path.clone(),
            "llm.model must be non-empty".to_string(),
        ));
        path.pop();
    }
    if api_key.trim().is_empty() {
        path.push(FieldPathSegment::Key("api_key".to_string()));
        issues.push(config_issue(
            "runner.config.llm.api_key",
            path,
            "llm.api_key must be non-empty".to_string(),
        ));
    }
}

fn deserialize_optional_token_count<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum TokenCountValue {
        Number(u64),
        String(String),
    }

    let value = Option::<TokenCountValue>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        TokenCountValue::Number(number) => usize::try_from(number)
            .map(Some)
            .map_err(|_| serde::de::Error::custom("token count is out of range")),
        TokenCountValue::String(text) => parse_token_count(text.as_str())
            .ok_or_else(|| {
                serde::de::Error::custom(
                    "invalid token count string; expected forms like `262k`, `1M`, or `262,144`",
                )
            })
            .map(Some),
    }
}

fn parse_token_count(raw: &str) -> Option<usize> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (digits, multiplier) = match trimmed.chars().last() {
        Some('k') | Some('K') => (&trimmed[..trimmed.len().saturating_sub(1)], 1_000_u128),
        Some('m') | Some('M') => (&trimmed[..trimmed.len().saturating_sub(1)], 1_000_000_u128),
        Some('b') | Some('B') => (
            &trimmed[..trimmed.len().saturating_sub(1)],
            1_000_000_000_u128,
        ),
        _ => (trimmed, 1_u128),
    };
    let normalized = digits.replace([',', '_', ' '], "");
    if normalized.is_empty() || !normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let base = normalized.parse::<u128>().ok()?;
    let total = base.checked_mul(multiplier)?;
    usize::try_from(total).ok()
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

fn collect_env_placeholder_issues(path: &Path, raw: &str) -> Vec<StructuredIssue> {
    let value = match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str::<Value>(raw).ok(),
        Some("yaml") | Some("yml") => serde_yaml::from_str::<Value>(raw).ok(),
        _ => serde_yaml::from_str::<Value>(raw)
            .ok()
            .or_else(|| serde_json::from_str::<Value>(raw).ok()),
    };
    let Some(value) = value else {
        return Vec::new();
    };

    let mut issues = Vec::<StructuredIssue>::new();
    collect_env_placeholder_issues_for_value(&value, Vec::new(), &mut issues);
    issues
}

fn collect_env_placeholder_issues_for_value(
    value: &Value,
    path: Vec<FieldPathSegment>,
    issues: &mut Vec<StructuredIssue>,
) {
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                let mut nested_path = path.clone();
                nested_path.push(FieldPathSegment::Key(key.clone()));
                collect_env_placeholder_issues_for_value(nested, nested_path, issues);
            }
        }
        Value::Array(items) => {
            for (index, nested) in items.iter().enumerate() {
                let mut nested_path = path.clone();
                nested_path.push(FieldPathSegment::Index(index));
                collect_env_placeholder_issues_for_value(nested, nested_path, issues);
            }
        }
        Value::String(text) => {
            issues.extend(
                inspect_env_placeholders(text)
                    .into_iter()
                    .map(|placeholder_issue| {
                        config_issue(
                            placeholder_issue.reference,
                            path.clone(),
                            placeholder_issue.message,
                        )
                    }),
            );
        }
        _ => {}
    }
}

struct EnvPlaceholderIssue {
    reference: &'static str,
    message: String,
}

fn inspect_env_placeholders(input: &str) -> Vec<EnvPlaceholderIssue> {
    let mut issues = Vec::<EnvPlaceholderIssue>::new();
    let mut cursor = 0;
    while let Some(start_offset) = input[cursor..].find("${") {
        let start = cursor + start_offset;
        let var_start = start + 2;
        let Some(end_offset) = input[var_start..].find('}') else {
            issues.push(EnvPlaceholderIssue {
                reference: "runner.config.env_placeholder.syntax",
                message: "unterminated env placeholder `${...`".to_string(),
            });
            break;
        };
        let end = var_start + end_offset;
        let key = &input[var_start..end];
        if key.is_empty() {
            issues.push(EnvPlaceholderIssue {
                reference: "runner.config.env_placeholder.syntax",
                message: "empty env placeholder `${}`".to_string(),
            });
            cursor = end + 1;
            continue;
        }
        if std::env::var(key).is_err() {
            issues.push(EnvPlaceholderIssue {
                reference: "runner.config.env_placeholder.missing",
                message: format!("missing env var for placeholder `${{{key}}}`"),
            });
        }
        cursor = end + 1;
    }
    issues
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainFamily {
    Evm,
    Solana,
    External,
}

fn chain_family(chain: &str) -> ChainFamily {
    if chain.starts_with("eip155:") {
        return ChainFamily::Evm;
    }
    if chain.starts_with("solana:") {
        return ChainFamily::Solana;
    }
    ChainFamily::External
}

fn validate_signer_family(
    chain: &str,
    family: ChainFamily,
    signer: &SignerConfig,
    mut path: Vec<FieldPathSegment>,
    issues: &mut Vec<StructuredIssue>,
) {
    let signer_ok = match (family, signer) {
        (ChainFamily::Evm, SignerConfig::EvmPrivateKey { .. }) => true,
        (ChainFamily::Solana, SignerConfig::SolanaPrivateKey { .. }) => true,
        (ChainFamily::External, _) => true,
        _ => false,
    };
    if signer_ok {
        return;
    }
    path.push(FieldPathSegment::Key("signer".to_string()));
    issues.push(config_issue(
        "runner.config.signer.type_mismatch",
        path,
        format!("signer type does not match chain `{chain}`"),
    ));
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
