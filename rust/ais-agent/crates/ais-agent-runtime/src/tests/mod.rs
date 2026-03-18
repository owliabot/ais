//! Runtime crate tests.

mod checkpoint_repository;
mod concurrency;
mod driver_binding;
mod durable_mutation;
mod evm_binding;
mod evm_live;
mod host_service;
mod mixed_matrix;
mod patch_apply;
mod persistence_contracts;
mod recovery_projection;
mod restart;
mod restore_apply;
mod runtime_repository;
mod scheduler;
mod solana_binding;
mod solana_live;
mod step_once;
pub(crate) mod tracing_capture;
