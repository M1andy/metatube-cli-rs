/// TUI 应用状态机：消费 `AppEvent`，维护渲染所需的全部状态。
/// 不含任何渲染逻辑，便于单元测试。
use super::event::{AppEvent, FileStatus, Stage};
use chrono::{DateTime, Local};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;
use tracing::Level;

/// 日志环形缓冲容量。
const LOG_BUFFER_MAX: usize = 500;

/// 统计面板保留的最近失败条数。
pub const RECENT_FAILURES_MAX: usize = 3;

/// 日志翻页滚动步长（逻辑行）。
const LOG_PAGE_SCROLL: usize = 10;

/// 盲文 spinner 帧（非 ASCII）。
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 活动任务列表最多渲染的条数，超出部分汇总显示。
pub const TASKS_VISIBLE_MAX: usize = 8;

#[derive(Debug)]
pub struct TaskState {
    pub filename: String,
    pub stage: Option<Stage>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub success: u32,
    pub skipped: u32,
    pub failed: u32,
}

#[derive(Debug)]
pub struct RoundSummary {
    pub success: u32,
    pub skipped: u32,
    pub failed: u32,
    /// 用户中途退出导致的中断结束。
    pub interrupted: bool,
}

#[derive(Debug)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub message: String,
}

#[derive(Debug)]
pub struct App {
    /// 运行模式显示名（单次扫描 / 定时执行 / 文件监视）。
    pub mode_label: &'static str,
    pub server_url: String,
    /// 预览模式（dry-run），头部显示醒目徽章。
    pub dry_run: bool,
    /// 并发处理数。
    pub concurrency: usize,
    /// 是否正在扫描目录。
    pub scanning: bool,
    pub scan_dirs: usize,
    /// 当前轮进度。
    pub total: usize,
    pub done: usize,
    pub round_started_at: Option<Instant>,
    pub round_active: bool,
    /// 活动任务（处理中的文件）。
    pub tasks: Vec<TaskState>,
    pub stats: Stats,
    pub last_round: Option<RoundSummary>,
    /// 最近失败的文件与原因摘要（新失败在尾部）。
    pub recent_failures: VecDeque<(String, String)>,
    /// 业务致命错误，进度区优先展示。
    pub fatal: Option<String>,
    pub next_schedule: Option<DateTime<Local>>,
    pub watch_dir: Option<PathBuf>,
    pub logs: VecDeque<LogLine>,
    /// 日志视图距底部的行数偏移（0 = 跟随最新）。
    pub log_scroll_offset: usize,
    /// 帮助浮层开关。
    pub show_help: bool,
    /// 渲染 tick，驱动 spinner 动画。
    pub tick: usize,
    /// 会话启动时刻，用于展示运行时长。
    pub started_at: Instant,
    pub exit: bool,
}

impl App {
    pub fn new(mode_label: &'static str, server_url: String) -> Self {
        Self {
            mode_label,
            server_url,
            dry_run: false,
            concurrency: 4,
            scanning: false,
            scan_dirs: 0,
            total: 0,
            done: 0,
            round_started_at: None,
            round_active: false,
            tasks: Vec::new(),
            stats: Stats::default(),
            last_round: None,
            recent_failures: VecDeque::new(),
            fatal: None,
            next_schedule: None,
            watch_dir: None,
            logs: VecDeque::new(),
            log_scroll_offset: 0,
            show_help: false,
            tick: 0,
            started_at: Instant::now(),
            exit: false,
        }
    }

    /// 附加展示元信息（dry-run / 并发数）。
    pub fn with_meta(mut self, dry_run: bool, concurrency: usize) -> Self {
        self.dry_run = dry_run;
        self.concurrency = concurrency;
        self
    }

    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::ScanStart => {
                self.scanning = true;
                self.scan_dirs = 0;
            }
            AppEvent::ScanProgress { dirs } => {
                self.scan_dirs = dirs;
            }
            AppEvent::ScanDone => {
                self.scanning = false;
            }
            AppEvent::RoundStart { total } => {
                self.total = total;
                self.done = 0;
                self.round_started_at = Some(Instant::now());
                self.round_active = true;
                self.tasks.clear();
                self.stats = Stats::default();
                self.last_round = None;
            }
            AppEvent::FileStart { filename } => {
                self.tasks.push(TaskState {
                    filename,
                    stage: None,
                });
            }
            AppEvent::FileStage { filename, stage } => {
                if let Some(task) = self.tasks.iter_mut().find(|t| t.filename == filename) {
                    task.stage = Some(stage);
                }
            }
            AppEvent::FileDone {
                filename,
                status,
                reason,
            } => {
                // 只移除第一个同名任务：不同目录的同名文件各自独立计数
                if let Some(idx) = self.tasks.iter().position(|t| t.filename == filename) {
                    self.tasks.swap_remove(idx);
                }
                self.done += 1;
                match status {
                    FileStatus::Success => self.stats.success += 1,
                    FileStatus::Skipped => self.stats.skipped += 1,
                    FileStatus::Failed => {
                        self.stats.failed += 1;
                        self.recent_failures.push_back((
                            filename,
                            reason.unwrap_or_else(|| "未知原因".to_string()),
                        ));
                        while self.recent_failures.len() > RECENT_FAILURES_MAX {
                            self.recent_failures.pop_front();
                        }
                    }
                }
            }
            AppEvent::RoundDone {
                success,
                skipped,
                failed,
                interrupted,
            } => {
                self.round_active = false;
                self.tasks.clear();
                self.last_round = Some(RoundSummary {
                    success,
                    skipped,
                    failed,
                    interrupted,
                });
            }
            AppEvent::NextSchedule { at } => {
                self.next_schedule = Some(at);
            }
            AppEvent::WatchReady { path } => {
                self.watch_dir = Some(path);
            }
            AppEvent::Fatal { message } => {
                self.fatal = Some(message);
            }
            AppEvent::Log { level, message } => {
                self.logs.push_back(LogLine {
                    time: Local::now().format("%H:%M:%S").to_string(),
                    level,
                    message,
                });
                if self.logs.len() > LOG_BUFFER_MAX {
                    self.logs.pop_front();
                }
            }
            AppEvent::Shutdown => {
                self.exit = true;
            }
        }
    }

    /// 当前 spinner 字符。
    pub fn spinner_frame(&self) -> &'static str {
        SPINNER_FRAMES[self.tick % SPINNER_FRAMES.len()]
    }

    /// 基于已完成进度与耗时估算剩余时间；无进度时返回 None。
    pub fn eta_secs(&self) -> Option<u64> {
        let started = self.round_started_at?;
        if self.done == 0 || self.total <= self.done {
            return None;
        }
        let elapsed = started.elapsed().as_secs_f64();
        let per_item = elapsed / self.done as f64;
        let remaining = (self.total - self.done) as f64 * per_item;
        Some(remaining.round() as u64)
    }

    /// watch 模式没有轮次概念：有活动任务即视为处理中。
    pub fn watch_processing(&self) -> bool {
        !self.round_active && !self.scanning && !self.tasks.is_empty()
    }

    /// 会话运行时长（秒）。
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// 日志向上滚动一行（查看更早的内容）。
    pub fn scroll_logs_up(&mut self) {
        let max_offset = self.logs.len().saturating_sub(1);
        if self.log_scroll_offset < max_offset {
            self.log_scroll_offset += 1;
        }
    }

    /// 日志向下滚动一行；回到 0 时恢复跟随最新。
    pub fn scroll_logs_down(&mut self) {
        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(1);
    }

    /// 日志向上翻页。
    pub fn scroll_logs_page_up(&mut self) {
        let max_offset = self.logs.len().saturating_sub(1);
        self.log_scroll_offset = (self.log_scroll_offset + LOG_PAGE_SCROLL).min(max_offset);
    }

    /// 日志向下翻页。
    pub fn scroll_logs_page_down(&mut self) {
        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(LOG_PAGE_SCROLL);
    }

    /// 日志跳到最旧一条。
    pub fn scroll_logs_top(&mut self) {
        self.log_scroll_offset = self.logs.len().saturating_sub(1);
    }

    /// 日志回到最新并恢复跟随。
    pub fn scroll_logs_bottom(&mut self) {
        self.log_scroll_offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::event::AppEvent;

    fn app() -> App {
        App::new("单次扫描", "http://localhost:8080".into())
    }

    #[test]
    fn test_scan_lifecycle() {
        let mut app = app();
        app.handle_event(AppEvent::ScanStart);
        assert!(app.scanning);
        app.handle_event(AppEvent::ScanProgress { dirs: 5 });
        assert_eq!(app.scan_dirs, 5);
        app.handle_event(AppEvent::ScanDone);
        assert!(!app.scanning);
    }

    #[test]
    fn test_round_resets_state() {
        let mut app = app();
        app.handle_event(AppEvent::RoundStart { total: 10 });
        app.handle_event(AppEvent::FileStart {
            filename: "a.mp4".into(),
        });
        app.handle_event(AppEvent::FileDone {
            filename: "a.mp4".into(),
            status: FileStatus::Skipped,
            reason: None,
        });
        assert_eq!(app.done, 1);
        assert_eq!(app.stats.skipped, 1);

        // 新一轮应清零
        app.handle_event(AppEvent::RoundStart { total: 4 });
        assert_eq!(app.total, 4);
        assert_eq!(app.done, 0);
        assert!(app.tasks.is_empty());
        assert_eq!(app.stats, Stats::default());
        assert!(app.round_active);
    }

    #[test]
    fn test_task_lifecycle_and_stats() {
        let mut app = app();
        app.handle_event(AppEvent::RoundStart { total: 3 });
        app.handle_event(AppEvent::FileStart {
            filename: "SSIS-001.mp4".into(),
        });
        assert_eq!(app.tasks.len(), 1);
        assert!(app.tasks[0].stage.is_none());

        app.handle_event(AppEvent::FileStage {
            filename: "SSIS-001.mp4".into(),
            stage: Stage::Search,
        });
        assert_eq!(app.tasks[0].stage, Some(Stage::Search));

        app.handle_event(AppEvent::FileStage {
            filename: "SSIS-001.mp4".into(),
            stage: Stage::Move,
        });
        assert_eq!(app.tasks[0].stage, Some(Stage::Move));

        app.handle_event(AppEvent::FileDone {
            filename: "SSIS-001.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });
        assert!(app.tasks.is_empty());
        assert_eq!(app.done, 1);
        assert_eq!(app.stats.success, 1);

        // 未知文件的 stage 事件不应插入新任务
        app.handle_event(AppEvent::FileStage {
            filename: "ghost.mp4".into(),
            stage: Stage::Detail,
        });
        assert!(app.tasks.is_empty());
    }

    #[test]
    fn test_failed_status_counts() {
        let mut app = app();
        app.handle_event(AppEvent::RoundStart { total: 1 });
        app.handle_event(AppEvent::FileStart {
            filename: "bad.mp4".into(),
        });
        app.handle_event(AppEvent::FileDone {
            filename: "bad.mp4".into(),
            status: FileStatus::Failed,
            reason: Some("网络超时".into()),
        });
        assert_eq!(app.stats.failed, 1);
        assert_eq!(app.done, 1);
        assert_eq!(app.recent_failures.len(), 1);
        assert_eq!(app.recent_failures[0].0, "bad.mp4");
        assert_eq!(app.recent_failures[0].1, "网络超时");
    }

    #[test]
    fn test_round_done_summary() {
        let mut app = app();
        app.handle_event(AppEvent::RoundStart { total: 2 });
        assert!(app.round_active);
        app.handle_event(AppEvent::RoundDone {
            success: 1,
            skipped: 1,
            failed: 0,
            interrupted: false,
        });
        assert!(!app.round_active);
        let r = app.last_round.as_ref().unwrap();
        assert_eq!((r.success, r.skipped, r.failed), (1, 1, 0));
        assert!(!r.interrupted);
    }

    #[test]
    fn test_log_ring_buffer_cap() {
        let mut app = app();
        for i in 0..(LOG_BUFFER_MAX + 100) {
            app.handle_event(AppEvent::Log {
                level: Level::INFO,
                message: format!("msg-{i}"),
            });
        }
        assert_eq!(app.logs.len(), LOG_BUFFER_MAX);
        assert_eq!(app.logs.front().unwrap().message, "msg-100");
        assert_eq!(
            app.logs.back().unwrap().message,
            format!("msg-{}", LOG_BUFFER_MAX + 99)
        );
    }

    #[test]
    fn test_log_scroll_bounds() {
        let mut app = app();
        for i in 0..10 {
            app.handle_event(AppEvent::Log {
                level: Level::INFO,
                message: format!("m{i}"),
            });
        }
        assert_eq!(app.log_scroll_offset, 0);
        for _ in 0..20 {
            app.scroll_logs_up();
        }
        // 最大只能滚到第一条
        assert_eq!(app.log_scroll_offset, 9);
        for _ in 0..20 {
            app.scroll_logs_down();
        }
        assert_eq!(app.log_scroll_offset, 0);
    }

    #[test]
    fn test_shutdown_sets_exit() {
        let mut app = app();
        assert!(!app.exit);
        app.handle_event(AppEvent::Shutdown);
        assert!(app.exit);
    }

    #[test]
    fn test_spinner_frames_cycle() {
        let mut app = app();
        let first = app.spinner_frame();
        for _ in 0..SPINNER_FRAMES.len() {
            app.tick += 1;
        }
        assert_eq!(app.spinner_frame(), first);
        assert_ne!(app.spinner_frame(), SPINNER_FRAMES[1]);
    }

    #[test]
    fn test_eta_requires_progress() {
        let mut app = app();
        assert_eq!(app.eta_secs(), None);

        app.handle_event(AppEvent::RoundStart { total: 10 });
        assert_eq!(app.eta_secs(), None); // done == 0

        app.handle_event(AppEvent::FileDone {
            filename: "x.mp4".into(),
            status: FileStatus::Skipped,
            reason: None,
        });
        assert!(app.eta_secs().is_some());

        // 全部完成后不再估算
        for i in 0..9 {
            app.handle_event(AppEvent::FileDone {
                filename: format!("f{i}.mp4"),
                status: FileStatus::Skipped,
                reason: None,
            });
        }
        assert_eq!(app.eta_secs(), None);
    }

    #[test]
    fn test_watch_ready_and_next_schedule() {
        let mut app = App::new("文件监视", "srv".into());
        app.handle_event(AppEvent::WatchReady {
            path: PathBuf::from("D:/videos"),
        });
        assert_eq!(app.watch_dir, Some(PathBuf::from("D:/videos")));

        let at = Local::now();
        app.handle_event(AppEvent::NextSchedule { at });
        assert_eq!(app.next_schedule, Some(at));
    }

    #[test]
    fn test_file_done_removes_first_matching_task_only() {
        // 不同目录的同名文件：一次 FileDone 只结束一个任务
        let mut app = app();
        app.handle_event(AppEvent::FileStart {
            filename: "a.mp4".into(),
        });
        app.handle_event(AppEvent::FileStart {
            filename: "a.mp4".into(),
        });
        app.handle_event(AppEvent::FileDone {
            filename: "a.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.done, 1);
    }

    #[test]
    fn test_recent_failures_capped() {
        let mut app = app();
        for i in 0..(RECENT_FAILURES_MAX + 2) {
            app.handle_event(AppEvent::FileDone {
                filename: format!("f{i}.mp4"),
                status: FileStatus::Failed,
                reason: Some(format!("原因{i}")),
            });
        }
        assert_eq!(app.recent_failures.len(), RECENT_FAILURES_MAX);
        // 最旧的被挤出，保留最近几条
        let names: Vec<&str> = app
            .recent_failures
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            names[0],
            format!("f{}.mp4", RECENT_FAILURES_MAX - 1).as_str()
        );
        // 失败无原因时使用占位文本
        app.handle_event(AppEvent::FileDone {
            filename: "g.mp4".into(),
            status: FileStatus::Failed,
            reason: None,
        });
        assert_eq!(app.recent_failures.back().unwrap().1, "未知原因");
    }

    #[test]
    fn test_round_done_interrupted_clears_tasks() {
        let mut app = app();
        app.handle_event(AppEvent::RoundStart { total: 5 });
        app.handle_event(AppEvent::FileStart {
            filename: "a.mp4".into(),
        });
        app.handle_event(AppEvent::FileStart {
            filename: "b.mp4".into(),
        });
        app.handle_event(AppEvent::RoundDone {
            success: 1,
            skipped: 0,
            failed: 0,
            interrupted: true,
        });
        assert!(!app.round_active);
        assert!(app.tasks.is_empty());
        assert!(app.last_round.as_ref().unwrap().interrupted);
    }

    #[test]
    fn test_watch_processing_state() {
        let mut app = App::new("文件监视", "srv".into());
        assert!(!app.watch_processing());
        app.handle_event(AppEvent::FileStart {
            filename: "a.mp4".into(),
        });
        assert!(app.watch_processing());
        app.handle_event(AppEvent::FileDone {
            filename: "a.mp4".into(),
            status: FileStatus::Success,
            reason: None,
        });
        assert!(!app.watch_processing());

        // 轮次进行中不属于 watch 处理态
        app.handle_event(AppEvent::RoundStart { total: 2 });
        app.handle_event(AppEvent::FileStart {
            filename: "b.mp4".into(),
        });
        assert!(!app.watch_processing());
    }

    #[test]
    fn test_fatal_event_sets_message() {
        let mut app = app();
        assert!(app.fatal.is_none());
        app.handle_event(AppEvent::Fatal {
            message: "无法连接服务器".into(),
        });
        assert_eq!(app.fatal.as_deref(), Some("无法连接服务器"));
    }

    #[test]
    fn test_log_scroll_page_and_top_bottom() {
        let mut app = app();
        for i in 0..30 {
            app.handle_event(AppEvent::Log {
                level: Level::INFO,
                message: format!("m{i}"),
            });
        }
        app.scroll_logs_page_up();
        assert_eq!(app.log_scroll_offset, 10);
        app.scroll_logs_page_up();
        assert_eq!(app.log_scroll_offset, 20);
        app.scroll_logs_top();
        assert_eq!(app.log_scroll_offset, 29);
        app.scroll_logs_page_down();
        assert_eq!(app.log_scroll_offset, 19);
        app.scroll_logs_bottom();
        assert_eq!(app.log_scroll_offset, 0);
    }

    #[test]
    fn test_with_meta_sets_display_fields() {
        let app = App::new("单次扫描", "srv".into()).with_meta(true, 8);
        assert!(app.dry_run);
        assert_eq!(app.concurrency, 8);
        assert!(app.elapsed_secs() < 5);
    }
}
