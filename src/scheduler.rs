use crate::config::Config;
use crate::processor;
use chrono::Utc;
use tracing::{error, info};

pub async fn run_scheduled(config: &Config, cron_expr: &str) -> anyhow::Result<()> {
    let schedule = croner::Cron::new(cron_expr).parse()?;

    run_once(config).await;

    loop {
        let next = schedule.find_next_occurrence(&Utc::now(), false)?;
        let delay = (next - Utc::now()).num_seconds().max(0) as u64;
        info!("→ 下次执行: {}", format_duration(delay));
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        run_once(config).await;
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{} 小时 {} 分钟", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{} 分钟", seconds / 60)
    } else {
        format!("{} 秒", seconds)
    }
}

async fn run_once(config: &Config) {
    match processor::run(config).await {
        Ok(()) => info!("✓ 本轮扫描完成"),
        Err(_e) => error!("✗ 扫描执行失败，将在下次计划重试"),
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
