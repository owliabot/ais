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
        config.storage = AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig {
            path: PathBuf::from(sqlite_path),
            create_if_missing: true,
        });
    }
    if let Ok(url) = env::var("AIS_AGENT_EVM_RPC_URL") {
        config.providers.evm_rpc_url = Some(url);
    }
    if let Ok(url) = env::var("AIS_AGENT_SOLANA_RPC_URL") {
        config.providers.solana_rpc_url = Some(url);
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
}

fn apply_cli_overrides(config: &mut AisAgentServiceConfig, args: &Args) {
    if let Some(path) = args.sqlite_path.as_ref() {
        config.storage = AisAgentStorageConfig::Sqlite(AisAgentSqliteStorageConfig {
            path: PathBuf::from(path),
            create_if_missing: true,
        });
    }
    if let Some(url) = args.evm_rpc_url.as_ref() {
        config.providers.evm_rpc_url = Some(url.clone());
    }
    if let Some(url) = args.solana_rpc_url.as_ref() {
        config.providers.solana_rpc_url = Some(url.clone());
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
            config.transport.http.bind = bind.clone();
        }
        CliCommand::Inspect { .. } => {}
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
    if config.transport.http.enabled && config.transport.http.bind.trim().is_empty() {
        return Err(ServiceConfigError::Invalid(
            "transport.http.bind must be non-empty when HTTP transport is enabled".to_owned(),
        ));
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
        env::remove_var("AIS_AGENT_EVM_RPC_URL");
        env::remove_var("AIS_AGENT_SOLANA_RPC_URL");
        env::remove_var("AIS_AGENT_CLAIM_LEASE_SECONDS");
        env::remove_var("AIS_AGENT_LOG_LEVEL");
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
runtime_defaults:
  claim_lease_seconds: 45
"#,
        )
        .expect("write");

        env::set_var("AIS_AGENT_CLAIM_LEASE_SECONDS", "75");
        let args = Args::try_parse_from([
            "ais-agent",
            "--config",
            path.to_str().expect("path"),
            "--sqlite-path",
            "./var/cli.db",
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
        match config.storage {
            super::super::types::AisAgentStorageConfig::Sqlite(ref sqlite) => {
                assert_eq!(sqlite.path, PathBuf::from("./var/cli.db"));
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
    fn preserve_command_parse_shapes() {
        let _guard = env_lock().lock().expect("lock");
        let args = Args::try_parse_from(["ais-agent", "daemon", "http"]).expect("args");
        assert_eq!(
            args.command,
            CliCommand::Daemon {
                command: DaemonCommand::Http {
                    bind: "127.0.0.1:3000".to_owned(),
                },
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
}
