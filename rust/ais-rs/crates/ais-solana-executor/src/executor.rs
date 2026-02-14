use crate::types::{
    ProviderError, SolanaInstructionAccount, SolanaInstructionRequest, SolanaProviderRegistry,
    SolanaRpcClientFactory,
};
use crate::signer::SolanaTransactionSigner;
use ais_engine::{Executor, ExecutorOutput};
use serde_json::{json, Map, Value};

const SOLANA_READ_ALLOWLIST: &[&str] = &[
    "getBalance",
    "getAccountInfo",
    "getTokenAccountBalance",
    "getSignatureStatuses",
];

pub struct SolanaExecutor {
    providers: SolanaProviderRegistry,
    client_factory: Box<dyn SolanaRpcClientFactory>,
    signer: Option<Box<dyn SolanaTransactionSigner>>,
    instruction_config: SolanaInstructionExecutionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaInstructionExecutionConfig {
    pub wait_for_confirmation: bool,
    pub poll_interval_ms: u64,
    pub max_poll_attempts: u32,
}

impl Default for SolanaInstructionExecutionConfig {
    fn default() -> Self {
        Self {
            wait_for_confirmation: true,
            poll_interval_ms: 1_500,
            max_poll_attempts: 20,
        }
    }
}

impl SolanaExecutor {
    pub fn new(providers: SolanaProviderRegistry, client_factory: Box<dyn SolanaRpcClientFactory>) -> Self {
        Self {
            providers,
            client_factory,
            signer: None,
            instruction_config: SolanaInstructionExecutionConfig::default(),
        }
    }

    pub fn with_signer(mut self, signer: Box<dyn SolanaTransactionSigner>) -> Self {
        self.signer = Some(signer);
        self
    }

    pub fn with_instruction_config(mut self, config: SolanaInstructionExecutionConfig) -> Self {
        self.instruction_config = config;
        self
    }

    pub fn supports(&self, chain: &str, execution_type: &str) -> bool {
        if !chain.starts_with("solana:") {
            return false;
        }
        if self.providers.endpoint(chain).is_err() {
            return false;
        }
        matches!(execution_type, "solana_read" | "solana_instruction")
    }
}

impl Executor for SolanaExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let chain = node
            .as_object()
            .and_then(|object| object.get("chain"))
            .and_then(Value::as_str)
            .ok_or_else(|| "node.chain must be string".to_string())?;
        let execution = node
            .as_object()
            .and_then(|object| object.get("execution"))
            .and_then(Value::as_object)
            .ok_or_else(|| "node.execution must be object".to_string())?;
        let execution_type = execution
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "execution.type must be string".to_string())?;

        if !self.supports(chain, execution_type) {
            return Err(format!(
                "solana executor does not support chain `{chain}` + execution type `{execution_type}`"
            ));
        }

        match execution_type {
            "solana_read" => self.execute_solana_read(chain, execution),
            "solana_instruction" => self.execute_solana_instruction(chain, execution),
            other => Err(format!("unsupported execution type for solana executor: {other}")),
        }
    }
}

impl SolanaExecutor {
    fn execute_solana_read(
        &self,
        chain: &str,
        execution: &Map<String, Value>,
    ) -> Result<ExecutorOutput, String> {
        let method = execution
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| "solana_read.method must be string".to_string())?;
        if !SOLANA_READ_ALLOWLIST.contains(&method) {
            return Err(format!(
                "solana_read method not allowed: `{method}` (allowlist={})",
                SOLANA_READ_ALLOWLIST.join(",")
            ));
        }
        let params = execution.get("params").map(lit_or_value).cloned().unwrap_or(Value::Null);
        let client = self
            .providers
            .build_client_for_chain(chain, self.client_factory.as_ref())
            .map_err(|error| format!("provider unavailable: {error}"))?;

        let result = match method {
            "getBalance" => {
                let pubkey = first_param_as_str(&params, "getBalance")?;
                json!(client.get_balance(pubkey).map_err(provider_to_string)?)
            }
            "getAccountInfo" => {
                let pubkey = first_param_as_str(&params, "getAccountInfo")?;
                client.get_account_info(pubkey).map_err(provider_to_string)?
            }
            "getTokenAccountBalance" => {
                let pubkey = first_param_as_str(&params, "getTokenAccountBalance")?;
                client
                    .get_token_account_balance(pubkey)
                    .map_err(provider_to_string)?
            }
            "getSignatureStatuses" => {
                let signatures = params
                    .as_array()
                    .ok_or_else(|| "getSignatureStatuses params must be array".to_string())?
                    .iter()
                    .map(lit_or_value)
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .ok_or_else(|| "getSignatureStatuses signatures must be strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                client
                    .get_signature_statuses(&signatures)
                    .map_err(provider_to_string)?
            }
            _ => unreachable!("allowlist already checked"),
        };

        Ok(ExecutorOutput {
            result: json!({
                "execution_type": "solana_read",
                "chain": chain,
                "method": method,
                "result": result,
            }),
            writes: Map::new(),
        })
    }

    fn execute_solana_instruction(
        &self,
        chain: &str,
        execution: &Map<String, Value>,
    ) -> Result<ExecutorOutput, String> {
        let tx_version = execution
            .get("tx_version")
            .map(lit_or_value)
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if execution.get("lookup_tables").is_some() {
                    "v0"
                } else {
                    "legacy"
                }
            })
            .to_string();
        let program = value_or_lit_as_str(execution, "program")?.to_string();
        let instruction = execution
            .get("instruction")
            .and_then(Value::as_str)
            .ok_or_else(|| "solana_instruction.instruction must be string".to_string())?
            .to_string();
        let data = value_or_lit_as_str(execution, "data")?.to_string();
        let accounts = parse_instruction_accounts(execution)?;
        let compute_units = execution
            .get("compute_units")
            .map(lit_or_value)
            .map(as_u64)
            .transpose()?;
        let lookup_tables = execution.get("lookup_tables").map(lit_or_value).cloned();

        let request = SolanaInstructionRequest {
            tx_version: tx_version.clone(),
            program,
            instruction: instruction.clone(),
            accounts,
            data,
            compute_units,
            lookup_tables,
        };

        if request.lookup_tables.is_some() && request.tx_version != "v0" {
            return Err("solana_instruction with lookup_tables must use tx_version=v0".to_string());
        }

        let Some(signer) = self.signer.as_ref() else {
            return Err(format!(
                "need_user_confirm: missing signer for solana_instruction summary={}",
                json!({
                    "chain": chain,
                    "tx_version": request.tx_version,
                    "program": request.program,
                    "instruction": request.instruction,
                    "accounts": request.accounts,
                    "lookup_tables": request.lookup_tables,
                })
            ));
        };

        let signed = signer
            .sign_instruction(&request)
            .map_err(|error| format!("sign solana instruction failed: {error}"))?;
        let client = self
            .providers
            .build_client_for_chain(chain, self.client_factory.as_ref())
            .map_err(|error| format!("provider unavailable: {error}"))?;
        let signature = client
            .send_signed_transaction(&request, &signed.raw_tx)
            .map_err(provider_to_string)?;

        let confirmation_status = if self.instruction_config.wait_for_confirmation {
            wait_for_confirmation(
                client.as_ref(),
                &signature,
                self.instruction_config.max_poll_attempts,
                self.instruction_config.poll_interval_ms,
            )?
        } else {
            None
        };

        Ok(ExecutorOutput {
            result: json!({
                "execution_type": "solana_instruction",
                "chain": chain,
                "tx_version": tx_version,
                "instruction": instruction,
                "signature": signature,
                "signed_tx_hash": signed.tx_hash,
                "confirmation_status": confirmation_status,
            }),
            writes: Map::new(),
        })
    }
}

fn parse_instruction_accounts(
    execution: &Map<String, Value>,
) -> Result<Vec<SolanaInstructionAccount>, String> {
    let accounts = execution
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| "solana_instruction.accounts must be array".to_string())?;
    accounts
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| "solana_instruction account item must be object".to_string())?;
            Ok(SolanaInstructionAccount {
                name: object
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "account.name must be string".to_string())?
                    .to_string(),
                pubkey: value_or_lit_as_str(object, "pubkey")?.to_string(),
                signer: as_bool(lit_or_value(
                    object
                        .get("signer")
                        .ok_or_else(|| "account.signer must exist".to_string())?,
                ))?,
                writable: as_bool(lit_or_value(
                    object
                        .get("writable")
                        .ok_or_else(|| "account.writable must exist".to_string())?,
                ))?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
}

fn value_or_lit_as_str<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
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

fn lit_or_value(value: &Value) -> &Value {
    value
        .as_object()
        .and_then(|object| object.get("lit"))
        .unwrap_or(value)
}

fn first_param_as_str<'a>(params: &'a Value, method: &str) -> Result<&'a str, String> {
    params
        .as_array()
        .and_then(|items| items.first())
        .map(lit_or_value)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{method} params[0] must be string"))
}

fn as_bool(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "expected boolean value".to_string())
}

fn as_u64(value: &Value) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| "expected u64 value".to_string())
}

fn provider_to_string(error: ProviderError) -> String {
    format!("solana rpc failed: {error}")
}

fn wait_for_confirmation(
    client: &dyn crate::types::SolanaRpcClient,
    signature: &str,
    max_attempts: u32,
    poll_interval_ms: u64,
) -> Result<Option<Value>, String> {
    let _ = poll_interval_ms;
    for _ in 0..max_attempts {
        let statuses = client
            .get_signature_statuses(&[signature.to_string()])
            .map_err(provider_to_string)?;
        if let Some(items) = statuses.as_array() {
            if let Some(item) = items.first() {
                if !item.is_null() {
                    return Ok(Some(item.clone()));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "executor_test.rs"]
mod tests;
