/// 业务侧 → TUI 的事件模型与上报接口。
/// 业务代码只依赖这里的 `Reporter`，不感知渲染细节；
/// 非 TTY / `--no-tui` 模式下使用 `NoopReporter`，行为退化为纯文本日志。
use chrono::{DateTime, Local};
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use tracing::Level;

/// 单个文件的处理阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Search,
    Detail,
    Normalize,
    Move,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Search => "搜索影片",
            Stage::Detail => "获取详情",
            Stage::Normalize => "标准化演员",
            Stage::Move => "移动文件",
        }
    }
}

/// 单个文件的最终结果（细节原因经 tracing 日志进入日志面板）。
#[derive(Debug, Clone)]
pub enum FileStatus {
    Success,
    Skipped,
    Failed,
}

/// TUI 渲染线程消费的事件。
#[derive(Debug)]
pub enum AppEvent {
    ScanStart,
    ScanProgress {
        dirs: usize,
    },
    ScanDone,
    RoundStart {
        total: usize,
    },
    FileStart {
        filename: String,
    },
    FileStage {
        filename: String,
        stage: Stage,
    },
    FileDone {
        filename: String,
        status: FileStatus,
        /// 失败时的原因摘要（完整原因经 tracing 日志进入日志面板）。
        reason: Option<String>,
    },
    RoundDone {
        success: u32,
        skipped: u32,
        failed: u32,
        /// 用户中途退出导致的中断结束。
        interrupted: bool,
    },
    NextSchedule {
        at: DateTime<Local>,
    },
    WatchReady {
        path: PathBuf,
    },
    /// 业务致命错误（如初始化失败）：UI 停留展示错误而非无提示等待。
    Fatal {
        message: String,
    },
    Log {
        level: Level,
        message: String,
    },
    Shutdown,
}

/// 业务进度上报接口。
pub trait Reporter: Send + Sync {
    fn emit(&self, event: AppEvent);
}

/// 失败原因摘要的最大字符数。
const REASON_MAX_CHARS: usize = 120;

/// 从错误链提取失败原因摘要（截断），供 `FileDone` 上报展示。
pub fn failure_reason(e: &anyhow::Error) -> String {
    let msg = format!("{:#}", e);
    msg.chars().take(REASON_MAX_CHARS).collect()
}

/// 非 TUI 模式（纯文本日志）与测试使用的空实现。
pub struct NoopReporter;

impl Reporter for NoopReporter {
    fn emit(&self, _event: AppEvent) {}
}

/// 通道背书的实现：发送方从不阻塞，通道满或接收端已关闭时静默丢弃。
pub struct ChannelReporter {
    tx: SyncSender<AppEvent>,
}

impl ChannelReporter {
    pub fn new(tx: SyncSender<AppEvent>) -> Self {
        Self { tx }
    }
}

impl Reporter for ChannelReporter {
    fn emit(&self, event: AppEvent) {
        let _ = self.tx.try_send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 断言 ChannelReporter 在接收端存活时事件可达。
    #[test]
    fn test_channel_reporter_delivers_event() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AppEvent>(4);
        let reporter = ChannelReporter::new(tx);
        reporter.emit(AppEvent::ScanStart);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::ScanStart)));
    }

    /// 接收端关闭后发送不 panic、不阻塞。
    #[test]
    fn test_channel_reporter_drops_after_receiver_closed() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AppEvent>(4);
        let reporter = ChannelReporter::new(tx);
        drop(rx);
        reporter.emit(AppEvent::Shutdown);
    }

    /// 通道满时丢弃事件而不是阻塞业务线程。
    #[test]
    fn test_channel_reporter_drops_when_full() {
        let (tx, _rx) = std::sync::mpsc::sync_channel::<AppEvent>(1);
        let reporter = ChannelReporter::new(tx);
        reporter.emit(AppEvent::ScanStart);
        reporter.emit(AppEvent::ScanStart); // 满了，应被丢弃
    }

    #[test]
    fn test_stage_labels() {
        assert_eq!(Stage::Search.label(), "搜索影片");
        assert_eq!(Stage::Detail.label(), "获取详情");
        assert_eq!(Stage::Normalize.label(), "标准化演员");
        assert_eq!(Stage::Move.label(), "移动文件");
    }

    #[test]
    fn test_noop_reporter_swallows_events() {
        NoopReporter.emit(AppEvent::Shutdown);
    }
}
