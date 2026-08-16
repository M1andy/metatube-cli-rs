/// 渲染布局：头部信息 / 进度区 / 任务与统计 / 日志 / 状态栏。
/// 进度条使用 ratatui Gauge 的 Unicode 模式（█▉▊▋▌▍▎▏ 八分块平滑渐变），
/// spinner 使用盲文字符帧动画，全程无 ASCII 字符进度。
/// 长日志自动换行（按显示行计算视口）；终端过小时展示提示页；
/// 颜色只用终端标准色，浅色 / 256 色主题下均可正常降级。
use super::app::{App, LogLine, RECENT_FAILURES_MAX};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use tracing::Level;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BORDER: Color = Color::DarkGray;
const ACCENT: Color = Color::Cyan;

/// 终端过小提示页阈值：低于该尺寸时不渲染正常布局。
pub const MIN_WIDTH: u16 = 50;
pub const MIN_HEIGHT: u16 = 25;

/// 头部省略服务器 URL 的宽度阈值。
const HEADER_COMPACT_WIDTH: u16 = 70;

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, chunks[0], app);
    render_progress(frame, chunks[1], app);
    render_body(frame, chunks[2], app);
    render_logs(frame, chunks[3], app);
    render_status(frame, chunks[4], app);

    if app.show_help {
        render_help(frame, area);
    }
}

/// 终端过小：居中提示，不渲染正常布局（避免挤压变形）。
fn render_too_small(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "终端尺寸过小",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "当前 {}×{}，建议至少 {}×{}",
                area.width, area.height, MIN_WIDTH, MIN_HEIGHT
            ),
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            "请调整窗口大小后继续",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(
        Paragraph::new(text).centered().wrap(Wrap { trim: true }),
        area,
    );
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

    let label = Style::default()
        .fg(Color::Gray)
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled("模式: ", label),
        Span::styled(app.mode_label, Style::default().fg(Color::Yellow)),
    ];

    // 窄终端下省略服务器地址，保证关键信息不被截断
    if inner.width >= HEADER_COMPACT_WIDTH {
        spans.push(Span::raw("  │  "));
        spans.push(Span::styled("服务器: ", label));
        spans.push(Span::styled(
            app.server_url.as_str(),
            Style::default().fg(Color::Green),
        ));
    }

    spans.push(Span::raw("  │  "));
    spans.push(Span::styled("并发: ", label));
    spans.push(Span::styled(
        app.concurrency.to_string(),
        Style::default().fg(Color::Green),
    ));

    if app.dry_run {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            " 预览模式 ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).centered(), inner);
}

fn render_progress(frame: &mut Frame, area: Rect, app: &App) {
    // 致命错误优先展示：程序已无法继续，提示用户退出
    if let Some(msg) = &app.fatal {
        let text = Line::from(vec![
            Span::styled("✗ ", Style::default().fg(Color::Red)),
            Span::styled(
                "发生错误",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(truncate(msg, 60), Style::default().fg(Color::Red)),
            Span::raw("  "),
            Span::styled("— 按 q 或 Esc 退出", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(text).centered(), area);
        return;
    }

    // 完成汇总：一轮结束后展示结果，等待用户手动退出
    if let Some(r) = &app.last_round {
        let (mark, title, title_style) = if r.interrupted {
            (
                "⏸ ",
                "已中断",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "✓ ",
                "处理完成",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        };
        let mut spans = vec![
            Span::styled(
                mark,
                Style::default().fg(if r.interrupted {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
            Span::styled(title, title_style),
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
        ];
        if r.interrupted && app.total > app.done {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("（未完成 {}/{}）", app.total - app.done, app.total),
                Style::default().fg(Color::Gray),
            ));
        }
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "— 按 q 或 Esc 退出",
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(Line::from(spans)).centered(), area);
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
            .gauge_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD));
        frame.render_widget(gauge, area);
        return;
    }

    // watch 模式处理中：文件陆续到来，无总量概念，展示活动任务与累计统计
    if app.watch_processing() {
        let text = Line::from(vec![
            Span::styled(
                format!("{} ", app.spinner_frame()),
                Style::default().fg(ACCENT),
            ),
            Span::styled(
                format!("正在处理 {} 个文件", app.tasks.len()),
                Style::default().fg(Color::Gray),
            ),
            Span::raw("  "),
            Span::styled(
                format!("✓ {}", app.stats.success),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                format!("✗ {}", app.stats.failed),
                Style::default().fg(Color::Red),
            ),
        ]);
        frame.render_widget(Paragraph::new(text), area);
        return;
    }

    // 空闲（watch 等待新文件 / once·cron 启动前）
    let idle_text = if app.mode_label.contains("监视") {
        "等待新视频文件..."
    } else {
        "准备中..."
    };
    let text = Line::from(vec![
        Span::styled(
            format!("{} ", app.spinner_frame()),
            Style::default().fg(ACCENT),
        ),
        Span::styled(idle_text, Style::default().fg(Color::Gray)),
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

    if app.tasks.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "暂无活动任务",
            Style::default().fg(Color::DarkGray),
        ))));
        frame.render_widget(List::new(items), inner);
        return;
    }

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

    // watch 模式无轮次总量，展示累计处理数而非 N/0
    let progress_line = if app.total == 0 {
        format!("累计    {}", app.done)
    } else {
        format!("进度    {}/{}", app.done, app.total)
    };

    let mut lines = vec![
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
            progress_line,
            Style::default().fg(Color::Gray),
        )]),
        Line::from(vec![
            Span::styled("运行    ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_duration(app.elapsed_secs()),
                Style::default().fg(Color::Gray),
            ),
        ]),
    ];

    if !app.recent_failures.is_empty() {
        lines.push(Line::from(Span::styled(
            "最近失败",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
        let width = inner.width.saturating_sub(4) as usize;
        for (filename, reason) in app.recent_failures.iter().take(RECENT_FAILURES_MAX) {
            let text = if width > filename.chars().count() + 3 {
                format!("{} — {}", filename, reason)
            } else {
                filename.clone()
            };
            lines.push(Line::from(Span::styled(
                truncate(&text, width.max(8)),
                Style::default().fg(Color::Red),
            )));
        }
    }

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
    let width = inner.width as usize;
    if vis == 0 || width == 0 || app.logs.is_empty() {
        return;
    }

    // 尾部窗口跟随 + 手动滚动偏移（按逻辑行）：
    // 每条日志先按显示宽度切分成行（首行带 时间戳/级别 前缀），
    // 从窗口末尾向前收集逻辑行直到填满视口显示行数；
    // 头部溢出的显示行用 scroll 跳过，保证内容底部对齐。
    let end = app.logs.len() - app.log_scroll_offset.min(app.logs.len());
    let mut window: Vec<Line<'_>> = Vec::new();
    let mut rows = 0usize;
    let mut idx = end;
    while idx > 0 && rows < vis {
        idx -= 1;
        let lines = build_log_lines(&app.logs[idx], width);
        rows += lines.len();
        window.extend(lines);
    }
    window.reverse();

    let scroll = rows.saturating_sub(vis) as u16;
    frame.render_widget(Paragraph::new(window).scroll((scroll, 0)), inner);
}

/// 单条日志 → 若干渲染行：首行带 时间戳/级别着色 前缀，
/// 超宽消息按显示宽度（中文占 2 列）续行，不用 ratatui 的 word wrap——
/// 中文消息无空格，word wrap 会整段下移且行数不可精确预估。
fn build_log_lines(log: &LogLine, width: usize) -> Vec<Line<'_>> {
    let (color, abbr) = level_style(log.level);
    let time = format!("{} ", log.time);
    let level = format!("[{:4}] ", abbr);
    let prefix_w = time.width() + level.width();

    let mut lines = Vec::new();
    let mut rest = log.message.as_str();
    let mut first = true;
    loop {
        let avail = if first {
            width.saturating_sub(prefix_w).max(1)
        } else {
            width.max(1)
        };
        let (chunk, remaining) = split_at_width(rest, avail);
        if first {
            lines.push(Line::from(vec![
                Span::styled(time.clone(), Style::default().fg(Color::DarkGray)),
                Span::styled(level.clone(), Style::default().fg(color)),
                Span::styled(chunk, Style::default().fg(Color::Gray)),
            ]));
            first = false;
        } else {
            lines.push(Line::from(Span::styled(
                chunk,
                Style::default().fg(Color::Gray),
            )));
        }
        if remaining.is_empty() {
            break;
        }
        rest = remaining;
    }
    lines
}

/// 按累计显示宽度切分字符串，返回 (头部, 余部)。
/// 首字符总是被取走（即使超宽），保证切分前进、不会死循环。
fn split_at_width(s: &str, max_w: usize) -> (&str, &str) {
    let mut w = 0;
    for (idx, ch) in s.char_indices() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > max_w && idx > 0 {
            return s.split_at(idx);
        }
        w += cw;
    }
    (s, "")
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " q 退出 ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw("  ↑↓ 滚动  PgUp/PgDn 翻页  ? 帮助 "),
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

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 帮助浮层：全屏居中的快捷键一览，`?` 切换。
fn render_help(frame: &mut Frame, area: Rect) {
    let w = (area.width * 3 / 5)
        .max(40)
        .min(area.width.saturating_sub(4));
    let h = 10u16.min(area.height.saturating_sub(4));
    let popup = center_rect(area, w, h);

    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let entries = [
        ("q / Esc", "退出程序"),
        ("Ctrl+C", "退出程序"),
        ("↑ / ↓", "逐行滚动日志"),
        ("PgUp / PgDn", "翻页滚动日志"),
        ("Home / End", "跳到最旧 / 最新日志"),
        ("?", "切换本帮助"),
    ];
    let lines: Vec<Line<'_>> = entries
        .iter()
        .map(|(k, desc)| {
            Line::from(vec![
                Span::styled(format!("{:<12}", k), key),
                Span::styled(*desc, Style::default().fg(Color::Gray)),
            ])
        })
        .collect();

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT))
                .title(" 帮助 "),
        ),
        popup,
    );
}

/// 在 `area` 内取居中的 w×h 矩形。
fn center_rect(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
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

/// 按字符数截断，尾部以省略号提示（宽度安全：中文按字符计）。
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    s.push('…');
    s
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
    use crate::tui::event::{AppEvent, FileStatus, Stage};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal as TestTerminal;

    fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = TestTerminal::new(backend).unwrap();
        terminal.draw(|f| render(app, f)).unwrap();
        let mut out = String::new();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .for_each(|c| out.push_str(c.symbol()));
        out
    }

    /// 宽字符（中文）在缓冲区中占 1 个 cell + 1 个填充 cell，
    /// 拼接后汉字之间会插入填充符号；此函数去掉空白便于断言。
    fn visible_text(app: &mut App) -> String {
        let rendered = render_to_string(app, 100, 30);
        rendered.split_whitespace().collect::<String>()
    }

    /// 指定尺寸渲染并去空白，用于中文断言。
    fn squeezed_text(app: &mut App, width: u16, height: u16) -> String {
        render_to_string(app, width, height)
            .split_whitespace()
            .collect::<String>()
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
            stage: Stage::Search,
        });
        app.handle_event(AppEvent::FileDone {
            filename: "SSIS-001.mp4".into(),
            status: FileStatus::Skipped,
            reason: None,
        });
        app.handle_event(AppEvent::FileStart {
            filename: "SSIS-002.mp4".into(),
        });
        app.handle_event(AppEvent::FileStage {
            filename: "SSIS-002.mp4".into(),
            stage: Stage::Detail,
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
            reason: None,
        });
        app.handle_event(AppEvent::FileDone {
            filename: "b.mp4".into(),
            status: FileStatus::Skipped,
            reason: None,
        });

        let out = render_to_string(&mut app, 100, 30);
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
            interrupted: false,
        });

        let out = visible_text(&mut app);
        assert!(out.contains("处理完成"));
        assert!(out.contains("2成功"));
        assert!(out.contains("1跳过"));
        // 完成后等待用户手动退出，应有按键提示
        assert!(out.contains("按q或Esc退出"));
    }

    #[test]
    fn test_render_interrupted_summary() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::RoundStart { total: 10 });
        app.handle_event(AppEvent::FileDone {
            filename: "a.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });
        app.handle_event(AppEvent::RoundDone {
            success: 1,
            skipped: 0,
            failed: 0,
            interrupted: true,
        });

        let out = visible_text(&mut app);
        assert!(out.contains("已中断"), "中断汇总应有明确标识");
        assert!(out.contains("未完成9/10"), "应显示未完成数量");
    }

    #[test]
    fn test_render_fatal_state() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::Fatal {
            message: "无法连接服务器".into(),
        });

        let out = visible_text(&mut app);
        assert!(out.contains("发生错误"));
        assert!(out.contains("无法连接服务器"));
        assert!(out.contains("按q或Esc退出"));
    }

    #[test]
    fn test_render_watch_processing_progress() {
        let mut app = App::new("文件监视", "srv".into());
        app.handle_event(AppEvent::WatchReady {
            path: std::path::PathBuf::from("D:/videos"),
        });
        app.handle_event(AppEvent::FileStart {
            filename: "SSIS-100.mp4".into(),
        });
        app.handle_event(AppEvent::FileDone {
            filename: "SSIS-099.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });

        let out = visible_text(&mut app);
        assert!(
            out.contains("正在处理1个文件"),
            "watch 处理中应显示活动任务数: {}",
            out
        );
        assert!(out.contains("✓1"), "应显示累计成功数");
    }

    #[test]
    fn test_render_stats_cumulative_when_no_total() {
        let mut app = App::new("文件监视", "srv".into());
        app.handle_event(AppEvent::FileDone {
            filename: "a.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });
        app.handle_event(AppEvent::FileDone {
            filename: "b.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });

        let out = visible_text(&mut app);
        assert!(out.contains("累计2"), "无总量时应显示累计数: {}", out);
        assert!(!out.contains("2/0"), "不应出现除零式进度");
    }

    #[test]
    fn test_render_recent_failures() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::RoundStart { total: 2 });
        app.handle_event(AppEvent::FileDone {
            filename: "ABC-123.mp4".into(),
            status: FileStatus::Failed,
            reason: Some("网络超时".into()),
        });

        let out = visible_text(&mut app);
        assert!(out.contains("最近失败"));
        assert!(out.contains("ABC-123.mp4"));
        assert!(out.contains("网络超时"));
    }

    #[test]
    fn test_render_dry_run_badge_and_concurrency() {
        let mut app = App::new("单次扫描", "srv".into()).with_meta(true, 6);
        let out = visible_text(&mut app);
        assert!(out.contains("预览模式"), "dry-run 应有醒目徽章");
        assert!(out.contains("并发:6"));
    }

    #[test]
    fn test_render_empty_tasks_placeholder() {
        let mut app = App::new("单次扫描", "srv".into());
        let out = visible_text(&mut app);
        assert!(out.contains("暂无活动任务"));
    }

    #[test]
    fn test_render_logs_with_level_colors() {
        let mut app = App::new("文件监视", "srv".into());
        app.handle_event(AppEvent::Log {
            level: Level::INFO,
            message: "hello tui".into(),
        });
        let out = render_to_string(&mut app, 100, 30);
        assert!(out.contains("hello tui"));
        assert!(out.contains("[INFO]"));
    }

    #[test]
    fn test_render_logs_wrap_long_line() {
        let mut app = App::new("单次扫描", "srv".into());
        // 超过日志面板宽度的长路径，按显示宽度续行后尾部内容仍应可见
        let long_msg = format!("{}-尾部标记", "很长的文件路径".repeat(30));
        app.handle_event(AppEvent::Log {
            level: Level::INFO,
            message: long_msg,
        });

        let out = squeezed_text(&mut app, 80, 30);
        assert!(out.contains("尾部标记"), "长日志续行后尾部应可见: {}", out);
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
    fn test_render_too_small_terminal() {
        let mut app = App::new("单次扫描", "srv".into());
        app.handle_event(AppEvent::Log {
            level: Level::INFO,
            message: "不应出现".into(),
        });

        let out = squeezed_text(&mut app, 40, 20);
        assert!(out.contains("终端尺寸过小"));
        assert!(out.contains("40×20"));
        assert!(!out.contains("不应出现"), "过小时不应渲染正常布局");
    }

    #[test]
    fn test_render_help_overlay() {
        let mut app = App::new("单次扫描", "srv".into());
        app.show_help = true;
        let out = visible_text(&mut app);
        assert!(out.contains("帮助"));
        assert!(out.contains("退出程序"));
        assert!(out.contains("翻页滚动日志"));
    }

    #[test]
    fn test_render_header_compact_hides_server() {
        let mut app = App::new("单次扫描", "http://localhost:8080".into());
        // 宽度低于阈值：服务器省略，模式保留
        let out = squeezed_text(&mut app, 60, 30);
        assert!(out.contains("单次扫描"));
        assert!(!out.contains("localhost:8080"), "窄终端应省略服务器地址");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "00:00");
        assert_eq!(format_duration(65), "01:05");
        assert_eq!(format_duration(3600), "01:00:00");
        assert_eq!(format_duration(3661), "01:01:01");
    }

    #[test]
    fn test_truncate_counts_chars() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 5), "abcd…");
        let cjk = "演员标准化目录名称很长很长很长";
        let cut = truncate(cjk, 6);
        assert!(cut.chars().count() <= 6);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn test_split_at_width_by_display_columns() {
        assert_eq!(split_at_width("abc", 10), ("abc", ""));
        // 中文每字占 2 列：4 列容量只装下 2 个字
        assert_eq!(split_at_width("演员名", 4), ("演员", "名"));
        assert_eq!(split_at_width("演员名", 5), ("演员", "名"));
        // 容量为 0 时首字符仍被取走，保证前进
        assert_eq!(split_at_width("abc", 0), ("a", "bc"));
        assert_eq!(split_at_width("演员", 1), ("演", "员"));
    }

    #[test]
    fn test_build_log_lines_wraps_by_prefix_width() {
        let log = LogLine {
            time: "12:00:00".into(),
            level: Level::INFO,
            message: "演员标准化: 长名字".into(),
        };
        // 消息总宽 18 列；前缀占 16 列，宽 18 的面板首行只放得下 1 个字，其余续行
        let lines = build_log_lines(&log, 18);
        assert_eq!(lines.len(), 2);
        // 宽面板下单行放下
        let single = build_log_lines(&log, 40);
        assert_eq!(single.len(), 1);
    }
}
