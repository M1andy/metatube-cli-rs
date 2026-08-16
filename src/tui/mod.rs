/// 全屏 TUI：ratatui + crossterm。
/// 渲染线程独占终端，通过 mpsc 通道消费业务事件；
/// 退出（完成/按键）时恢复终端——alt-screen 整体消失，终端历史零残留。
pub mod app;
pub mod event;
pub mod ui;

use crate::tui::app::App;
use crate::tui::event::AppEvent;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;
use tracing::error;

/// 渲染 tick 间隔（同时也是键盘事件的响应粒度）。
const TICK: Duration = Duration::from_millis(100);

/// 进入 TUI 模式：raw mode + 备用屏幕 + panic 恢复钩子。
/// panic 时先恢复终端再传播，避免终端残留乱码。
pub fn setup() -> std::io::Result<DefaultTerminal> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::try_restore();
        previous_hook(info);
    }));
    ratatui::try_init()
}

/// TUI 渲染线程主循环。线程返回前恢复终端。
#[allow(clippy::too_many_arguments)]
pub fn render_loop(
    mut terminal: DefaultTerminal,
    rx: Receiver<AppEvent>,
    quit_flag: Arc<AtomicBool>,
    mode_label: &'static str,
    server_url: String,
    dry_run: bool,
    concurrency: usize,
) {
    let mut app = App::new(mode_label, server_url).with_meta(dry_run, concurrency);

    loop {
        // 批量消费业务事件，超时当作一次 tick
        match rx.recv_timeout(TICK) {
            Ok(ev) => {
                app.handle_event(ev);
                while let Ok(ev) = rx.try_recv() {
                    app.handle_event(ev);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // 发送端全部关闭：业务已结束且不再有事件来源
            Err(RecvTimeoutError::Disconnected) => app.exit = true,
        }
        app.tick += 1;

        // 键盘事件（非阻塞扫描，只处理按下）
        while crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
            let event = match crossterm::event::read() {
                Ok(ev) => ev,
                Err(e) => {
                    // 终端事件流已损坏（如终端被关闭），无法继续渲染
                    error!("读取键盘事件失败: {}", e);
                    app.exit = true;
                    break;
                }
            };
            let Event::Key(key) = event else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // 帮助浮层打开时：仅响应关闭浮层，其余按键不透传
            if app.show_help {
                if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
                    app.show_help = false;
                }
                continue;
            }

            let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL));
            if quit {
                app.exit = true;
                quit_flag.store(true, Ordering::SeqCst);
            } else {
                match key.code {
                    KeyCode::Up => app.scroll_logs_up(),
                    KeyCode::Down => app.scroll_logs_down(),
                    KeyCode::PageUp => app.scroll_logs_page_up(),
                    KeyCode::PageDown => app.scroll_logs_page_down(),
                    KeyCode::Home => app.scroll_logs_top(),
                    KeyCode::End => app.scroll_logs_bottom(),
                    KeyCode::Char('?') => app.show_help = true,
                    _ => {}
                }
            }
        }

        if let Err(e) = terminal.draw(|f| ui::render(&mut app, f)) {
            error!("TUI 渲染失败: {}", e);
            app.exit = true;
        }

        if app.exit {
            break;
        }
    }

    ratatui::restore();
}
