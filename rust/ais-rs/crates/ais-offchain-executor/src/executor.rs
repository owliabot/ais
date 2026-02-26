use ais_engine::{
    canonical_side_effect_status, is_pending_side_effect_status, CheckpointSideEffectRecord,
    Executor, ExecutorOutput, SIDE_EFFECT_RECORD_SCHEMA_0_1_0, SIDE_EFFECT_STATUS_SENT,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::thread;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffchainApyExecutorConfig {
    pub chain: String,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
}

fn default_timeout_ms() -> u64 {
    5_000
}

fn default_retry_backoff_ms() -> u64 {
    250
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffchainApyHttpRequest {
    pub method: String,
    pub endpoint: String,
    pub args: Option<Value>,
    pub timeout_ms: u64,
}

pub trait OffchainHttpClient: Send + Sync {
    fn send_json(&self, request: &OffchainApyHttpRequest) -> Result<Value, String>;
}

pub struct ReqwestOffchainHttpClient {
    client: reqwest::blocking::Client,
}

impl ReqwestOffchainHttpClient {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .map_err(|error| format!("build reqwest client failed: {error}"))?;
        Ok(Self { client })
    }
}

impl OffchainHttpClient for ReqwestOffchainHttpClient {
    fn send_json(&self, request: &OffchainApyHttpRequest) -> Result<Value, String> {
        let method = request.method.to_ascii_uppercase();
        let timeout = Duration::from_millis(request.timeout_ms);
        let mut req = match method.as_str() {
            "GET" => self.client.get(request.endpoint.as_str()).timeout(timeout),
            "POST" => self.client.post(request.endpoint.as_str()).timeout(timeout),
            other => {
                return Err(format!(
                    "offchain_apy_query.method must be GET|POST, got `{other}`"
                ))
            }
        };
        if method == "POST" {
            req = req.json(&request.args.clone().unwrap_or(Value::Null));
        }
        let response = req
            .send()
            .map_err(|error| format!("offchain request failed: {error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .map_err(|error| format!("offchain response decode failed: {error}"))?;
        if !status.is_success() {
            return Err(format!("offchain response status {status}: {body}"));
        }
        Ok(body)
    }
}

pub struct OffchainApyExecutor {
    config: OffchainApyExecutorConfig,
    client: Box<dyn OffchainHttpClient>,
}

impl OffchainApyExecutor {
    pub fn new(config: OffchainApyExecutorConfig) -> Result<Self, String> {
        let client = ReqwestOffchainHttpClient::new()?;
        Ok(Self {
            config,
            client: Box::new(client),
        })
    }

    pub fn with_client(
        config: OffchainApyExecutorConfig,
        client: Box<dyn OffchainHttpClient>,
    ) -> Self {
        Self { config, client }
    }

    fn supports(&self, chain: &str, execution_type: &str) -> bool {
        chain == self.config.chain && execution_type == "offchain_apy_query"
    }

    fn validate_endpoint(&self, endpoint: &str) -> Result<(), String> {
        let parsed =
            Url::parse(endpoint).map_err(|error| format!("invalid endpoint url: {error}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "endpoint url must include host".to_string())?;
        if self.config.allowed_domains.is_empty() {
            return Err("offchain_apy_query allowed_domains must not be empty".to_string());
        }
        let allowed = self
            .config
            .allowed_domains
            .iter()
            .any(|rule| domain_rule_matches(rule.as_str(), host));
        if !allowed {
            return Err(format!(
                "endpoint host `{host}` is not in allowlist [{}]",
                self.config.allowed_domains.join(",")
            ));
        }
        Ok(())
    }
}

impl Executor for OffchainApyExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_obj = node
            .as_object()
            .ok_or_else(|| "node must be object".to_string())?;
        let node_id = node_obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "node.id must be string".to_string())?;
        let chain = node_obj
            .get("chain")
            .and_then(Value::as_str)
            .ok_or_else(|| "node.chain must be string".to_string())?;
        let execution = node_obj
            .get("execution")
            .and_then(Value::as_object)
            .ok_or_else(|| "node.execution must be object".to_string())?;
        let execution_type = execution
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "execution.type must be string".to_string())?;
        if !self.supports(chain, execution_type) {
            return Err(format!(
                "offchain executor does not support chain `{chain}` + execution type `{execution_type}`"
            ));
        }

        let endpoint = value_or_lit_as_str(execution, "endpoint")?.to_string();
        self.validate_endpoint(endpoint.as_str())?;
        let method = execution
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_string();
        let args = execution.get("args").map(lit_or_value).cloned();
        let timeout_ms = execution
            .get("timeout_ms")
            .map(lit_or_value)
            .and_then(Value::as_u64)
            .unwrap_or(self.config.timeout_ms);

        let request = OffchainApyHttpRequest {
            method,
            endpoint: endpoint.clone(),
            args,
            timeout_ms,
        };

        let mut last_error = String::new();
        for attempt in 0..=self.config.max_retries {
            match self.client.send_json(&request) {
                Ok(body) => {
                    let outputs = normalize_outputs(body);
                    let side_effects = extract_side_effects(
                        node_id,
                        chain,
                        execution_type,
                        &outputs,
                        "1970-01-01T00:00:00Z",
                    );
                    let mut result = Map::<String, Value>::new();
                    result.insert(
                        "execution_type".to_string(),
                        Value::String("offchain_apy_query".to_string()),
                    );
                    result.insert("chain".to_string(), Value::String(chain.to_string()));
                    result.insert("endpoint".to_string(), Value::String(endpoint.clone()));
                    result.insert(
                        "attempt".to_string(),
                        Value::Number((attempt as u64 + 1).into()),
                    );
                    result.insert("outputs".to_string(), outputs);
                    return Ok(ExecutorOutput {
                        result: Value::Object(result),
                        writes: Map::new(),
                        side_effects,
                    });
                }
                Err(error) => {
                    last_error = error;
                    if attempt < self.config.max_retries {
                        thread::sleep(Duration::from_millis(self.config.retry_backoff_ms));
                    }
                }
            }
        }
        Err(format!(
            "offchain_apy_query failed after {} attempt(s): {last_error}",
            self.config.max_retries + 1
        ))
    }

    fn reconcile_side_effect(
        &self,
        record: &CheckpointSideEffectRecord,
    ) -> Result<Option<CheckpointSideEffectRecord>, String> {
        if record.execution_type.as_deref() != Some("offchain_apy_query") {
            return Ok(None);
        }
        let Some(chain) = record.chain.as_deref() else {
            return Ok(None);
        };
        if !self.supports(chain, "offchain_apy_query") {
            return Ok(None);
        }
        if !is_pending_side_effect_status(record.status.as_str()) {
            return Ok(Some(record.clone()));
        }

        let mut updated = record.clone();
        updated.observed_at = "1970-01-01T00:00:00Z".to_string();
        updated.reason_code = Some("side_effect_reconcile_not_supported".to_string());
        Ok(Some(updated))
    }
}

fn extract_side_effects(
    node_id: &str,
    chain: &str,
    execution_type: &str,
    outputs: &Value,
    observed_at: &str,
) -> Vec<CheckpointSideEffectRecord> {
    let Some(items) = outputs.get("side_effects").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_object)
        .enumerate()
        .map(|(index, item)| {
            let effect_type = item
                .get("effect_type")
                .and_then(Value::as_str)
                .unwrap_or("tx")
                .to_string();
            let effect_node_id = item
                .get("node_id")
                .and_then(Value::as_str)
                .unwrap_or(node_id)
                .to_string();
            let tx_hash = item
                .get("tx_hash")
                .and_then(Value::as_str)
                .map(str::to_string);
            let idempotency_key = item
                .get("idempotency_key")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    tx_hash.as_ref().map_or_else(
                        || format!("{effect_type}:{effect_node_id}:{index}"),
                        |hash| format!("{effect_type}:{effect_node_id}:{hash}"),
                    )
                });
            let raw_status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or(SIDE_EFFECT_STATUS_SENT);
            CheckpointSideEffectRecord {
                schema: Some(SIDE_EFFECT_RECORD_SCHEMA_0_1_0.to_string()),
                idempotency_key,
                node_id: effect_node_id,
                effect_type,
                chain: Some(
                    item.get("chain")
                        .and_then(Value::as_str)
                        .unwrap_or(chain)
                        .to_string(),
                ),
                execution_type: Some(
                    item.get("execution_type")
                        .and_then(Value::as_str)
                        .unwrap_or(execution_type)
                        .to_string(),
                ),
                tx_hash,
                nonce: item.get("nonce").and_then(Value::as_u64),
                provider_ref: item
                    .get("provider_ref")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                reason_code: item
                    .get("reason_code")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                details: item.get("details").cloned(),
                status: canonical_side_effect_status(raw_status).to_string(),
                observed_at: item
                    .get("observed_at")
                    .and_then(Value::as_str)
                    .unwrap_or(observed_at)
                    .to_string(),
            }
        })
        .collect()
}

fn lit_or_value(value: &Value) -> &Value {
    value
        .as_object()
        .and_then(|object| object.get("lit"))
        .unwrap_or(value)
}

fn value_or_lit_as_str<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .ok_or_else(|| format!("missing field `{key}`"))?;
    if let Some(text) = value.as_str() {
        return Ok(text);
    }
    value
        .as_object()
        .and_then(|obj| obj.get("lit"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("field `{key}` must be string or {{lit: string}}"))
}

fn normalize_outputs(body: Value) -> Value {
    match body {
        Value::Object(map) => Value::Object(map),
        other => json!({ "value": other }),
    }
}

fn domain_rule_matches(rule: &str, host: &str) -> bool {
    let normalized = rule.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    let host = host.to_ascii_lowercase();
    if let Some(suffix) = normalized.strip_prefix("*.") {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }
    host == normalized
}

#[cfg(test)]
#[path = "executor_test.rs"]
mod tests;
