mod api;
mod config;
mod error;
mod number;
mod processor;
mod scanner;
mod scheduler;

use config::Config;
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

    if let Some(ref cron_expr) = config.cron.clone() {
        info!("starting cron mode: {}", cron_expr);
        scheduler::run_scheduled(config, cron_expr).await?;
    } else {
        info!("starting single run");
        processor::run(&config).await?;
    }

    Ok(())
}
