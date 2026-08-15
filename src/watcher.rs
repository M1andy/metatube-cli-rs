/// File-system watcher: monitors the download directory for new video files
/// and processes them one-by-one via `process_one`.
use crate::api::Client;
use crate::config::Config;
use crate::processor::process_one;
use crate::scanner::{VideoFile, VIDEO_EXTENSIONS};
use crate::tui::event::{AppEvent, FileStatus, Reporter};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{error, info, warn};

pub async fn run_watch(
    config: &Config,
    reporter: Arc<dyn Reporter>,
    quit_flag: &AtomicBool,
) -> anyhow::Result<()> {
    let client = Arc::new(Client::new(
        config.server_url.clone(),
        config.token.clone(),
        config.proxy.as_deref(),
    )?);
    let jav_output = config.jav_output.clone();
    let dry_run = config.dry_run;
    let normalize_actors = config.actor_name_normalization;
    let min_size = config.min_size_bytes();
    let watch_dir = config.jav_download.clone();

    let stop_flag = Arc::new(AtomicBool::new(false));

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Vec<DebouncedEvent>>(128);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

    let stop = stop_flag.clone();
    let watch_dir_display = watch_dir.display().to_string();
    let watcher_handle = tokio::task::spawn_blocking(move || {
        let event_tx = event_tx.clone();
        let mut debouncer =
            match new_debouncer(Duration::from_secs(5), move |events| match events {
                Ok(evts) => {
                    let _ = event_tx.try_send(evts);
                }
                Err(e) => error!("⚠ 文件监视出错: {}", e),
            }) {
                Ok(d) => d,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("无法启动文件监视: {}", e)));
                    return;
                }
            };

        if let Err(e) = debouncer
            .watcher()
            .watch(&watch_dir, notify::RecursiveMode::Recursive)
        {
            let _ = ready_tx.send(Err(format!(
                "无法监视文件夹: {} — {}",
                watch_dir.display(),
                e
            )));
            return;
        }

        // Signal that the watcher has started successfully.
        let _ = ready_tx.send(Ok(()));
        info!("→ 正在监视文件夹: {}", watch_dir_display);

        // Keep the debouncer alive; its internal thread calls our callback.
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if stop.load(Ordering::SeqCst) {
                drop(debouncer);
                break;
            }
        }
    });

    // Wait for the watcher initialization to complete.
    match ready_rx.await {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => return Err(anyhow::anyhow!("{}", msg)),
        Err(_) => {
            // oneshot dropped without sending — watcher thread panicked or crashed
            if let Err(panic_err) = watcher_handle.await {
                if let Ok(panic_payload) = panic_err.try_into_panic() {
                    let msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "（未知原因）".to_string()
                    };
                    return Err(anyhow::anyhow!("文件监视线程异常: {}", msg));
                }
            }
            return Err(anyhow::anyhow!("文件监视初始化异常退出"));
        }
    }

    info!("→ 等待新视频文件...");
    reporter.emit(AppEvent::WatchReady {
        path: config.jav_download.clone(),
    });

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("收到终止信号，关闭文件监视...");
                stop_flag.store(true, Ordering::SeqCst);
                break;
            }
            // TUI 按键退出（raw mode 下 Ctrl+C 不产生信号，经 quit_flag 传递）
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if quit_flag.load(Ordering::SeqCst) {
                    info!("收到退出请求，关闭文件监视...");
                    stop_flag.store(true, Ordering::SeqCst);
                    break;
                }
            }
            events = event_rx.recv() => {
                let events = match events {
                    Some(e) => e,
                    None => {
                        // Sender dropped — check if watcher thread panicked.
                        if let Err(panic_err) = watcher_handle.await {
                            error!("✗ 文件监视线程异常退出: {:?}", panic_err);
                        }
                        break;
                    }
                };

                for event in events {
                    let path = event.path;

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
                    let initial_size = match std::fs::metadata(&path) {
                        Ok(m) => m.len(),
                        Err(e) => {
                            warn!("⚠ 无法读取文件: {} — {}", path.display(), e);
                            continue;
                        }
                    };
                    if initial_size < min_size {
                        continue;
                    }

                    // File size stability check: wait and verify size hasn't changed.
                    // Spawn each file's processing concurrently so slow checks don't
                    // block the event loop from receiving new events.
                    let client = client.clone();
                    let jav_output = jav_output.clone();
                    let reporter = reporter.clone();
                    tokio::spawn(async move {
                        let path_for_check = path.clone();
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        match std::fs::metadata(&path_for_check) {
                            Ok(meta) if meta.len() == initial_size => {}
                            _ => {
                                info!("文件仍在写入中，延迟处理: {}", path_for_check.display());
                                return;
                            }
                        }

                        let filename = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let video = VideoFile {
                            path: path.clone(),
                            size: initial_size,
                            filename: filename.clone(),
                        };

                        reporter.emit(AppEvent::FileStart {
                            filename: filename.clone(),
                        });
                        let result = process_one(
                            client,
                            &jav_output,
                            dry_run,
                            normalize_actors,
                            &video,
                            reporter.as_ref(),
                        )
                        .await;
                        let status = match &result {
                            Ok(Some(_)) => FileStatus::Success,
                            Ok(None) => FileStatus::Skipped,
                            Err(_) => FileStatus::Failed,
                        };
                        reporter.emit(AppEvent::FileDone {
                            filename: filename.clone(),
                            status,
                        });

                        if let Err(e) = result {
                            error!("✗ 文件处理失败: {} — {:#}", video.filename, e);
                        }
                    });
                }
            }
        }
    }

    Ok(())
}
