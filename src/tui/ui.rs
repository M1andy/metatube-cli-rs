/// 渲染布局：头部信息 / 进度区 / 任务与统计 / 日志 / 状态栏。
/// 进度条使用 ratatui Gauge 的 Unicode 模式（█▉▊▋▌▍▎▏ 八分块平滑渐变），
/// spinner 使用盲文字符帧动画，全程无 ASCII 字符进度。
use super::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;
use tracing::Level;

const BORDER: Color = Color::DarkGray;
const ACCENT: Color = Color::Cyan;

pub fn render(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(10),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], app);
    render_progress(frame, chunks[1], app);
    render_body(frame, chunks[2], app);
    render_logs(frame, chunks[3], app);
    render_status(frame, chunks[4], app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!(" metatube-cli-rs v{} ", env!("CARGO_PKG_VERSION"));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let line = Line::from(vec![
        Span::styled(
            "模式: ",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.mode_label, Style::default().fg(Color::Yellow)),
        Span::raw("  │  "),
        Span::styled(
            "服务器: ",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.server_url.as_str(), Style::default().fg(Color::Green)),
    ]);
    frame.render_widget(Paragraph::new(line).centered(), inner);
}

fn render_progress(frame: &mut Frame, area: Rect, app: &App) {
    // 完成汇总：一轮结束后展示结果，等待用户手动退出
    if let Some(r) = &app.last_round {
        let text = Line::from(vec![
            Span::styled("✓ ", Style::default().fg(Color::Green)),
            Span::styled(
                "处理完成",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} 成功", r.success),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} 跳过", r.skipped),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} 失败", r.failed),
                Style::default().fg(Color::Red),
            ),
            Span::raw("  "),
            Span::styled("— 按 q 或 Esc 退出", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(text).centered(), area);
        return;
    }

    if app.scanning {
        let text = Line::from(vec![
            Span::styled(
                format!("{} ", app.spinner_frame()),
                Style::default().fg(ACCENT),
            ),
            Span::styled("正在扫描目录...", Style::default().fg(Color::Gray)),
            Span::raw(format!("  已遍历 {} 个目录", app.scan_dirs)),
        ]);
        frame.render_widget(Paragraph::new(text), area);
        return;
    }

    if app.round_active && app.total > 0 {
        let ratio = (app.done as f64 / app.total as f64).clamp(0.0, 1.0);
        let eta = app
            .eta_secs()
            .map(|s| format!("  ETA {}", format_duration(s)))
            .unwrap_or_default();
        let label = format!(" {}/{}{} ", app.done, app.total, eta);
        let gauge = Gauge::default()
            .ratio(ratio)
            .use_unicode(true)
            .label(label)
            .gauge_style(
                Style::default()
                    .fg(ACCENT)
                    .bg(Color::Rgb(40, 44, 52))
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_widget(gauge, area);
        return;
    }

    // 空闲（watch 等待新文件）
    let text = Line::from(vec![
        Span::styled(
            format!("{} ", app.spinner_frame()),
            Style::default().fg(ACCENT),
        ),
        Span::styled("等待新视频文件...", Style::default().fg(Color::Gray)),
    ]);
    frame.render_widget(Paragraph::new(text), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    render_tasks(frame, cols[0], app);
    render_stats(frame, cols[1], app);
}

fn render_tasks(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if app.tasks.is_empty() { BORDER } else { ACCENT }))
        .title(" 当前任务 ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible = inner.height as usize;
    let mut items: Vec<ListItem<'_>> = Vec::new();
    let shown: usize = app
        .tasks
        .len()
        .min(super::app::TASKS_VISIBLE_MAX)
        .min(visible.saturating_sub(1).max(1));

    for task in app.tasks.iter().take(shown) {
        let stage_label = task.stage.map(|s| s.label()).unwrap_or("识别番号");
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!("{} ", app.spinner_frame()),
                Style::default().fg(ACCENT),
            ),
            Span::styled(task.filename.as_str(), Style::default().fg(Color::White)),
            Span::styled(
                format!(" ─ {}", stage_label),
                Style::default().fg(Color::Gray),
            ),
        ])));
    }

    let hidden = app.tasks.len().saturating_sub(shown);
    if hidden > 0 && items.len() < visible {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("… 还有 {} 个任务", hidden),
            Style::default().fg(Color::DarkGray),
        ))));
    }

    frame.render_widget(List::new(items), inner);
}

fn render_stats(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(" 统计 ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("✓ 成功  ", Style::default().fg(Color::Green)),
            Span::styled(
                app.stats.success.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("→ 跳过  ", Style::default().fg(Color::Yellow)),
            Span::styled(
                app.stats.skipped.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("✗ 失败  ", Style::default().fg(Color::Red)),
            Span::styled(
                app.stats.failed.to_string(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![Span::styled(
            format!("进度    {}/{}", app.done, app.total),
            Style::default().fg(Color::Gray),
        )]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_logs(frame: &mut Frame, area: Rect, app: &App) {
    let title = if app.log_scroll_offset > 0 {
        format!(" 日志（已上滚 {} 行） ", app.log_scroll_offset)
    } else {
        " 日志 ".to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let vis = inner.height as usize;
    if vis == 0 || app.logs.is_empty() {
        return;
    }

    // 尾部窗口跟随 + 手动滚动偏移
    let start = app.logs.len().saturating_sub(vis + app.log_scroll_offset);
    let end = (start + vis).min(app.logs.len());
    let lines: Vec<Line<'_>> = app
        .logs
        .range(start..end)
        .map(|log| {
            let (color, abbr) = level_style(log.level);
            Line::from(vec![
                Span::styled(
                    format!("{} ", log.time),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("[{:4}] ", abbr), Style::default().fg(color)),
                Span::styled(log.message.as_str(), Style::default().fg(Color::Gray)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " q 退出 ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw("  ↑↓ 滚动日志  "),
    ];

    if let Some(dir) = &app.watch_dir {
        spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("监视中 {}", dir.display()),
            Style::default().fg(Color::Gray),
        ));
    }

    if let Some(at) = app.next_schedule {
        let remaining = (at - chrono::Local::now()).num_seconds().max(0) as u64;
        spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("下次调度 {}", format_duration(remaining)),
            Style::default().fg(Color::Gray),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(40, 44, 52))),
        area,
    );
}

fn level_style(level: Level) -> (Color, &'static str) {
    match level {
        Level::ERROR => (Color::Red, "ERRO"),
        Level::WARN => (Color::Yellow, "WARN"),
        Level::INFO => (Color::Green, "INFO"),
        Level::DEBUG => (Color::Blue, "DEBG"),
        Level::TRACE => (Color::Magenta, "TRCE"),
    }
}

/// 秒数 → "MM:SS" / "HH:MM:SS"。
pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::App;
    use super::*;
    use crate::tui::event::{AppEvent, FileStatus};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal as TestTerminal;

    fn render_to_string(app: &mut App) -> String {
        let backend = TestBackend::new(100, 30);
        let mut terminal = TestTerminal::new(backend).unwrap();
        terminal.draw(|f| render(app, f)).unwrap();
        let mut out = String::new();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .for_each(|c| out.push_str(&c.symbol().to_string()));
        out
    }

    /// 宽字符（中文）在缓冲区中占 1 个 cell + 1 个填充 cell，
    /// 拼接后汉字之间会插入填充符号；此函数去掉空白便于断言。
    fn visible_text(app: &mut App) -> String {
        let rendered = render_to_string(app);
        rendered.split_whitespace().collect::<String>()
    }

    #[test]
    fn test_render_active_round_shows_gauge_and_stats() {
        let mut app = App::new("单次扫描", "http://localhost:8080".into());
        app.handle_event(AppEvent::RoundStart { total: 4 });
        app.handle_event(AppEvent::FileStart {
            filename: "SSIS-001.mp4".into(),
        });
        app.handle_event(AppEvent::FileStage {
            filename: "SSIS-001.mp4".into(),
            stage: crate::tui::event::Stage::Search,
        });
        app.handle_event(AppEvent::FileDone {
            filename: "SSIS-001.mp4".into(),
            status: FileStatus::Skipped,
        });
        app.handle_event(AppEvent::FileStart {
            filename: "SSIS-002.mp4".into(),
        });
        app.handle_event(AppEvent::FileStage {
            filename: "SSIS-002.mp4".into(),
            stage: crate::tui::event::Stage::Detail,
        });

        let out = visible_text(&mut app);
        assert!(out.contains("1/4"), "进度数字应渲染");
        assert!(out.contains("单次扫描"));
        assert!(out.contains("localhost:8080"));
        assert!(out.contains("SSIS-002.mp4"), "活动任务应渲染");
        assert!(out.contains("获取详情"), "任务阶段应渲染");
    }

    #[test]
    fn test_render_unicode_gauge_blocks() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::RoundStart { total: 2 });
        app.handle_event(AppEvent::FileDone {
            filename: "a.mp4".into(),
            status: FileStatus::Skipped,
        });
        app.handle_event(AppEvent::FileDone {
            filename: "b.mp4".into(),
            status: FileStatus::Skipped,
        });

        let out = render_to_string(&mut app);
        // Gauge Unicode 模式应渲染八分块字符而非 ASCII
        assert!(out.contains('█'), "应包含 Unicode 全块字符: {}", out);
        assert!(!out.contains('#'), "不应包含 ASCII 进度字符");
    }

    #[test]
    fn test_render_scanning_spinner() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::ScanStart);
        app.handle_event(AppEvent::ScanProgress { dirs: 7 });

        let out = visible_text(&mut app);
        assert!(out.contains("正在扫描目录"));
        assert!(out.contains('7'));
        let spinner_chars: Vec<char> = super::super::app::SPINNER_FRAMES
            .iter()
            .flat_map(|s| s.chars())
            .collect();
        assert!(out.chars().any(|c| spinner_chars.contains(&c)));
    }

    #[test]
    fn test_render_round_done_summary() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::RoundStart { total: 3 });
        app.handle_event(AppEvent::RoundDone {
            success: 2,
            skipped: 1,
            failed: 0,
        });

        let out = visible_text(&mut app);
        assert!(out.contains("处理完成"));
        assert!(out.contains("2成功"));
        assert!(out.contains("1跳过"));
        // 完成后等待用户手动退出，应有按键提示
        assert!(out.contains("按q或Esc退出"));
    }

    #[test]
    fn test_render_logs_with_level_colors() {
        let mut app = App::new("文件监视", "srv".into());
        app.handle_event(AppEvent::Log {
            level: Level::INFO,
            message: "hello tui".into(),
        });
        let out = render_to_string(&mut app);
        assert!(out.contains("hello tui"));
        assert!(out.contains("[INFO]"));
    }

    #[test]
    fn test_render_watch_status_bar() {
        let mut app = App::new("文件监视", "srv".into());
        app.handle_event(AppEvent::WatchReady {
            path: std::path::PathBuf::from("D:/videos"),
        });
        let out = visible_text(&mut app);
        assert!(out.contains("D:/videos"));
        assert!(out.contains("等待新视频文件"));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "00:00");
        assert_eq!(format_duration(65), "01:05");
        assert_eq!(format_duration(3600), "01:00:00");
        assert_eq!(format_duration(3661), "01:01:01");
    }
}
