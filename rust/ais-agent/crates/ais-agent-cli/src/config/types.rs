use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentServiceConfig {
    pub service: AisAgentServiceIdentityConfig,
    pub transport: AisAgentTransportConfig,
    pub storage: AisAgentStorageConfig,
    pub providers: AisAgentProviderConfig,
    pub runtime_defaults: AisAgentRuntimeDefaultsConfig,
    pub observability: AisAgentObservabilityConfig,
}

impl Default for AisAgentServiceConfig {
    fn default() -> Self {
        Self {
            service: AisAgentServiceIdentityConfig::default(),
            transport: AisAgentTransportConfig::default(),
            storage: AisAgentStorageConfig::default(),
            providers: AisAgentProviderConfig::default(),
            runtime_defaults: AisAgentRuntimeDefaultsConfig::default(),
            observability: AisAgentObservabilityConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentServiceIdentityConfig {
    pub instance_id: String,
}

impl Default for AisAgentServiceIdentityConfig {
    fn default() -> Self {
        Self {
            instance_id: "ais-agent".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentTransportConfig {
    pub http: AisAgentHttpTransportConfig,
    pub jsonl: AisAgentJsonlTransportConfig,
}

impl Default for AisAgentTransportConfig {
    fn default() -> Self {
        Self {
            http: AisAgentHttpTransportConfig::default(),
            jsonl: AisAgentJsonlTransportConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentHttpTransportConfig {
    pub enabled: bool,
    pub bind: String,
}

impl Default for AisAgentHttpTransportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1:3000".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentJsonlTransportConfig {
    pub enabled: bool,
}

impl Default for AisAgentJsonlTransportConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentFileLoggingConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub retention_days: u16,
}

impl Default for AisAgentFileLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: PathBuf::from("./var/log/ais-agent"),
            retention_days: 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentJsonlCaptureConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub retention_days: u16,
}

impl Default for AisAgentJsonlCaptureConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: PathBuf::from("./var/captures/jsonl"),
            retention_days: 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum AisAgentStorageConfig {
    InMemory,
    Sqlite(AisAgentSqliteStorageConfig),
}

impl Default for AisAgentStorageConfig {
    fn default() -> Self {
        Self::InMemory
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentSqliteRetentionConfig {
    pub checkpoint_full_window_days: u16,
    pub checkpoint_boundary_only_window_days: u16,
    pub wait_state_orphan_ttl_days: u16,
    pub destructive_purge_enabled: bool,
    pub require_purge_confirmation: bool,
    pub auto_prune_cadence_minutes: u32,
    pub vacuum_freelist_threshold_pages: u32,
}

impl Default for AisAgentSqliteRetentionConfig {
    fn default() -> Self {
        Self {
            checkpoint_full_window_days: 7,
            checkpoint_boundary_only_window_days: 30,
            wait_state_orphan_ttl_days: 7,
            destructive_purge_enabled: false,
            require_purge_confirmation: true,
            auto_prune_cadence_minutes: 24 * 60,
            vacuum_freelist_threshold_pages: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentSqliteStorageConfig {
    pub path: PathBuf,
    pub create_if_missing: bool,
    pub retention: AisAgentSqliteRetentionConfig,
}

impl Default for AisAgentSqliteStorageConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./var/ais-agent.db"),
            create_if_missing: true,
            retention: AisAgentSqliteRetentionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AisAgentProviderConfig {
    pub evm_rpc_url: Option<String>,
    pub solana_rpc_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentRuntimeDefaultsConfig {
    pub claim_lease_seconds: u64,
    pub step_wall_clock_ms: u64,
    pub confirmation_poll_ms: u64,
}

impl Default for AisAgentRuntimeDefaultsConfig {
    fn default() -> Self {
        Self {
            claim_lease_seconds: 60,
            step_wall_clock_ms: 10_000,
            confirmation_poll_ms: 2_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AisAgentLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Default for AisAgentLogLevel {
    fn default() -> Self {
        Self::Info
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AisAgentObservabilityConfig {
    pub log_level: AisAgentLogLevel,
    pub file_logging: AisAgentFileLoggingConfig,
    pub jsonl_capture: AisAgentJsonlCaptureConfig,
}

impl Default for AisAgentObservabilityConfig {
    fn default() -> Self {
        Self {
            log_level: AisAgentLogLevel::Info,
            file_logging: AisAgentFileLoggingConfig::default(),
            jsonl_capture: AisAgentJsonlCaptureConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AisAgentCliServiceConfig {
    pub config_path: Option<PathBuf>,
}
