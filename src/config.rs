/// CLI argument parsing with config.toml support.
/// Priority: CLI args > env vars > config.toml > defaults.
use anyhow::{bail, Context};
use clap::Parser;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Run mode: single scan, scheduled, or file-system watch.
#[derive(Debug, Clone, clap::ValueEnum, PartialEq)]
pub enum RunMode {
    /// Run once and exit.
    Once,
    /// Run on a cron schedule.
    Cron,
    /// Watch the download directory for new files.
    Watch,
}

impl RunMode {
    pub fn label(&self) -> &'static str {
        match self {
            RunMode::Once => "单次扫描",
            RunMode::Cron => "定时执行",
            RunMode::Watch => "文件监视",
        }
    }
}

/// CLI args (all optional during initial parse, validated after merge).
#[derive(Parser, Debug)]
#[command(name = "metatube-cli-rs")]
#[command(about = "Organize JAV videos by actress using MetaTube SDK")]
#[command(version)]
struct RawConfig {
    /// Run mode: once, cron, or watch
    #[arg(long, env = "MODE")]
    mode: Option<RunMode>,

    /// Directory to scan for video files
    #[arg(long, env = "JAV_DOWNLOAD", value_hint = clap::ValueHint::DirPath)]
    jav_download: Option<PathBuf>,

    /// Output directory for organized videos
    #[arg(long, env = "JAV_OUTPUT", value_hint = clap::ValueHint::DirPath)]
    jav_output: Option<PathBuf>,

    /// MetaTube server URL
    #[arg(long, env = "SERVER_URL")]
    server_url: Option<String>,

    /// Bearer token for API authentication
    #[arg(long, env = "TOKEN")]
    token: Option<String>,

    /// Minimum file size in MB (default 300)
    #[arg(long, env = "MIN_SIZE_MB")]
    min_size_mb: Option<u64>,

    /// Cron expression for scheduled runs
    #[arg(long, env = "CRON")]
    cron: Option<String>,

    /// Maximum concurrent file processing (default 4)
    #[arg(long, env = "CONCURRENCY")]
    concurrency: Option<usize>,

    /// Dry-run: show what would be done without moving files
    #[arg(long)]
    dry_run: bool,

    /// HTTP(S) proxy URL (e.g. http://localhost:7890)
    #[arg(long, env = "PROXY")]
    proxy: Option<String>,

    /// Path to config.toml (skips auto-discovery)
    #[arg(long, env = "CONFIG")]
    config_path: Option<PathBuf>,

    /// Directory for failed videos (default: <jav_output>/../JAV_failed)
    #[arg(long, env = "JAV_FAILED", value_hint = clap::ValueHint::DirPath)]
    jav_failed: Option<PathBuf>,

    /// Disable the full-screen TUI even if terminal is interactive
    #[arg(long)]
    no_tui: bool,

    /// Disable the full-screen TUI (legacy alias of --no-tui)
    #[arg(long)]
    no_progress: bool,

    /// Disable actor name normalization via the SDK (enabled by default)
    #[arg(long)]
    no_actor_name_normalization: bool,
}

/// config.toml structure — all fields optional.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    mode: Option<String>,
    jav_download: Option<String>,
    jav_output: Option<String>,
    jav_failed: Option<String>,
    server_url: Option<String>,
    token: Option<String>,
    proxy: Option<String>,
    min_size_mb: Option<u64>,
    cron: Option<String>,
    concurrency: Option<usize>,
    dry_run: Option<bool>,
    no_tui: Option<bool>,
    no_progress: Option<bool>,
    actor_name_normalization: Option<bool>,
}

/// Final merged config — all required fields resolved.
#[derive(Debug, Clone)]
pub struct Config {
    pub mode: RunMode,
    pub jav_download: PathBuf,
    pub jav_output: PathBuf,
    pub jav_failed: PathBuf,
    pub server_url: String,
    pub token: Option<String>,
    pub proxy: Option<String>,
    pub min_size_mb: u64,
    pub cron_expr: Option<String>,
    pub concurrency: usize,
    pub dry_run: bool,
    pub no_tui: bool,
    pub actor_name_normalization: bool,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let raw = RawConfig::parse();

        // Load config file (custom path or auto-discover)
        let file = load_config_file(raw.config_path.as_deref());

        // Merge mode: CLI (--mode) > env (MODE) > config.toml (mode) > default (Once)
        let mode = raw
            .mode
            .or_else(|| {
                file.as_ref()
                    .and_then(|f| f.mode.as_deref())
                    .and_then(parse_mode)
            })
            .unwrap_or(RunMode::Once);

        // Merge cron_expr: CLI (--cron) > env (CRON) > config.toml (cron)
        let cron_expr = raw
            .cron
            .or_else(|| file.as_ref().and_then(|f| f.cron.clone()));

        // Validation
        if mode == RunMode::Cron && cron_expr.is_none() {
            bail!("--cron expression is required when mode is 'cron'. Set via CLI --cron, env CRON, or config.toml [cron].");
        }
        if mode == RunMode::Once && cron_expr.is_some() {
            warn!("⚠ 已设置定时计划，但运行模式为单次。使用 --mode cron 启用定时");
        }

        let jav_download = raw
            .jav_download
            .or_else(|| {
                file.as_ref()
                    .and_then(|f| f.jav_download.as_ref())
                    .map(PathBuf::from)
            })
            .with_context(|| {
                "--jav-download is required (set via CLI, env JAV_DOWNLOAD, or config.toml)"
            })?;

        let jav_output = raw
            .jav_output
            .or_else(|| {
                file.as_ref()
                    .and_then(|f| f.jav_output.as_ref())
                    .map(PathBuf::from)
            })
            .with_context(|| {
                "--jav-output is required (set via CLI, env JAV_OUTPUT, or config.toml)"
            })?;

        let jav_failed = raw
            .jav_failed
            .or_else(|| {
                file.as_ref()
                    .and_then(|f| f.jav_failed.as_ref())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| {
                jav_output
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("JAV_failed")
            });

        Ok(Self {
            mode,
            jav_download,
            jav_output,
            jav_failed,
            server_url: raw
                .server_url
                .or_else(|| file.as_ref().and_then(|f| f.server_url.clone()))
                .unwrap_or_else(|| "http://localhost:8080".into()),
            token: raw
                .token
                .or_else(|| file.as_ref().and_then(|f| f.token.clone())),
            proxy: raw
                .proxy
                .or_else(|| file.as_ref().and_then(|f| f.proxy.clone())),
            min_size_mb: raw
                .min_size_mb
                .or_else(|| file.as_ref().and_then(|f| f.min_size_mb))
                .unwrap_or(300),
            cron_expr,
            concurrency: raw
                .concurrency
                .or_else(|| file.as_ref().and_then(|f| f.concurrency))
                .unwrap_or(4),
            dry_run: raw.dry_run || file.as_ref().and_then(|f| f.dry_run).unwrap_or(false),
            no_tui: raw.no_tui
                || raw.no_progress
                || file
                    .as_ref()
                    .and_then(|f| f.no_tui.or(f.no_progress))
                    .unwrap_or(false),
            actor_name_normalization: !raw.no_actor_name_normalization
                && file
                    .as_ref()
                    .and_then(|f| f.actor_name_normalization)
                    .unwrap_or(true),
        })
    }

    pub fn min_size_bytes(&self) -> u64 {
        self.min_size_mb * 1024 * 1024
    }
}

/// Parse a mode string from config.toml.
fn parse_mode(s: &str) -> Option<RunMode> {
    match s.to_lowercase().as_str() {
        "once" => Some(RunMode::Once),
        "cron" => Some(RunMode::Cron),
        "watch" => Some(RunMode::Watch),
        other => {
            warn!("⚠ 未知运行模式: {}，应为 once/cron/watch", other);
            None
        }
    }
}

/// Discover and load config.toml. Returns None if no file found or parse fails.
fn load_config_file(custom_path: Option<&Path>) -> Option<ConfigFile> {
    let path = if let Some(p) = custom_path {
        if p.exists() {
            Some(p.to_path_buf())
        } else {
            warn!("⚠ 配置文件未找到: {}", p.display());
            return None;
        }
    } else {
        discover_config_path()
    };

    if let Some(ref p) = path {
        match std::fs::read_to_string(p) {
            Ok(content) => match toml::from_str::<ConfigFile>(&content) {
                Ok(cf) => {
                    debug!("✓ 已加载配置文件: {}", p.display());
                    return Some(cf);
                }
                Err(e) => warn!("⚠ 配置文件格式错误: {} — {}", p.display(), e),
            },
            Err(e) => warn!("⚠ 无法读取配置文件: {} — {}", p.display(), e),
        }
    }
    None
}

/// Auto-discover config.toml in standard locations.
fn discover_config_path() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = [
        Some(PathBuf::from("config.toml")),
        dirs::config_dir().map(|d| d.join("metatube-cli-rs").join("config.toml")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for p in &candidates {
        if p.exists() {
            debug!("→ 发现配置文件: {}", p.display());
            return Some(p.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mode_valid() {
        assert_eq!(parse_mode("once"), Some(RunMode::Once));
        assert_eq!(parse_mode("cron"), Some(RunMode::Cron));
        assert_eq!(parse_mode("watch"), Some(RunMode::Watch));
    }

    #[test]
    fn test_parse_mode_case_insensitive() {
        assert_eq!(parse_mode("ONCE"), Some(RunMode::Once));
        assert_eq!(parse_mode("Once"), Some(RunMode::Once));
        assert_eq!(parse_mode("CRON"), Some(RunMode::Cron));
        assert_eq!(parse_mode("Cron"), Some(RunMode::Cron));
        assert_eq!(parse_mode("WATCH"), Some(RunMode::Watch));
        assert_eq!(parse_mode("Watch"), Some(RunMode::Watch));
    }

    #[test]
    fn test_parse_mode_invalid() {
        assert_eq!(parse_mode(""), None);
        assert_eq!(parse_mode("invalid"), None);
    }

    #[test]
    fn test_min_size_bytes() {
        let config = Config {
            mode: RunMode::Once,
            jav_download: PathBuf::from("/tmp/dl"),
            jav_output: PathBuf::from("/tmp/out"),
            jav_failed: PathBuf::from("/tmp/JAV_failed"),
            server_url: "http://localhost".into(),
            token: None,
            proxy: None,
            min_size_mb: 300,
            cron_expr: None,
            concurrency: 4,
            dry_run: false,
            no_tui: false,
            actor_name_normalization: true,
        };
        assert_eq!(config.min_size_bytes(), 300 * 1024 * 1024);

        let config2 = Config {
            min_size_mb: 1,
            ..config
        };
        assert_eq!(config2.min_size_bytes(), 1024 * 1024);
    }

    #[test]
    fn test_config_file_deserialize_basic() {
        let toml_str = r#"
jav_download = "/tmp/dl"
jav_output = "/tmp/out"
server_url = "http://localhost:8080"
"#;
        let cf: ConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(cf.jav_download.unwrap(), "/tmp/dl");
        assert_eq!(cf.jav_output.unwrap(), "/tmp/out");
        assert_eq!(cf.server_url.unwrap(), "http://localhost:8080");
        assert!(cf.mode.is_none());
        assert!(cf.cron.is_none());
        assert!(cf.token.is_none());
    }

    #[test]
    fn test_config_file_deserialize_with_mode_and_cron() {
        let toml_str = r#"
mode = "cron"
cron = "0 */6 * * *"
jav_download = "/tmp/dl"
jav_output = "/tmp/out"
concurrency = 8
dry_run = true
min_size_mb = 100
no_tui = true
"#;
        let cf: ConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(cf.mode.unwrap(), "cron");
        assert_eq!(cf.cron.unwrap(), "0 */6 * * *");
        assert_eq!(cf.concurrency.unwrap(), 8);
        assert!(cf.dry_run.unwrap());
        assert_eq!(cf.min_size_mb.unwrap(), 100);
        assert!(cf.no_tui.unwrap());
    }

    #[test]
    fn test_config_file_deserialize_legacy_no_progress() {
        let toml_str = r#"
jav_download = "/tmp/dl"
jav_output = "/tmp/out"
no_progress = true
"#;
        let cf: ConfigFile = toml::from_str(toml_str).unwrap();
        assert!(cf.no_progress.unwrap());
        assert!(cf.no_tui.is_none());
    }

    #[test]
    fn test_run_mode_labels() {
        assert_eq!(RunMode::Once.label(), "单次扫描");
        assert_eq!(RunMode::Cron.label(), "定时执行");
        assert_eq!(RunMode::Watch.label(), "文件监视");
    }

    #[test]
    fn test_config_file_deserialize_empty_toml() {
        let cf: ConfigFile = toml::from_str("").unwrap();
        assert!(cf.mode.is_none());
        assert!(cf.jav_download.is_none());
        assert!(cf.cron.is_none());
    }

    #[test]
    fn test_config_file_deserialize_actor_name_normalization() {
        let toml_str = r#"
jav_download = "/tmp/dl"
jav_output = "/tmp/out"
actor_name_normalization = false
"#;
        let cf: ConfigFile = toml::from_str(toml_str).unwrap();
        assert_eq!(cf.actor_name_normalization, Some(false));

        let cf: ConfigFile = toml::from_str("jav_download = \"/tmp/dl\"\n").unwrap();
        assert_eq!(cf.actor_name_normalization, None);
    }

    #[test]
    fn test_load_config_file_nonexistent() {
        let result = load_config_file(Some(Path::new("/nonexistent/path/config.toml")));
        assert!(result.is_none());
    }
}
