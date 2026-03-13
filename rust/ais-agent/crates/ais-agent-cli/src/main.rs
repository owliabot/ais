mod bootstrap;
mod cli;
mod commands;
mod config;
mod service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::run(std::env::args().skip(1)).await
}
