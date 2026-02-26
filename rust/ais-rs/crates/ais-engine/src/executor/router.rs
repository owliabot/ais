use crate::checkpoint::CheckpointSideEffectRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub trait Executor {
    fn execute(&self, node: &Value, runtime: &mut Value) -> Result<ExecutorOutput, String>;

    fn reconcile_side_effect(
        &self,
        _record: &CheckpointSideEffectRecord,
    ) -> Result<Option<CheckpointSideEffectRecord>, String> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutorOutput {
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub writes: Map<String, Value>,
    #[serde(default)]
    pub side_effects: Vec<CheckpointSideEffectRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionHandlerKind {
    Core,
    Plugin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterExecutorRegistration {
    pub name: String,
    pub chain: String,
    pub kind: ExecutionHandlerKind,
    pub execution_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterExecuteResult {
    pub executor_name: String,
    pub chain: String,
    pub output: ExecutorOutput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterReconcileResult {
    pub executor_name: String,
    pub chain: String,
    pub record: Option<CheckpointSideEffectRecord>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouterExecuteError {
    #[error("node must include string `id`")]
    MissingNodeId,
    #[error("node `{node_id}` must include string `chain`")]
    MissingNodeChain { node_id: String },
    #[error("node `{node_id}` must include string `execution.type`")]
    MissingExecutionType { node_id: String },
    #[error("chain mismatch for node `{node_id}`: `{chain}` has no registered executor")]
    ChainMismatch { node_id: String, chain: String },
    #[error(
        "unregistered execution type for node `{node_id}`: chain `{chain}` has no handler for `{execution_type}`"
    )]
    UnregisteredExecutionType {
        node_id: String,
        chain: String,
        execution_type: String,
    },
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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouterReconcileError {
    #[error("side-effect `{node_id}` missing `chain`")]
    MissingChain { node_id: String },
    #[error("side-effect `{node_id}` missing `execution_type`")]
    MissingExecutionType { node_id: String },
    #[error(
        "side-effect route chain mismatch for node `{node_id}`: `{chain}` has no registered executor"
    )]
    ChainMismatch { node_id: String, chain: String },
    #[error(
        "side-effect route unregistered execution type for node `{node_id}`: chain `{chain}` has no handler for `{execution_type}`"
    )]
    UnregisteredExecutionType {
        node_id: String,
        chain: String,
        execution_type: String,
    },
    #[error("side-effect route ambiguous for node `{node_id}`: chain `{chain}` matched multiple executors [{executors}]")]
    AmbiguousRoute {
        node_id: String,
        chain: String,
        executors: String,
    },
    #[error("executor `{executor}` reconcile failed for node `{node_id}`: {reason}")]
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
        self.register_core(name, chain, ["*"], executor);
    }

    pub fn register_core(
        &mut self,
        name: impl Into<String>,
        chain: impl Into<String>,
        execution_types: impl IntoIterator<Item = impl Into<String>>,
        executor: Box<dyn Executor>,
    ) {
        self.register_with_kind(
            ExecutionHandlerKind::Core,
            name,
            chain,
            execution_types,
            executor,
        );
    }

    pub fn register_plugin(
        &mut self,
        name: impl Into<String>,
        chain: impl Into<String>,
        execution_types: impl IntoIterator<Item = impl Into<String>>,
        executor: Box<dyn Executor>,
    ) {
        self.register_with_kind(
            ExecutionHandlerKind::Plugin,
            name,
            chain,
            execution_types,
            executor,
        );
    }

    fn register_with_kind(
        &mut self,
        kind: ExecutionHandlerKind,
        name: impl Into<String>,
        chain: impl Into<String>,
        execution_types: impl IntoIterator<Item = impl Into<String>>,
        executor: Box<dyn Executor>,
    ) {
        let execution_types = normalize_execution_types(execution_types);
        self.registrations.push(RouterExecutorRegistration {
            name: name.into(),
            chain: chain.into(),
            kind,
            execution_types,
        });
        self.executors.push(executor);
    }

    pub fn registrations(&self) -> &[RouterExecutorRegistration] {
        &self.registrations
    }

    pub fn can_route(&self, chain: &str, execution_type: &str) -> bool {
        self.registrations.iter().any(|registration| {
            registration.chain == chain
                && registration_supports_type(
                    registration.execution_types.as_slice(),
                    execution_type,
                )
        })
    }

    pub fn registered_execution_types(&self) -> Vec<String> {
        let mut out = self
            .registrations
            .iter()
            .flat_map(|registration| registration.execution_types.iter().cloned())
            .filter(|execution_type| execution_type != "*")
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        out.sort();
        out
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
        let execution_type = node
            .as_object()
            .and_then(|object| object.get("execution"))
            .and_then(Value::as_object)
            .and_then(|execution| execution.get("type"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| RouterExecuteError::MissingExecutionType {
                node_id: node_id.clone(),
            })?;

        let matched_indexes = self
            .registrations
            .iter()
            .enumerate()
            .filter(|(_, registration)| {
                registration.chain == chain
                    && registration_supports_type(
                        registration.execution_types.as_slice(),
                        execution_type.as_str(),
                    )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        if matched_indexes.is_empty() {
            let has_chain = self
                .registrations
                .iter()
                .any(|registration| registration.chain == chain);
            if has_chain {
                return Err(RouterExecuteError::UnregisteredExecutionType {
                    node_id,
                    chain,
                    execution_type,
                });
            }
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

        let output = executor.execute(node, runtime).map_err(|reason| {
            RouterExecuteError::ExecutorFailed {
                executor: registration.name.clone(),
                node_id: node_id.clone(),
                reason,
            }
        })?;

        Ok(RouterExecuteResult {
            executor_name: registration.name.clone(),
            chain: chain.clone(),
            output,
        })
    }

    pub fn reconcile_side_effect(
        &self,
        record: &CheckpointSideEffectRecord,
    ) -> Result<RouterReconcileResult, RouterReconcileError> {
        let node_id = record.node_id.clone();
        let chain = record.chain.as_deref().map(str::to_string).ok_or_else(|| {
            RouterReconcileError::MissingChain {
                node_id: node_id.clone(),
            }
        })?;
        let execution_type = record
            .execution_type
            .as_deref()
            .map(str::to_string)
            .ok_or_else(|| RouterReconcileError::MissingExecutionType {
                node_id: node_id.clone(),
            })?;

        let matched_indexes = self
            .registrations
            .iter()
            .enumerate()
            .filter(|(_, registration)| {
                registration.chain == chain
                    && registration_supports_type(
                        registration.execution_types.as_slice(),
                        execution_type.as_str(),
                    )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();

        if matched_indexes.is_empty() {
            let has_chain = self
                .registrations
                .iter()
                .any(|registration| registration.chain == chain);
            if has_chain {
                return Err(RouterReconcileError::UnregisteredExecutionType {
                    node_id,
                    chain,
                    execution_type,
                });
            }
            return Err(RouterReconcileError::ChainMismatch { node_id, chain });
        }
        if matched_indexes.len() > 1 {
            let executors = matched_indexes
                .iter()
                .map(|index| self.registrations[*index].name.clone())
                .collect::<Vec<_>>()
                .join(",");
            return Err(RouterReconcileError::AmbiguousRoute {
                node_id,
                chain,
                executors,
            });
        }

        let matched_index = matched_indexes[0];
        let registration = &self.registrations[matched_index];
        let executor = &self.executors[matched_index];
        let reconciled = executor.reconcile_side_effect(record).map_err(|reason| {
            RouterReconcileError::ExecutorFailed {
                executor: registration.name.clone(),
                node_id: record.node_id.clone(),
                reason,
            }
        })?;

        Ok(RouterReconcileResult {
            executor_name: registration.name.clone(),
            chain,
            record: reconciled,
        })
    }
}

fn normalize_execution_types(
    execution_types: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<String> {
    let mut out = execution_types
        .into_iter()
        .map(Into::into)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if out.is_empty() {
        out.push("*".to_string());
    }
    out
}

fn registration_supports_type(registered: &[String], execution_type: &str) -> bool {
    registered
        .iter()
        .any(|value| value == "*" || value == execution_type)
}

impl Default for RouterExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "router_test.rs"]
mod tests;
