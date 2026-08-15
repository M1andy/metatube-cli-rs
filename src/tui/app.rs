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
    pub next_schedule: Option<DateTime<Local>>,
    pub watch_dir: Option<PathBuf>,
    pub logs: VecDeque<LogLine>,
    /// 日志视图距底部的行数偏移（0 = 跟随最新）。
    pub log_scroll_offset: usize,
    /// 渲染 tick，驱动 spinner 动画。
    pub tick: usize,
    pub exit: bool,
}

impl App {
    pub fn new(mode_label: &'static str, server_url: String) -> Self {
        Self {
            mode_label,
            server_url,
            scanning: false,
            scan_dirs: 0,
            total: 0,
            done: 0,
            round_started_at: None,
            round_active: false,
            tasks: Vec::new(),
            stats: Stats::default(),
            last_round: None,
            next_schedule: None,
            watch_dir: None,
            logs: VecDeque::new(),
            log_scroll_offset: 0,
            tick: 0,
            exit: false,
        }
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
            AppEvent::FileDone { filename, status } => {
                self.tasks.retain(|t| t.filename != filename);
                self.done += 1;
                match status {
                    FileStatus::Success => self.stats.success += 1,
                    FileStatus::Skipped => self.stats.skipped += 1,
                    FileStatus::Failed => self.stats.failed += 1,
                }
            }
            AppEvent::RoundDone {
                success,
                skipped,
                failed,
            } => {
                self.round_active = false;
                self.last_round = Some(RoundSummary {
                    success,
                    skipped,
                    failed,
                });
            }
            AppEvent::NextSchedule { at } => {
                self.next_schedule = Some(at);
            }
            AppEvent::WatchReady { path } => {
                self.watch_dir = Some(path);
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
        });
        assert_eq!(app.stats.failed, 1);
        assert_eq!(app.done, 1);
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
        });
        assert!(!app.round_active);
        let r = app.last_round.as_ref().unwrap();
        assert_eq!((r.success, r.skipped, r.failed), (1, 1, 0));
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
        });
        assert!(app.eta_secs().is_some());

        // 全部完成后不再估算
        for i in 0..9 {
            app.handle_event(AppEvent::FileDone {
                filename: format!("f{i}.mp4"),
                status: FileStatus::Skipped,
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
}
