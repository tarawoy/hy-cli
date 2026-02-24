use anyhow::Result;

mod cli;
mod watch;
mod trade;
mod server_cmd;
mod watch_server;
mod format;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
