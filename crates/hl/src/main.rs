use anyhow::Result;

mod cli;
mod watch;
mod trade;
mod server_cmd;
mod watch_server;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
