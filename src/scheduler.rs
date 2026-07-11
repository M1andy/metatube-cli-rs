use crate::config::Config;
use crate::processor;
use chrono::Utc;
use tracing::{error, info};

pub async fn run_scheduled(config: &Config, cron_expr: &str) -> anyhow::Result<()> {
    let schedule = croner::Cron::new(cron_expr).parse()?;
    info!("cron mode: {}", cron_expr);

    run_once(config).await;

    loop {
        let next = schedule.find_next_occurrence(&Utc::now(), false)?;
        let delay = (next - Utc::now()).num_seconds().max(0) as u64;
        info!("next run in {}s", delay);
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        run_once(config).await;
    }
}

async fn run_once(config: &Config) {
    match processor::run(config).await {
        Ok(()) => info!("run complete"),
        Err(e) => error!("run failed: {}", e),
    }
}
