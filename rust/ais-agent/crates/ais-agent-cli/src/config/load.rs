use std::{env, fs, path::PathBuf};

use thiserror::Error;

use crate::cli::args::{Args, CliCommand, DaemonCommand, LocalCommand, LogLevelArg};

use super::types::{
    AisAgentCliServiceConfig, AisAgentLogLevel, AisAgentServiceConfig, AisAgentSqliteStorageConfig,
    AisAgentStorageConfig,
};

#[derive(Debug, Error)]
pub enum ServiceConfigError {
    #[error("service config file not found: {0}")]
    ConfigFileNotFound(String),
    #[error("service config file is invalid YAML: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
    #[error("service config file could not be read: {0}")]
    ReadFailed(#[from] std::io::Error),
    #[error("service config is invalid: {0}")]
    Invalid(String),
}

pub fn load_service_config(args: &Args) -> Result<AisAgentServiceConfig, ServiceConfigError> {
    let cli = AisAgentCliServiceConfig {
        config_path: args.config.as_ref().map(PathBuf::from),
    };
    let mut config = if let Some(path) = cli.config_path.as_ref() {
        load_config_file(path)?
    } else {
        AisAgentServiceConfig::default()
    };

    apply_env_overrides(&mut config);
    apply_cli_overrides(&mut config, args);
    apply_command_defaults(&mut config, &args.command);
    validate_config(&config)?;

    Ok(config)
}

fn load_config_file(path: &PathBuf) -> Result<AisAgentServiceConfig, ServiceConfigError> {
    if !path.exists() {
        return Err(ServiceConfigError::ConfigFileNotFound(
            path.display().to_string(),
        ));
    }

    let content = fs::read_to_string(path)?;
    Ok(serde_yaml::from_str::<AisAgentServiceConfig>(&content)?)
}

fn apply_env_overrides(config: &mut AisAgentServiceConfig) {
    if let Ok(bind) = env::var("AIS_AGENT_HTTP_BIND") {
        config.transport.http.bind = bind;
    }
    if let Ok(sqlite_path) = env::var("AIS_AGENT_SQLITE_PATH") {
        let mut sqlite = sqlite_storage_config(config);
        sqlite.path = PathBuf::from(sqlite_path);
        config.storage = AisAgentStorageConfig::Sqlite(sqlite);
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_CHECKPOINT_FULL_WINDOW_DAYS") {
        if let Ok(days) = value.parse::<u16>() {
            sqlite_storage_config_mut(config)
                .retention
                .checkpoint_full_window_days = days;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_CHECKPOINT_BOUNDARY_ONLY_WINDOW_DAYS") {
        if let Ok(days) = value.parse::<u16>() {
            sqlite_storage_config_mut(config)
                .retention
                .checkpoint_boundary_only_window_days = days;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_WAIT_STATE_ORPHAN_TTL_DAYS") {
        if let Ok(days) = value.parse::<u16>() {
            sqlite_storage_config_mut(config)
                .retention
                .wait_state_orphan_ttl_days = days;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_PURGE_ENABLED") {
        if let Some(enabled) = parse_bool_flag(&value) {
            sqlite_storage_config_mut(config)
                .retention
                .destructive_purge_enabled = enabled;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_PURGE_REQUIRE_CONFIRMATION") {
        if let Some(enabled) = parse_bool_flag(&value) {
            sqlite_storage_config_mut(config)
                .retention
                .require_purge_confirmation = enabled;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_AUTO_PRUNE_CADENCE_MINUTES") {
        if let Ok(minutes) = value.parse::<u32>() {
            sqlite_storage_config_mut(config)
                .retention
                .auto_prune_cadence_minutes = minutes;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_SQLITE_VACUUM_FREELIST_THRESHOLD_PAGES") {
        if let Ok(pages) = value.parse::<u32>() {
            sqlite_storage_config_mut(config)
                .retention
                .vacuum_freelist_threshold_pages = pages;
        }
    }
    if let Ok(value) = env::var("AIS_AGENT_CLAIM_LEASE_SECONDS") {
        if let Ok(seconds) = value.parse::<u64>() {
            config.runtime_defaults.claim_lease_seconds = seconds;
        }
    }
    if let Ok(level) = env::var("AIS_AGENT_LOG_LEVEL") {
        if let Some(parsed) = parse_log_level(&level) {
            config.observability.log_level = parsed;
        }
    }
    if let Ok(dir) = env::var("AIS_AGENT_LOG_DIR") {
        config.observability.file_logging.enabled = true;
        config.observability.file_logging.dir = PathBuf::from(dir);
    }
    if let Ok(value) = env::var("AIS_AGENT_LOG_RETENTION_DAYS") {
        if let Ok(days) = value.parse::<u16>() {
            config.observability.file_logging.retention_days = days;
        }
    }
    if let Ok(dir) = env::var("AIS_AGENT_JSONL_CAPTURE_DIR") {
        config.observability.jsonl_capture.enabled = true;
        config.observability.jsonl_capture.dir = PathBuf::from(dir);
    }
    if let Ok(value) = env::var("AIS_AGENT_JSONL_CAPTURE_RETENTION_DAYS") {
        if let Ok(days) = value.parse::<u16>() {
            config.observability.jsonl_capture.retention_days = days;
        }
    }
}

fn apply_cli_overrides(config: &mut AisAgentServiceConfig, args: &Args) {
    if let Some(path) = args.sqlite_path.as_ref() {
        let mut sqlite = sqlite_storage_config(config);
        sqlite.path = PathBuf::from(path);
        config.storage = AisAgentStorageConfig::Sqlite(sqlite);
    }
    if let Some(seconds) = args.claim_lease_seconds {
        config.runtime_defaults.claim_lease_seconds = seconds;
    }
    if let Some(level) = args.log_level {
        config.observability.log_level = match level {
            LogLevelArg::Trace => AisAgentLogLevel::Trace,
            LogLevelArg::Debug => AisAgentLogLevel::Debug,
            LogLevelArg::Info => AisAgentLogLevel::Info,
            LogLevelArg::Warn => AisAgentLogLevel::Warn,
            LogLevelArg::Error => AisAgentLogLevel::Error,
        };
    }
    if let Some(dir) = args.log_dir.as_ref() {
        config.observability.file_logging.enabled = true;
        config.observability.file_logging.dir = PathBuf::from(dir);
    }
    if let Some(days) = args.log_retention_days {
        config.observability.file_logging.retention_days = days;
    }
    if let Some(dir) = args.jsonl_capture_dir.as_ref() {
        config.observability.jsonl_capture.enabled = true;
        config.observability.jsonl_capture.dir = PathBuf::from(dir);
    }
    if let Some(days) = args.jsonl_capture_retention_days {
        config.observability.jsonl_capture.retention_days = days;
    }
}

fn apply_command_defaults(config: &mut AisAgentServiceConfig, command: &CliCommand) {
    match command {
        CliCommand::Local {
            command: LocalCommand::Jsonl,
        } => {
            config.transport.jsonl.enabled = true;
        }
        CliCommand::Daemon {
            command: DaemonCommand::Http { bind },
        } => {
            config.transport.http.enabled = true;
            if let Some(bind) = bind.as_ref() {
                config.transport.http.bind = bind.clone();
            }
        }
        CliCommand::Inspect { .. } => {}
        CliCommand::Maintenance { .. } => {}
    }
}

fn validate_config(config: &AisAgentServiceConfig) -> Result<(), ServiceConfigError> {
    if config.runtime_defaults.claim_lease_seconds == 0 {
        return Err(ServiceConfigError::Invalid(
            "runtime_defaults.claim_lease_seconds must be greater than zero".to_owned(),
        ));
    }
    if config.runtime_defaults.step_wall_clock_ms == 0 {
        return Err(ServiceConfigError::Invalid(
            "runtime_defaults.step_wall_clock_ms must be greater than zero".to_owned(),
        ));
    }
    if config.runtime_defaults.confirmation_poll_ms == 0 {
        return Err(ServiceConfigError::Invalid(
            "runtime_defaults.confirmation_poll_ms must be greater than zero".to_owned(),
        ));
    }
    if config.observability.file_logging.retention_days == 0 {
        return Err(ServiceConfigError::Invalid(
            "observability.file_logging.retention_days must be greater than zero".to_owned(),
        ));
    }
    if config.observability.jsonl_capture.retention_days == 0 {
        return Err(ServiceConfigError::Invalid(
            "observability.jsonl_capture.retention_days must be greater than zero".to_owned(),
        ));
    }
    if config.transport.http.enabled && config.transport.http.bind.trim().is_empty() {
        return Err(ServiceConfigError::Invalid(
            "transport.http.bind must be non-empty when HTTP transport is enabled".to_owned(),
        ));
    }
    if let AisAgentStorageConfig::Sqlite(sqlite) = &config.storage {
        if sqlite.retention.checkpoint_full_window_days == 0 {
            return Err(ServiceConfigError::Invalid(
                "storage.retention.checkpoint_full_window_days must be greater than zero"
                    .to_owned(),
            ));
        }
        if sqlite.retention.checkpoint_boundary_only_window_days == 0 {
            return Err(ServiceConfigError::Invalid(
                "storage.retention.checkpoint_boundary_only_window_days must be greater than zero"
                    .to_owned(),
            ));
        }
        if sqlite.retention.wait_state_orphan_ttl_days == 0 {
            return Err(ServiceConfigError::Invalid(
                "storage.retention.wait_state_orphan_ttl_days must be greater than zero".to_owned(),
            ));
        }
        if sqlite.retention.auto_prune_cadence_minutes == 0 {
            return Err(ServiceConfigError::Invalid(
                "storage.retention.auto_prune_cadence_minutes must be greater than zero".to_owned(),
            ));
        }
    }
    validate_provider_config(config)?;
    Ok(())
}

fn validate_provider_config(config: &AisAgentServiceConfig) -> Result<(), ServiceConfigError> {
    let mut seen_chains = std::collections::BTreeSet::new();
    let mut seen_labels = std::collections::BTreeSet::new();
    for entry in &config.providers.chains {
        let chain = entry.chain.trim();
        if chain.is_empty() {
            return Err(ServiceConfigError::Invalid(
                "providers.chains[*].chain must be non-empty".to_owned(),
            ));
        }
        let family_prefix = parse_chain_scope_family(chain).ok_or_else(|| {
            ServiceConfigError::Invalid(format!(
                "providers.chains[{chain}].chain must use a supported canonical scope such as `eip155:8453` or `solana:mainnet`"
            ))
        })?;
        if !seen_chains.insert(chain.to_ascii_lowercase()) {
            return Err(ServiceConfigError::Invalid(format!(
                "providers.chains contains duplicate chain entry `{chain}`"
            )));
        }
        if !matches!(family_prefix, "eip155" | "solana") {
            return Err(ServiceConfigError::Invalid(format!(
                "providers.chains[{chain}].chain must use a supported canonical scope such as `eip155:8453` or `solana:mainnet`"
            )));
        }
        validate_http_url(
            &entry.connection.http_url,
            &format!("providers.chains[{chain}].connection.http_url"),
        )?;
        validate_ws_url(
            entry.connection.ws_url.as_deref(),
            &format!("providers.chains[{chain}].connection.ws_url"),
        )?;
        for label in &entry.labels {
            let normalized = label.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(ServiceConfigError::Invalid(format!(
                    "providers.chains[{chain}].labels entries must be non-empty"
                )));
            }
            if !seen_labels.insert(normalized.clone()) {
                return Err(ServiceConfigError::Invalid(format!(
                    "providers.chains contains duplicate label `{}`",
                    label.trim()
                )));
            }
        }
    }
    Ok(())
}

fn parse_chain_scope_family(value: &str) -> Option<&str> {
    let (prefix, suffix) = value.split_once(':')?;
    if prefix.is_empty() || suffix.trim().is_empty() {
        return None;
    }
    match prefix {
        "eip155" | "solana" => Some(prefix),
        _ => None,
    }
}

fn validate_http_url(value: &str, label: &str) -> Result<(), ServiceConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ServiceConfigError::Invalid(format!(
            "{label} must be non-empty"
        )));
    }
    if !matches!(
        trimmed.split_once("://"),
        Some(("http", _)) | Some(("https", _))
    ) {
        return Err(ServiceConfigError::Invalid(format!(
            "{label} must start with http:// or https://"
        )));
    }
    Ok(())
}

fn validate_ws_url(value: Option<&str>, label: &str) -> Result<(), ServiceConfigError> {
    let Some(value) = value else {
        return Ok(());
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ServiceConfigError::Invalid(format!(
            "{label} must be non-empty when provided"
        )));
    }
    if !matches!(
        trimmed.split_once("://"),
        Some(("ws", _)) | Some(("wss", _))
    ) {
        return Err(ServiceConfigError::Invalid(format!(
            "{label} must start with ws:// or wss://"
        )));
    }
    Ok(())
}

fn parse_log_level(value: &str) -> Option<AisAgentLogLevel> {
    match value {
        "trace" => Some(AisAgentLogLevel::Trace),
        "debug" => Some(AisAgentLogLevel::Debug),
        "info" => Some(AisAgentLogLevel::Info),
        "warn" => Some(AisAgentLogLevel::Warn),
        "error" => Some(AisAgentLogLevel::Error),
        _ => None,
    }
}

fn parse_bool_flag(value: &str) -> Option<bool> {
    match value {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn sqlite_storage_config(config: &AisAgentServiceConfig) -> AisAgentSqliteStorageConfig {
    match &config.storage {
        AisAgentStorageConfig::Sqlite(sqlite) => sqlite.clone(),
        AisAgentStorageConfig::InMemory => AisAgentSqliteStorageConfig::default(),
    }
}

fn sqlite_storage_config_mut(
    config: &mut AisAgentServiceConfig,
) -> &mut AisAgentSqliteStorageConfig {
    if !matches!(config.storage, AisAgentStorageConfig::Sqlite(_)) {
        config.storage = AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig::default());
    }
    match &mut config.storage {
        AisAgentStorageConfig::Sqlite(sqlite) => sqlite,
        AisAgentStorageConfig::InMemory => unreachable!("storage converted to sqlite"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        sync::{Mutex, OnceLock},
    };

    use clap::Parser;

    use crate::cli::args::{Args, CliCommand, DaemonCommand, LocalCommand};

    use super::load_service_config;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn reset_env() {
        env::remove_var("AIS_AGENT_HTTP_BIND");
        env::remove_var("AIS_AGENT_SQLITE_PATH");
        env::remove_var("AIS_AGENT_SQLITE_CHECKPOINT_FULL_WINDOW_DAYS");
        env::remove_var("AIS_AGENT_SQLITE_CHECKPOINT_BOUNDARY_ONLY_WINDOW_DAYS");
        env::remove_var("AIS_AGENT_SQLITE_WAIT_STATE_ORPHAN_TTL_DAYS");
        env::remove_var("AIS_AGENT_SQLITE_PURGE_ENABLED");
        env::remove_var("AIS_AGENT_SQLITE_PURGE_REQUIRE_CONFIRMATION");
        env::remove_var("AIS_AGENT_SQLITE_AUTO_PRUNE_CADENCE_MINUTES");
        env::remove_var("AIS_AGENT_SQLITE_VACUUM_FREELIST_THRESHOLD_PAGES");
        env::remove_var("AIS_AGENT_CLAIM_LEASE_SECONDS");
        env::remove_var("AIS_AGENT_LOG_LEVEL");
        env::remove_var("AIS_AGENT_LOG_DIR");
        env::remove_var("AIS_AGENT_LOG_RETENTION_DAYS");
        env::remove_var("AIS_AGENT_JSONL_CAPTURE_DIR");
        env::remove_var("AIS_AGENT_JSONL_CAPTURE_RETENTION_DAYS");
    }

    #[test]
    fn defaults_local_jsonl_to_jsonl_enabled() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let args = Args::try_parse_from(["ais-agent", "local", "jsonl"]).expect("args");
        let config = load_service_config(&args).expect("config");

        assert!(config.transport.jsonl.enabled);
        assert!(!config.transport.http.enabled);
    }

    #[test]
    fn loads_yaml_file_then_applies_env_and_cli_overrides() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-config.yaml");
        fs::write(
            &path,
            r#"
transport:
  http:
    enabled: true
    bind: 127.0.0.1:4100
storage:
  backend: sqlite
  retention:
    checkpoint_full_window_days: 9
runtime_defaults:
  claim_lease_seconds: 45
observability:
  file_logging:
    enabled: true
    dir: ./var/yaml-logs
    retention_days: 9
"#,
        )
        .expect("write");

        env::set_var("AIS_AGENT_CLAIM_LEASE_SECONDS", "75");
        env::set_var("AIS_AGENT_JSONL_CAPTURE_DIR", "./var/env-captures");
        env::set_var("AIS_AGENT_SQLITE_PURGE_ENABLED", "true");
        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "--sqlite-path",
            "./var/cli.db",
            "--log-retention-days",
            "14",
            "daemon",
            "http",
            "--bind",
            "0.0.0.0:8081",
        ])
        .expect("args");

        let config = load_service_config(&args).expect("config");

        assert!(config.transport.http.enabled);
        assert_eq!(config.transport.http.bind, "0.0.0.0:8081");
        assert_eq!(config.runtime_defaults.claim_lease_seconds, 75);
        assert!(config.observability.file_logging.enabled);
        assert_eq!(
            config.observability.file_logging.dir,
            PathBuf::from("./var/yaml-logs")
        );
        assert_eq!(config.observability.file_logging.retention_days, 14);
        assert!(config.observability.jsonl_capture.enabled);
        assert_eq!(
            config.observability.jsonl_capture.dir,
            PathBuf::from("./var/env-captures")
        );
        match config.storage {
            super::super::types::AisAgentStorageConfig::Sqlite(ref sqlite) => {
                assert_eq!(sqlite.path, PathBuf::from("./var/cli.db"));
                assert_eq!(sqlite.retention.checkpoint_full_window_days, 9);
                assert!(sqlite.retention.destructive_purge_enabled);
            }
            other => panic!("unexpected storage config: {other:?}"),
        }

        let _ = fs::remove_file(path);
        reset_env();
    }

    #[test]
    fn rejects_zero_claim_lease() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let args =
            Args::try_parse_from(["ais-agent", "--claim-lease-seconds", "0", "local", "jsonl"])
                .expect("args");

        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error
            .to_string()
            .contains("runtime_defaults.claim_lease_seconds"));
    }

    #[test]
    fn daemon_http_preserves_yaml_bind_when_flag_is_omitted() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-daemon-bind-preserve.yaml");
        fs::write(
            &path,
            r#"
transport:
  http:
    enabled: true
    bind: 127.0.0.1:3200
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "daemon",
            "http",
        ])
        .expect("args");

        let config = load_service_config(&args).expect("config");

        assert!(config.transport.http.enabled);
        assert_eq!(config.transport.http.bind, "127.0.0.1:3200");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn daemon_http_bind_flag_overrides_yaml_bind() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-daemon-bind-override.yaml");
        fs::write(
            &path,
            r#"
transport:
  http:
    enabled: true
    bind: 127.0.0.1:3200
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "daemon",
            "http",
            "--bind",
            "127.0.0.1:3000",
        ])
        .expect("args");

        let config = load_service_config(&args).expect("config");

        assert!(config.transport.http.enabled);
        assert_eq!(config.transport.http.bind, "127.0.0.1:3000");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn preserve_command_parse_shapes() {
        let _guard = env_lock().lock().expect("lock");
        let args = Args::try_parse_from(["ais-agent", "daemon", "http"]).expect("args");
        assert_eq!(
            args.command,
            CliCommand::Daemon {
                command: DaemonCommand::Http { bind: None },
            }
        );

        let args = Args::try_parse_from(["ais-agent", "local", "jsonl"]).expect("args");
        assert_eq!(
            args.command,
            CliCommand::Local {
                command: LocalCommand::Jsonl,
            }
        );
    }

    #[test]
    fn rejects_zero_observability_retention_windows() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let args =
            Args::try_parse_from(["ais-agent", "--log-retention-days", "0", "local", "jsonl"])
                .expect("args");

        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error
            .to_string()
            .contains("observability.file_logging.retention_days"));
    }

    #[test]
    fn rejects_zero_sqlite_retention_windows() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-invalid-retention.yaml");
        fs::write(
            &path,
            r#"
storage:
  backend: sqlite
  path: ./var/ais-agent.db
  retention:
    checkpoint_full_window_days: 0
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "inspect",
            "config",
        ])
        .expect("args");

        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error
            .to_string()
            .contains("storage.retention.checkpoint_full_window_days"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn loads_chain_scoped_provider_registry_from_yaml() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-provider-registry.yaml");
        fs::write(
            &path,
            r#"
providers:
  chains:
    - chain: eip155:8453
      labels: [base, 8453]
      connection:
        http_url: https://base.example
        ws_url: wss://base.example/ws
    - chain: solana:mainnet
      labels: [solana-mainnet]
      connection:
        http_url: https://solana.example
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "inspect",
            "config",
        ])
        .expect("args");
        let config = load_service_config(&args).expect("config");

        assert_eq!(config.providers.chains.len(), 2);
        assert_eq!(config.providers.chains[0].chain, "eip155:8453");
        assert_eq!(config.providers.chains[0].labels, vec!["base", "8453"]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_duplicate_provider_chains() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-provider-dup-chain.yaml");
        fs::write(
            &path,
            r#"
providers:
  chains:
    - chain: eip155:8453
      connection:
        http_url: https://base-a.example
    - chain: eip155:8453
      connection:
        http_url: https://base-b.example
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "inspect",
            "config",
        ])
        .expect("args");
        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error.to_string().contains("duplicate chain entry"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_duplicate_provider_labels() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-provider-dup-label.yaml");
        fs::write(
            &path,
            r#"
providers:
  chains:
    - chain: eip155:8453
      labels: [base]
      connection:
        http_url: https://base.example
    - chain: eip155:1
      labels: [Base]
      connection:
        http_url: https://eth.example
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "inspect",
            "config",
        ])
        .expect("args");
        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error.to_string().contains("duplicate label"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_unsupported_provider_chain_scope() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-provider-family-mismatch.yaml");
        fs::write(
            &path,
            r#"
providers:
  chains:
    - chain: cosmos:osmosis
      connection:
        http_url: https://base.example
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "inspect",
            "config",
        ])
        .expect("args");
        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error
            .to_string()
            .contains("must use a supported canonical scope"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_invalid_provider_ws_url() {
        let _guard = env_lock().lock().expect("lock");
        reset_env();
        let path = std::env::temp_dir().join("ais-agent-cli-provider-invalid-ws.yaml");
        fs::write(
            &path,
            r#"
providers:
  chains:
    - chain: eip155:8453
      connection:
        http_url: https://base.example
        ws_url: https://base.example/ws
"#,
        )
        .expect("write");

        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "inspect",
            "config",
        ])
        .expect("args");
        let error = load_service_config(&args).expect_err("invalid config");
        assert!(error
            .to_string()
            .contains("must start with ws:// or wss://"));

        let _ = fs::remove_file(path);
    }
}
