mod api;
mod config;
mod error;
mod logging;
mod number;
mod processor;
mod scanner;
mod scheduler;
mod watcher;

use config::{Config, RunMode};
use logging::CleanFormat;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .event_format(CleanFormat)
        .init();

    let config = Config::load();

    // Startup banner
    info!("══════════════════════════════════════");
    info!("  MetaTube 视频整理工具 v{}", env!("CARGO_PKG_VERSION"));
    info!("══════════════════════════════════════");
    info!("  下载目录: {}", config.jav_download.display());
    info!("  输出目录: {}", config.jav_output.display());
    info!(
        "  运行模式: {}",
        match config.mode {
            RunMode::Once => "单次扫描",
            RunMode::Cron => "定时执行",
            RunMode::Watch => "文件监视",
        }
    );
    info!("  并发处理: {} 个文件", config.concurrency);
    if config.dry_run {
        info!("  预览模式: 是");
    }
    info!("══════════════════════════════════════");

    match config.mode {
        RunMode::Once => {
            info!("→ 开始扫描视频文件...");
            processor::run(&config).await?;
        }
        RunMode::Cron => {
            let cron_expr = config
                .cron_expr
                .as_ref()
                .expect("cron expression required for cron mode");
            info!("→ 定时模式已启动，计划: {}", cron_expr);
            scheduler::run_scheduled(&config, cron_expr).await?;
        }
        RunMode::Watch => {
            info!("→ 文件监视模式已启动");
            watcher::run_watch(&config).await?;
        }
    }

    Ok(())
}
