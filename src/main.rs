mod api;
mod config;
mod error;
mod number;
mod processor;
mod scanner;
mod scheduler;
mod watcher;

use config::{Config, RunMode};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::load();

    match config.mode {
        RunMode::Once => {
            info!("starting single run");
            processor::run(&config).await?;
        }
        RunMode::Cron => {
            let cron_expr = config
                .cron_expr
                .as_ref()
                .expect("cron expression required for cron mode");
            info!("starting cron mode: {}", cron_expr);
            scheduler::run_scheduled(&config, cron_expr).await?;
        }
        RunMode::Watch => {
            info!("starting watch mode");
            watcher::run_watch(&config).await?;
        }
    }

    Ok(())
}
