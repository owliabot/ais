use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

use ais_agent_observability_files::DailyFileSink;
use thiserror::Error;
use tracing_subscriber::fmt::writer::MakeWriter;

use crate::config::{AisAgentLogLevel, AisAgentServiceConfig};

pub fn install_tracing(config: &AisAgentServiceConfig) -> Result<(), InstallTracingError> {
    let file_sink = if config.observability.file_logging.enabled {
        Some(Arc::new(Mutex::new(DailyFileSink::new(
            config.observability.file_logging.dir.clone(),
            "ais-agent",
            "log",
            config.observability.file_logging.retention_days,
        )?)))
    } else {
        None
    };
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()) && file_sink.is_none())
        .with_writer(CombinedMakeWriter { file_sink })
        .with_max_level(to_tracing_level(config.observability.log_level))
        .finish();

    tracing::subscriber::set_global_default(subscriber).map_err(InstallTracingError::from)
}

#[derive(Debug, Error)]
pub enum InstallTracingError {
    #[error("failed to initialize file logging: {0}")]
    Io(#[from] io::Error),
    #[error("failed to install tracing subscriber: {0}")]
    Subscriber(#[from] tracing::subscriber::SetGlobalDefaultError),
}

#[derive(Clone)]
struct CombinedMakeWriter {
    file_sink: Option<Arc<Mutex<DailyFileSink>>>,
}

impl<'a> MakeWriter<'a> for CombinedMakeWriter {
    type Writer = CombinedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CombinedWriter {
            stderr: io::stderr(),
            file_sink: self.file_sink.clone(),
        }
    }
}

struct CombinedWriter {
    stderr: io::Stderr,
    file_sink: Option<Arc<Mutex<DailyFileSink>>>,
}

impl Write for CombinedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stderr.write_all(buf)?;
        if let Some(file_sink) = self.file_sink.as_ref() {
            let mut sink = file_sink
                .lock()
                .map_err(|_| io::Error::other("file sink mutex poisoned"))?;
            sink.append_bytes(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stderr.flush()?;
        if let Some(file_sink) = self.file_sink.as_ref() {
            let mut sink = file_sink
                .lock()
                .map_err(|_| io::Error::other("file sink mutex poisoned"))?;
            sink.flush()?;
        }
        Ok(())
    }
}

fn to_tracing_level(level: AisAgentLogLevel) -> tracing::Level {
    match level {
        AisAgentLogLevel::Trace => tracing::Level::TRACE,
        AisAgentLogLevel::Debug => tracing::Level::DEBUG,
        AisAgentLogLevel::Info => tracing::Level::INFO,
        AisAgentLogLevel::Warn => tracing::Level::WARN,
        AisAgentLogLevel::Error => tracing::Level::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::to_tracing_level;
    use crate::config::AisAgentLogLevel;

    #[test]
    fn maps_cli_log_levels_to_tracing_levels() {
        assert_eq!(
            to_tracing_level(AisAgentLogLevel::Trace),
            tracing::Level::TRACE
        );
        assert_eq!(
            to_tracing_level(AisAgentLogLevel::Debug),
            tracing::Level::DEBUG
        );
        assert_eq!(
            to_tracing_level(AisAgentLogLevel::Info),
            tracing::Level::INFO
        );
        assert_eq!(
            to_tracing_level(AisAgentLogLevel::Warn),
            tracing::Level::WARN
        );
        assert_eq!(
            to_tracing_level(AisAgentLogLevel::Error),
            tracing::Level::ERROR
        );
    }
}
