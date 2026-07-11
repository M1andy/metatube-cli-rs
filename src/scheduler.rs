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

#[cfg(test)]
mod tests {
    #[test]
    fn test_cron_parse_valid() {
        let schedule = croner::Cron::new("0 */6 * * *").parse();
        assert!(schedule.is_ok());
    }

    #[test]
    fn test_cron_parse_valid_every_minute() {
        let schedule = croner::Cron::new("* * * * *").parse();
        assert!(schedule.is_ok());
    }

    #[test]
    fn test_cron_parse_valid_daily() {
        let schedule = croner::Cron::new("0 9 * * *").parse();
        assert!(schedule.is_ok());
    }

    #[test]
    fn test_cron_parse_valid_weekly() {
        let schedule = croner::Cron::new("0 9 * * 1").parse();
        assert!(schedule.is_ok());
    }

    #[test]
    fn test_cron_parse_invalid() {
        let schedule = croner::Cron::new("not a cron expression").parse();
        assert!(schedule.is_err());
    }

    #[test]
    fn test_cron_parse_invalid_empty() {
        let schedule = croner::Cron::new("").parse();
        assert!(schedule.is_err());
    }

    #[test]
    fn test_cron_find_next_occurrence_future() {
        let schedule = croner::Cron::new("* * * * *").parse().unwrap();
        let now = chrono::Utc::now();
        let next = schedule.find_next_occurrence(&now, false).unwrap();
        assert!(next > now);
    }
}
