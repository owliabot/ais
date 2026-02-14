use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub trait Executor {
    fn execute(&self, node: &Value, runtime: &mut Value) -> Result<ExecutorOutput, String>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutorOutput {
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub writes: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterExecutorRegistration {
    pub name: String,
    pub chain: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterExecuteResult {
    pub executor_name: String,
    pub chain: String,
    pub output: ExecutorOutput,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouterExecuteError {
    #[error("node must include string `id`")]
    MissingNodeId,
    #[error("node `{node_id}` must include string `chain`")]
    MissingNodeChain { node_id: String },
    #[error("chain mismatch for node `{node_id}`: `{chain}` has no registered executor")]
    ChainMismatch { node_id: String, chain: String },
    #[error("ambiguous route for node `{node_id}`: chain `{chain}` matched multiple executors [{executors}]")]
    AmbiguousRoute {
        node_id: String,
        chain: String,
        executors: String,
    },
    #[error("executor `{executor}` failed for node `{node_id}`: {reason}")]
    ExecutorFailed {
        executor: String,
        node_id: String,
        reason: String,
    },
}

pub struct RouterExecutor {
    registrations: Vec<RouterExecutorRegistration>,
    executors: Vec<Box<dyn Executor>>,
}

impl RouterExecutor {
    pub fn new() -> Self {
        Self {
            registrations: Vec::new(),
            executors: Vec::new(),
        }
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        chain: impl Into<String>,
        executor: Box<dyn Executor>,
    ) {
        self.registrations.push(RouterExecutorRegistration {
            name: name.into(),
            chain: chain.into(),
        });
        self.executors.push(executor);
    }

    pub fn registrations(&self) -> &[RouterExecutorRegistration] {
        &self.registrations
    }

    pub fn execute(
        &self,
        node: &Value,
        runtime: &mut Value,
    ) -> Result<RouterExecuteResult, RouterExecuteError> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(RouterExecuteError::MissingNodeId)?;
        let chain = node
            .as_object()
            .and_then(|object| object.get("chain"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| RouterExecuteError::MissingNodeChain {
                node_id: node_id.clone(),
            })?;

        let matched_indexes = self
            .registrations
            .iter()
            .enumerate()
            .filter(|(_, registration)| registration.chain == chain)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        if matched_indexes.is_empty() {
            return Err(RouterExecuteError::ChainMismatch { node_id, chain });
        }
        if matched_indexes.len() > 1 {
            let executors = matched_indexes
                .iter()
                .map(|index| self.registrations[*index].name.clone())
                .collect::<Vec<_>>()
                .join(",");
            return Err(RouterExecuteError::AmbiguousRoute {
                node_id,
                chain,
                executors,
            });
        }

        let matched_index = matched_indexes[0];
        let registration = &self.registrations[matched_index];
        let executor = &self.executors[matched_index];

        let output = executor
            .execute(node, runtime)
            .map_err(|reason| RouterExecuteError::ExecutorFailed {
                executor: registration.name.clone(),
                node_id: node_id.clone(),
                reason,
            })?;

        Ok(RouterExecuteResult {
            executor_name: registration.name.clone(),
            chain: chain.clone(),
            output,
        })
    }
}

impl Default for RouterExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "router_test.rs"]
mod tests;
