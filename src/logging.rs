/// 日志初始化与格式化。
/// - 纯文本模式：`CleanFormat` 彩色输出到 stdout（原有行为）。
/// - TUI 模式：`TuiLogLayer` 将 tracing 事件转发到 TUI 日志面板，
///   发送非阻塞，通道满/关闭时静默丢弃。
use crate::tui::event::AppEvent;
use chrono::Local;
use std::fmt;
use std::sync::mpsc::SyncSender;
use tracing::field::Visit;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::Context;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

pub struct CleanFormat;

impl<S, N> FormatEvent<S, N> for CleanFormat
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let now = Local::now();
        write!(writer, "{} ", now.format("%Y-%m-%d %H:%M:%S"))?;

        let meta = event.metadata();
        let level = *meta.level();
        let ansi = writer.has_ansi_escapes();

        if ansi {
            let color = match level {
                Level::ERROR => "\x1b[31m", // red
                Level::WARN => "\x1b[33m",  // yellow
                Level::INFO => "\x1b[32m",  // green
                Level::DEBUG => "\x1b[34m", // blue
                Level::TRACE => "\x1b[35m", // magenta
            };
            write!(writer, "{}[{:4}]\x1b[0m ", color, abbr(level))?;
        } else {
            write!(writer, "[{:4}] ", abbr(level))?;
        }

        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn abbr(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERRO",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBG",
        Level::TRACE => "TRCE",
    }
}

/// 将 tracing 事件（级别 + message 字段）转发进 TUI 事件通道。
pub struct TuiLogLayer {
    tx: SyncSender<AppEvent>,
}

impl TuiLogLayer {
    pub fn new(tx: SyncSender<AppEvent>) -> Self {
        Self { tx }
    }
}

/// 收集事件 `message` 字段的访问器。
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

impl<S: Subscriber> Layer<S> for TuiLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);
        if visitor.message.is_empty() {
            return;
        }
        let _ = self.tx.try_send(AppEvent::Log {
            level: *event.metadata().level(),
            message: visitor.message,
        });
    }
}

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// 非 TUI 模式初始化：彩色日志到 stdout。
pub fn init_plain() {
    tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .event_format(CleanFormat)
        .init();
}

/// TUI 模式初始化：日志转发到 TUI 日志面板。
pub fn init_tui(tx: SyncSender<AppEvent>) {
    tracing_subscriber::registry()
        .with(env_filter())
        .with(TuiLogLayer::new(tx))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_log_layer_forwards_message_and_level() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AppEvent>(8);
        let subscriber = tracing_subscriber::registry().with(TuiLogLayer::new(tx));
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!("hello {}", "tui");
        tracing::warn!("小心");

        let events: Vec<AppEvent> = (0..2).filter_map(|_| rx.try_recv().ok()).collect();
        assert_eq!(events.len(), 2);
        for ev in &events {
            if let AppEvent::Log { level, message } = ev {
                assert!(!message.is_empty());
                assert!(matches!(*level, Level::INFO | Level::WARN));
            } else {
                panic!("应为 Log 事件");
            }
        }
    }

    #[test]
    fn test_tui_log_layer_drops_when_channel_full() {
        let (tx, _rx) = std::sync::mpsc::sync_channel::<AppEvent>(1);
        let subscriber = tracing_subscriber::registry().with(TuiLogLayer::new(tx));
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!("first");
        tracing::info!("second"); // 通道满，应被丢弃而非阻塞
        tracing::info!("third");
    }

    #[test]
    fn test_tui_log_layer_ignores_field_only_events() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AppEvent>(8);
        let subscriber = tracing_subscriber::registry().with(TuiLogLayer::new(tx));
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(count = 5, "有消息");
        assert!(rx.try_recv().is_ok());

        // 纯字段事件（无 message 字段）不应转发
        tracing::info!(count = 7);
        assert!(rx.try_recv().is_err());
    }
}
