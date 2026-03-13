mod unavailable;

use std::{future::Future, pin::Pin};

use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::InMemoryHostSessionStore,
};
use ais_agent_runtime::{
    persistence::{
        InMemoryCheckpointRepository, InMemoryEventArchive, InMemoryMissionRepository,
        InMemoryRunCatalogRepository,
    },
    runtime::InMemoryRunRepository,
    service::RuntimeHostService,
};
use ais_agent_store_sqlite::SqliteStore;

pub use unavailable::UnavailableHostService;

pub type InMemoryCliRuntimeHostService = RuntimeHostService<
    InMemoryRunRepository,
    InMemoryCheckpointRepository,
    InMemoryMissionRepository,
    InMemoryRunCatalogRepository,
    InMemoryEventArchive,
    InMemoryHostSessionStore,
>;

pub type SqliteCliRuntimeHostService = RuntimeHostService<
    InMemoryRunRepository,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
    InMemoryHostSessionStore,
    SqliteStore,
    SqliteStore,
    SqliteStore,
>;

#[derive(Debug)]
pub enum CliHostService {
    RuntimeInMemory(InMemoryCliRuntimeHostService),
    RuntimeSqlite(SqliteCliRuntimeHostService),
    Unavailable(UnavailableHostService),
}

impl CliHostService {
    pub fn unavailable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Unavailable(UnavailableHostService::new(code, message))
    }
}

impl Default for CliHostService {
    fn default() -> Self {
        Self::Unavailable(UnavailableHostService::default())
    }
}

impl HostCommandService for CliHostService {
    fn handle(
        &mut self,
        command: ais_agent_host::session::HostedRunCommand,
    ) -> Pin<Box<dyn Future<Output = HostCommandOutcome> + Send + '_>> {
        match self {
            Self::RuntimeInMemory(service) => service.handle(command),
            Self::RuntimeSqlite(service) => service.handle(command),
            Self::Unavailable(service) => service.handle(command),
        }
    }
}

impl HostRunEventService for CliHostService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> Pin<Box<dyn Future<Output = Result<HostRunEventBatch, HostEventServiceError>> + Send + '_>>
    {
        match self {
            Self::RuntimeInMemory(service) => service.list_events(query),
            Self::RuntimeSqlite(service) => service.list_events(query),
            Self::Unavailable(service) => service.list_events(query),
        }
    }
}
