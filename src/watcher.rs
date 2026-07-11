/// File-system watcher: monitors the download directory for new video files
/// and processes them one-by-one via `process_one`.
use crate::api::Client;
use crate::config::Config;
use crate::processor::process_one;
use crate::scanner::{VideoFile, VIDEO_EXTENSIONS};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

pub async fn run_watch(config: &Config) -> anyhow::Result<()> {
    let client = Arc::new(Client::new(
        config.server_url.clone(),
        config.token.clone(),
        config.proxy.as_deref(),
    ));
    let jav_output = config.jav_output.clone();
    let dry_run = config.dry_run;
    let min_size = config.min_size_bytes();
    let watch_dir = config.jav_download.clone();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<DebouncedEvent>>();

    // Spawn blocking watcher on a dedicated OS thread.
    let _watcher_handle = tokio::task::spawn_blocking(move || {
        let mut debouncer = match new_debouncer(Duration::from_secs(5), move |events| {
            match events {
                Ok(evts) => {
                    let _ = tx.send(evts);
                }
                Err(e) => error!("watch error: {}", e),
            }
        }) {
            Ok(d) => d,
            Err(e) => {
                error!("failed to create file watcher: {}", e);
                return;
            }
        };

        if let Err(e) = debouncer
            .watcher()
            .watch(&watch_dir, notify::RecursiveMode::Recursive)
        {
            error!("failed to watch {:?}: {}", watch_dir, e);
            return;
        }

        info!("watching {:?} for new video files", watch_dir);

        // Keep the debouncer alive; its internal thread calls our callback.
        loop {
            std::thread::park();
        }
    });

    info!("watch mode started, waiting for file events...");

    while let Some(events) = rx.recv().await {
        for event in events {
            let path = &event.path;

            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            let is_video = ext
                .as_deref()
                .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e));
            if !is_video {
                continue;
            }
            let size = match std::fs::metadata(path) {
                Ok(m) => m.len(),
                Err(e) => {
                    warn!("cannot stat {:?}: {}", path, e);
                    continue;
                }
            };
            if size < min_size {
                continue;
            }

            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let video = VideoFile {
                path: path.clone(),
                size,
                filename,
            };

            let client = client.clone();
            let jav_output = jav_output.clone();
            tokio::spawn(async move {
                if let Err(e) = process_one(&client, &jav_output, dry_run, &video).await {
                    error!("{}: {}", video.filename, e);
                }
            });
        }
    }

    Ok(())
}
