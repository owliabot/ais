use std::net::SocketAddr;

use ais_agent_host::{control::HostCommandService, events::HostRunEventService};
use ais_agent_transport::http::build_http_router;

use crate::config::AisAgentServiceConfig;

pub async fn daemon_http<S>(
    config: &AisAgentServiceConfig,
    service: S,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    let addr: SocketAddr = config.transport.http.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let app = build_http_router(service);

    axum::serve(listener, app).await?;
    Ok(())
}
