mod api;
mod config;
mod error;
mod logging;
mod number;
mod processor;
mod scanner;
mod scheduler;
mod tui;
mod watcher;

use crate::tui::event::{AppEvent, ChannelReporter, NoopReporter, Reporter};
use config::{Config, RunMode};
use std::io::IsTerminal;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 配置先于日志加载：是否启用 TUI 决定日志去向
    // （代价是配置加载期间的少量 warn 日志丢失，可接受）
    let config = Config::load()?;

    let tui_supported =
        !config.no_tui && std::io::stdout().is_terminal() && std::io::stderr().is_terminal();

    // TUI 初始化失败（终端不支持等）时回退纯文本模式
    let (reporter, tui_join, quit_flag) = if tui_supported {
        match tui::setup() {
            Ok(terminal) => {
                let (tx, rx) = mpsc::sync_channel::<AppEvent>(1024);
                logging::init_tui(tx.clone());
                let quit_flag = Arc::new(AtomicBool::new(false));
                let mode_label = config.mode.label();
                let server_url = config.server_url.clone();
                let flag = quit_flag.clone();
                let join = std::thread::Builder::new()
                    .name("tui-renderer".into())
                    .spawn(move || tui::render_loop(terminal, rx, flag, mode_label, server_url))?;
                (
                    Arc::new(ChannelReporter::new(tx)) as Arc<dyn Reporter>,
                    Some(join),
                    quit_flag,
                )
            }
            Err(e) => {
                let _ = ratatui::try_restore();
                logging::init_plain();
                warn!("⚠ 终端不支持 TUI 模式（{}），回退纯文本输出", e);
                (
                    Arc::new(NoopReporter) as Arc<dyn Reporter>,
                    None,
                    Arc::new(AtomicBool::new(false)),
                )
            }
        }
    } else {
        logging::init_plain();
        (
            Arc::new(NoopReporter) as Arc<dyn Reporter>,
            None,
            Arc::new(AtomicBool::new(false)),
        )
    };

    // Startup banner（TUI 模式下进入日志面板）
    info!("══════════════════════════════════════");
    info!("  MetaTube 视频整理工具 v{}", env!("CARGO_PKG_VERSION"));
    info!("══════════════════════════════════════");
    info!("  下载目录: {}", config.jav_download.display());
    info!("  输出目录: {}", config.jav_output.display());
    info!("  失败目录: {}", config.jav_failed.display());
    info!("  运行模式: {}", config.mode.label());
    info!("  并发处理: {} 个文件", config.concurrency);
    if config.dry_run {
        info!("  预览模式: 是");
    }
    info!("══════════════════════════════════════");

    // 结果统一捕获，保证任何路径（含出错）都先完成 TUI 收尾再返回
    let result = match config.mode {
        RunMode::Once => {
            info!("→ 开始扫描视频文件...");
            processor::run(&config, reporter.clone()).await
        }
        RunMode::Cron => {
            match config
                .cron_expr
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("cron expression required for cron mode"))
            {
                Ok(cron_expr) => {
                    info!("→ 定时模式已启动，计划: {}", cron_expr);
                    scheduler::run_scheduled(&config, cron_expr, reporter.clone(), &quit_flag).await
                }
                Err(e) => Err(e),
            }
        }
        RunMode::Watch => {
            info!("→ 文件监视模式已启动");
            watcher::run_watch(&config, reporter.clone(), &quit_flag).await
        }
    };

    // TUI 收尾：终端恢复原状
    if tui_join.is_some() {
        if config.mode == RunMode::Once {
            // once 模式：完成后保持界面（汇总 + 日志可滚动），等用户手动按 q/Esc 退出
        } else {
            // cron/watch：业务循环已退出，通知渲染线程关闭
            reporter.emit(AppEvent::Shutdown);
        }
    }
    if let Some(join) = tui_join {
        let _ = join.join();
    }

    result
}
