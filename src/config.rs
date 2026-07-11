/// CLI argument parsing with config.toml support.
/// Priority: CLI args > env vars > config.toml > defaults.
use clap::Parser;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// CLI args (all optional during initial parse, validated after merge).
#[derive(Parser, Debug)]
#[command(name = "metatube-cli")]
#[command(about = "Organize JAV videos by actress using MetaTube SDK")]
#[command(version)]
struct RawConfig {
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
}

/// config.toml structure — all fields optional.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    jav_download: Option<String>,
    jav_output: Option<String>,
    server_url: Option<String>,
    token: Option<String>,
    proxy: Option<String>,
    min_size_mb: Option<u64>,
    cron: Option<String>,
    concurrency: Option<usize>,
    dry_run: Option<bool>,
}

/// Final merged config — all required fields resolved.
#[derive(Debug, Clone)]
pub struct Config {
    pub jav_download: PathBuf,
    pub jav_output: PathBuf,
    pub server_url: String,
    pub token: Option<String>,
    pub proxy: Option<String>,
    pub min_size_mb: u64,
    pub cron: Option<String>,
    pub concurrency: usize,
    pub dry_run: bool,
}

impl Config {
    pub fn load() -> Self {
        let raw = RawConfig::parse();

        // Load config file (custom path or auto-discover)
        let file = load_config_file(raw.config_path.as_deref());

        Self {
            jav_download: raw.jav_download
                .or_else(|| file.as_ref().and_then(|f| f.jav_download.as_ref()).map(PathBuf::from))
                .expect("--jav-download is required (set via CLI, env JAV_DOWNLOAD, or config.toml)"),
            jav_output: raw.jav_output
                .or_else(|| file.as_ref().and_then(|f| f.jav_output.as_ref()).map(PathBuf::from))
                .expect("--jav-output is required (set via CLI, env JAV_OUTPUT, or config.toml)"),
            server_url: raw.server_url
                .or_else(|| file.as_ref().and_then(|f| f.server_url.clone()))
                .unwrap_or_else(|| "http://localhost:8080".into()),
            token: raw.token.or_else(|| file.as_ref().and_then(|f| f.token.clone())),
            proxy: raw.proxy.or_else(|| file.as_ref().and_then(|f| f.proxy.clone())),
            min_size_mb: raw.min_size_mb
                .or_else(|| file.as_ref().and_then(|f| f.min_size_mb))
                .unwrap_or(300),
            cron: raw.cron.or_else(|| file.as_ref().and_then(|f| f.cron.clone())),
            concurrency: raw.concurrency
                .or_else(|| file.as_ref().and_then(|f| f.concurrency))
                .unwrap_or(4),
            dry_run: raw.dry_run || file.as_ref().and_then(|f| f.dry_run).unwrap_or(false),
        }
    }

    pub fn min_size_bytes(&self) -> u64 {
        self.min_size_mb * 1024 * 1024
    }
}

/// Discover and load config.toml. Returns None if no file found or parse fails.
fn load_config_file(custom_path: Option<&Path>) -> Option<ConfigFile> {
    let path = if let Some(p) = custom_path {
        if p.exists() {
            Some(p.to_path_buf())
        } else {
            warn!("config file not found: {:?}", p);
            return None;
        }
    } else {
        discover_config_path()
    };

    if let Some(ref p) = path {
        match std::fs::read_to_string(p) {
            Ok(content) => match toml::from_str::<ConfigFile>(&content) {
                Ok(cf) => {
                    debug!("loaded config from {:?}", p);
                    return Some(cf);
                }
                Err(e) => warn!("failed to parse {:?}: {}", p, e),
            },
            Err(e) => warn!("failed to read {:?}: {}", p, e),
        }
    }
    None
}

/// Auto-discover config.toml in standard locations.
fn discover_config_path() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = [
        Some(PathBuf::from("metatube-cli.toml")),
        dirs::config_dir().map(|d| d.join("metatube-cli").join("config.toml")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for p in &candidates {
        if p.exists() {
            debug!("found config at {:?}", p);
            return Some(p.clone());
        }
    }
    None
}
