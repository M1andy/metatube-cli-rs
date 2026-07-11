use chrono::Local;
use std::fmt;
use tracing::Level;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};

pub struct CleanFormat;

impl<S, N> FormatEvent<S, N> for CleanFormat
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
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
