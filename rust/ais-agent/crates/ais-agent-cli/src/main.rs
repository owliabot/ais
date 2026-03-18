mod bootstrap;
mod cli;
mod commands;
mod config;
mod observability;
mod service;
mod storage_maintenance;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::run(std::env::args().skip(1)).await
}
