use std::net::SocketAddr;

use ais_agent_transport::http::build_http_router;

use crate::service::UnavailableHostService;

pub async fn daemon_http(bind: &str) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let app = build_http_router(UnavailableHostService::default());

    axum::serve(listener, app).await?;
    Ok(())
}
