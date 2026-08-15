use crate::api::Client;
use crate::config::Config;
use crate::error::Error;
use crate::number;
use crate::scanner::{scan, VideoFile};
use crate::tui::event::{AppEvent, FileStatus, Reporter, Stage};
use chrono::Utc;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
struct FailedRecord {
    error_type: String,
    reason: String,
    number: Option<String>,
    file_path: String,
    timestamp: String,
}

fn move_file_fallback(src: &Path, dst: &Path) -> Result<(), Error> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(Error::Io)?;
    }
    if let Err(e) = std::fs::rename(src, dst) {
        let msg = e.to_string().to_lowercase();
        if msg.contains("cross-device") || msg.contains("not same device") {
            debug!("跨文件系统移动，使用 copy+delete 备用方案");
            std::fs::copy(src, dst).map_err(|e| Error::MoveFailed {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
                reason: format!("copy failed: {}", e),
            })?;
            std::fs::remove_file(src).map_err(|e| Error::MoveFailed {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
                reason: format!("delete source after copy failed: {}", e),
            })?;
            return Ok(());
        }
        return Err(Error::MoveFailed {
            src: src.to_path_buf(),
            dst: dst.to_path_buf(),
            reason: e.to_string(),
        });
    }
    Ok(())
}

fn archive_failed(failed_dir: &Path, subdir_name: &str, file_path: &Path, record: &FailedRecord) {
    let dir = failed_dir.join(subdir_name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("无法创建失败归档目录 {}: {}", dir.display(), e);
        return;
    }

    let dest = dir.join(
        file_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("unknown")),
    );

    if let Err(e) = move_file_fallback(file_path, &dest) {
        error!("归档失败文件失败: {}", e);
        return;
    }

    let reason_path = dir.join("failed_reason.txt");
    match toml::to_string_pretty(record) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&reason_path, &content) {
                error!("写入 failed_reason.txt 失败: {}", e);
            } else {
                info!(
                    "已归档失败文件: {} -> {}",
                    file_path.display(),
                    dest.display()
                );
            }
        }
        Err(e) => {
            error!("序列化失败记录失败: {}", e);
        }
    }
}

fn scan_failed_for_retry(failed_dir: &Path, download_dir: &Path) -> Vec<VideoFile> {
    let mut retry_files = Vec::new();

    let entries = match std::fs::read_dir(failed_dir) {
        Ok(entries) => entries,
        Err(_) => return retry_files,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let reason_file = path.join("failed_reason.txt");
        if !reason_file.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&reason_file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let record: FailedRecord = match toml::from_str(&content) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "解析 failed_reason.txt 失败 {}: {}",
                    reason_file.display(),
                    e
                );
                continue;
            }
        };

        if !should_retry_record(&record) {
            continue;
        }

        for child in std::fs::read_dir(&path).into_iter().flatten().flatten() {
            let child_path = child.path();
            if child_path.is_file()
                && child_path
                    .file_name()
                    .is_some_and(|n| n != "failed_reason.txt")
            {
                let filename = child_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let dest = download_dir.join(&filename);

                if let Err(e) = move_file_fallback(&child_path, &dest) {
                    warn!("回迁重试文件失败: {}", e);
                    continue;
                }

                info!(
                    "回迁重试: {} -> {} (原因: {})",
                    filename,
                    dest.display(),
                    record.reason
                );

                retry_files.push(VideoFile {
                    path: dest,
                    filename,
                    size: 0,
                });
            }
        }

        let _ = std::fs::remove_dir_all(&path);
    }

    retry_files
}

fn should_retry_record(record: &FailedRecord) -> bool {
    match record.error_type.as_str() {
        "Http" => true,
        "ClientInit" => true,
        "FileNotFound" => true,
        "Api" => {
            if let Ok(code) = record.reason.parse::<u16>() {
                code >= 500
            } else {
                record.reason.contains("500")
                    || record.reason.contains("502")
                    || record.reason.contains("503")
            }
        }
        "Io" => {
            let r = record.reason.to_lowercase();
            r.contains("permission denied")
                || r.contains("access is denied")
                || r.contains("no space")
        }
        "MoveFailed" => {
            let r = record.reason.to_lowercase();
            r.contains("permission denied")
                || r.contains("access is denied")
                || r.contains("cross-device")
                || r.contains("not same device")
        }
        _ => false,
    }
}

fn make_subdir_name(filename: &str, number: Option<&str>) -> String {
    if let Some(n) = number {
        return n.to_string();
    }
    let mut hasher = DefaultHasher::new();
    filename.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Group directory for a video: normalized actress names joined by ",",
/// or the configured unknown-actress directory when none could be scraped.
fn actress_group_dir(actresses: &[String], unknown_dir: &str) -> String {
    if actresses.is_empty() {
        unknown_dir.to_string()
    } else {
        actresses.join(",")
    }
}

/// Standardized filename: `{number}-{UC|C}.{ext}`, extension falls back to mp4.
fn standard_filename(movie_number: &str, original_filename: &str) -> String {
    let suffix = if number::is_uncensored(movie_number) {
        "UC"
    } else {
        "C"
    };
    let ext = Path::new(original_filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4");
    format!("{}-{}.{}", movie_number, suffix, ext)
}

pub async fn run(config: &Config, reporter: Arc<dyn Reporter>) -> anyhow::Result<()> {
    let client = Arc::new(Client::new(
        config.server_url.clone(),
        config.token.clone(),
        config.proxy.as_deref(),
    )?);
    let semaphore = Arc::new(Semaphore::new(config.concurrency));

    let retry_videos = scan_failed_for_retry(&config.jav_failed, &config.jav_download);
    if !retry_videos.is_empty() {
        info!("→ 从失败目录回迁 {} 个可重试文件", retry_videos.len());
    }

    reporter.emit(AppEvent::ScanStart);
    let mut videos = scan(
        &config.jav_download.to_string_lossy(),
        config.min_size_bytes(),
        reporter.clone(),
    )
    .await;
    videos.extend(retry_videos);
    reporter.emit(AppEvent::ScanDone);

    if videos.is_empty() {
        info!("✓ 在 {} 中没有找到视频文件", config.jav_download.display());
        return Ok(());
    }

    let total = videos.len();
    info!("→ 找到 {} 个视频文件，开始处理...", total);
    reporter.emit(AppEvent::RoundStart { total });

    let dry_run = config.dry_run;
    let normalize_actors = config.actor_name_normalization;
    let unknown_actress_dir = config.unknown_actress_dir.clone();
    let mut handles = Vec::new();
    for video in videos {
        let client = client.clone();
        let sem = semaphore.clone();
        let jav_output = config.jav_output.clone();
        let reporter = reporter.clone();
        let filename = video.filename.clone();
        let unknown_actress_dir = unknown_actress_dir.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore shouldn't close");
            reporter.emit(AppEvent::FileStart {
                filename: filename.clone(),
            });
            let result = process_one(
                client.clone(),
                &jav_output,
                dry_run,
                normalize_actors,
                &unknown_actress_dir,
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
            (filename, result)
        }));
    }

    let mut success = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    let jav_download = &config.jav_download;
    let jav_failed = &config.jav_failed;

    for (i, handle) in handles.into_iter().enumerate() {
        let idx = i + 1;
        let (filename, result) = handle.await?;
        match &result {
            Ok(Some(dest)) => {
                if dry_run {
                    info!(
                        "[{}/{}] [预览] {} → {}",
                        idx,
                        total,
                        filename,
                        dest.display()
                    );
                } else {
                    info!("[{}/{}] ✓ {} → {}", idx, total, filename, dest.display());
                }
                success += 1;
            }
            Ok(None) => {
                skipped += 1;
            }
            Err(e) => {
                failed += 1;
                error!("[{}/{}] ✗ {} 处理失败: {:#}", idx, total, filename, e);

                if !dry_run {
                    let file_path = jav_download.join(&filename);
                    if file_path.exists() {
                        let number = number::trim(&filename);
                        let number_opt = if number.is_empty() {
                            None
                        } else {
                            Some(&*number)
                        };
                        let subdir = make_subdir_name(&filename, number_opt);

                        let error_type = e
                            .downcast_ref::<Error>()
                            .map(|err| err.error_type().to_string())
                            .unwrap_or_else(|| "Unknown".to_string());
                        let reason = e
                            .downcast_ref::<Error>()
                            .map(|err| err.to_string())
                            .unwrap_or_else(|| format!("{:#}", e));

                        let record = FailedRecord {
                            error_type,
                            reason,
                            number: number_opt.map(|s| s.to_string()),
                            file_path: file_path.to_string_lossy().to_string(),
                            timestamp: Utc::now().to_rfc3339(),
                        };

                        archive_failed(jav_failed, &subdir, &file_path, &record);
                    }
                }
            }
        }
    }

    reporter.emit(AppEvent::RoundDone {
        success,
        skipped,
        failed,
    });

    info!("══════════════════════════════════════");
    info!(
        "  处理完成: {} 成功, {} 跳过, {} 失败",
        success, skipped, failed
    );
    info!("══════════════════════════════════════");

    Ok(())
}

/// Returns Ok(Some(dest_path)) on success — videos with no scraped actress
/// are renamed to the standard format and organized into the configured
/// unknown-actress directory. Returns Err on failure.
pub async fn process_one(
    client: Arc<Client>,
    jav_output: &Path,
    dry_run: bool,
    normalize_actors: bool,
    unknown_actress_dir: &str,
    video: &VideoFile,
    reporter: &dyn Reporter,
) -> anyhow::Result<Option<PathBuf>> {
    let id = number::trim(&video.filename);
    if id.is_empty() {
        warn!("⚠ 无法识别番号，已跳过: {}", video.filename);
        return Err(Error::IdExtraction(video.filename.clone()).into());
    }
    debug!("→ 识别番号 \"{}\" ← \"{}\"", id, video.filename);

    reporter.emit(AppEvent::FileStage {
        filename: video.filename.clone(),
        stage: Stage::Search,
    });
    let search_result = client.search_movie(&id).await?;
    debug!(
        "→ 找到: {} ({}) 来源: {}",
        search_result.number, search_result.title, search_result.provider
    );

    reporter.emit(AppEvent::FileStage {
        filename: video.filename.clone(),
        stage: Stage::Detail,
    });
    let movie_info = client
        .get_movie_info(&search_result.provider, &search_result.id)
        .await?;

    let movie_number = movie_info.number.clone();

    reporter.emit(AppEvent::FileStage {
        filename: video.filename.clone(),
        stage: Stage::Normalize,
    });
    let actresses = normalize_actresses(&client, &movie_info.actors, normalize_actors).await;

    if actresses.is_empty() {
        warn!("⚠ {} — 未找到演员，归入未知演员目录", movie_number);
    }

    let new_filename = standard_filename(&movie_number, &video.filename);
    let actress_dir = actress_group_dir(&actresses, unknown_actress_dir);
    let mut dest = PathBuf::from(jav_output);
    dest.push(&actress_dir);
    dest.push(&movie_number);
    dest.push(&new_filename);

    if dry_run {
        return Ok(Some(dest));
    }

    reporter.emit(AppEvent::FileStage {
        filename: video.filename.clone(),
        stage: Stage::Move,
    });
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(Error::Io)?;
    }
    std::fs::rename(&video.path, &dest).map_err(|e| Error::MoveFailed {
        src: video.path.clone(),
        dst: dest.clone(),
        reason: e.to_string(),
    })?;

    Ok(Some(dest))
}

async fn normalize_actresses(client: &Client, actors: &[String], enabled: bool) -> Vec<String> {
    if !enabled {
        debug!("演员标准化已关闭，使用原始名称");
        return actors.to_vec();
    }

    let handles: Vec<_> = actors
        .iter()
        .map(|actress| {
            let actress = actress.clone();
            async move { client.normalize_actor_name(&actress).await }
        })
        .collect();

    join_all(handles).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_move_file_fallback_same_device() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("test.mp4");
        let dst = dir.path().join("subdir").join("test.mp4");
        std::fs::write(&src, b"hello").unwrap();
        move_file_fallback(&src, &dst).unwrap();
        assert!(!src.exists());
        assert!(dst.exists());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn test_move_file_fallback_dst_exists() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.mp4");
        let dst = dir.path().join("dst.mp4");
        std::fs::write(&src, b"new content").unwrap();
        std::fs::write(&dst, b"old content").unwrap();
        move_file_fallback(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "new content");
    }

    #[test]
    fn test_move_file_fallback_src_not_found() {
        let dir = tempdir().unwrap();
        let result = move_file_fallback(
            &dir.path().join("nonexistent.mp4"),
            &dir.path().join("dst.mp4"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_should_retry_record_http() {
        let record = FailedRecord {
            error_type: "Http".into(),
            reason: "timeout".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_client_init() {
        let record = FailedRecord {
            error_type: "ClientInit".into(),
            reason: "bad proxy".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_file_not_found() {
        let record = FailedRecord {
            error_type: "FileNotFound".into(),
            reason: "missing file".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_api_5xx() {
        let record = FailedRecord {
            error_type: "Api".into(),
            reason: "500".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_api_4xx_not_retryable() {
        let record = FailedRecord {
            error_type: "Api".into(),
            reason: "404".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(!should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_io_permission() {
        let record = FailedRecord {
            error_type: "Io".into(),
            reason: "permission denied".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_io_no_space() {
        let record = FailedRecord {
            error_type: "Io".into(),
            reason: "No space left on device".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_move_failed_permission() {
        let record = FailedRecord {
            error_type: "MoveFailed".into(),
            reason: "Access is denied".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_move_failed_cross_device() {
        let record = FailedRecord {
            error_type: "MoveFailed".into(),
            reason: "Invalid cross-device link".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(should_retry_record(&record));
    }

    #[test]
    fn test_should_retry_record_non_retryable() {
        for (error_type, reason) in [
            ("IdExtraction", "bad filename"),
            ("NoResults", "ABC-123"),
            ("NoMovieInfo", "fanza/ssis00123"),
        ] {
            let record = FailedRecord {
                error_type: error_type.into(),
                reason: reason.into(),
                number: None,
                file_path: "/tmp/test.mp4".into(),
                timestamp: "2024-01-01T00:00:00Z".into(),
            };
            assert!(!should_retry_record(&record));
        }
    }

    #[test]
    fn test_should_retry_record_unknown_not_retryable() {
        let record = FailedRecord {
            error_type: "UnknownError".into(),
            reason: "something went wrong".into(),
            number: None,
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        assert!(!should_retry_record(&record));
    }

    #[test]
    fn test_failed_record_serialization_roundtrip() {
        let record = FailedRecord {
            error_type: "Api".into(),
            reason: "500 Internal Server Error".into(),
            number: Some("SSIS-123".into()),
            file_path: "/tmp/test.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        let toml_str = toml::to_string_pretty(&record).unwrap();
        let parsed: FailedRecord = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.error_type, "Api");
        assert_eq!(parsed.reason, "500 Internal Server Error");
        assert_eq!(parsed.number, Some("SSIS-123".into()));
        assert_eq!(parsed.file_path, "/tmp/test.mp4");
        assert_eq!(parsed.timestamp, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_make_subdir_name_with_number() {
        assert_eq!(
            make_subdir_name("SSIS-123.mp4", Some("SSIS-123")),
            "SSIS-123"
        );
    }

    #[test]
    fn test_make_subdir_name_without_number() {
        let result = make_subdir_name("bad_file.mp4", None);
        assert!(!result.is_empty());
        assert!(result.len() == 16);
    }

    #[test]
    fn test_make_subdir_name_deterministic() {
        let a = make_subdir_name("same_file.mp4", None);
        let b = make_subdir_name("same_file.mp4", None);
        assert_eq!(a, b);
        let c = make_subdir_name("different.mp4", None);
        assert_ne!(a, c);
    }

    #[test]
    fn test_actress_group_dir_single() {
        let actresses = ["深田えいみ".to_string()];
        assert_eq!(actress_group_dir(&actresses, "1-未知演员"), "深田えいみ");
    }

    #[test]
    fn test_actress_group_dir_multiple() {
        let actresses = ["actress_a".to_string(), "actress_b".to_string()];
        assert_eq!(
            actress_group_dir(&actresses, "1-未知演员"),
            "actress_a,actress_b"
        );
    }

    #[test]
    fn test_actress_group_dir_empty_falls_back_to_unknown() {
        assert_eq!(actress_group_dir(&[], "1-未知演员"), "1-未知演员");
    }

    #[test]
    fn test_standard_filename_censored() {
        assert_eq!(
            standard_filename("SSIS-123", "ssis00123.mp4"),
            "SSIS-123-C.mp4"
        );
    }

    #[test]
    fn test_standard_filename_uncensored() {
        assert_eq!(
            standard_filename("HEYZO-1789", "heyzo-1789.wmv"),
            "HEYZO-1789-UC.wmv"
        );
    }

    #[test]
    fn test_standard_filename_no_extension() {
        assert_eq!(standard_filename("ABP-030", "ABP-030"), "ABP-030-C.mp4");
    }

    #[test]
    fn test_archive_failed_creates_directory_and_files() {
        let dir = tempdir().unwrap();
        let failed_dir = dir.path().join("JAV_failed");
        let src_file = dir.path().join("test_video.mp4");
        std::fs::write(&src_file, b"video content").unwrap();

        let record = FailedRecord {
            error_type: "Api".into(),
            reason: "500 Server Error".into(),
            number: Some("TEST-001".into()),
            file_path: src_file.to_string_lossy().to_string(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };

        archive_failed(&failed_dir, "TEST-001", &src_file, &record);

        let expected_dir = failed_dir.join("TEST-001");
        assert!(expected_dir.exists());
        let expected_file = expected_dir.join("test_video.mp4");
        assert!(expected_file.exists());
        let reason_file = expected_dir.join("failed_reason.txt");
        assert!(reason_file.exists());

        let parsed: FailedRecord =
            toml::from_str(&std::fs::read_to_string(&reason_file).unwrap()).unwrap();
        assert_eq!(parsed.error_type, "Api");
        assert_eq!(parsed.reason, "500 Server Error");
    }

    #[test]
    fn test_scan_failed_for_retry_retryable() {
        let dir = tempdir().unwrap();
        let failed_dir = dir.path().join("JAV_failed");
        let download_dir = dir.path().join("download");
        std::fs::create_dir_all(&download_dir).unwrap();

        let subdir = failed_dir.join("TEST-001");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("test_video.mp4"), b"video").unwrap();

        let record = FailedRecord {
            error_type: "Http".into(),
            reason: "timeout".into(),
            number: Some("TEST-001".into()),
            file_path: "/original/test_video.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        std::fs::write(
            subdir.join("failed_reason.txt"),
            toml::to_string_pretty(&record).unwrap(),
        )
        .unwrap();

        let result = scan_failed_for_retry(&failed_dir, &download_dir);
        assert_eq!(result.len(), 1);
        assert!(download_dir.join("test_video.mp4").exists());
        assert!(!subdir.exists());
    }

    #[test]
    fn test_scan_failed_for_retry_non_retryable_skipped() {
        let dir = tempdir().unwrap();
        let failed_dir = dir.path().join("JAV_failed");
        let download_dir = dir.path().join("download");
        std::fs::create_dir_all(&download_dir).unwrap();

        let subdir = failed_dir.join("BADNAME");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("bad.mp4"), b"video").unwrap();

        let record = FailedRecord {
            error_type: "IdExtraction".into(),
            reason: "bad file name".into(),
            number: None,
            file_path: "/original/bad.mp4".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
        };
        std::fs::write(
            subdir.join("failed_reason.txt"),
            toml::to_string_pretty(&record).unwrap(),
        )
        .unwrap();

        let result = scan_failed_for_retry(&failed_dir, &download_dir);
        assert!(result.is_empty());
        assert!(subdir.exists());
    }

    #[test]
    fn test_scan_failed_for_retry_empty_dir() {
        let dir = tempdir().unwrap();
        let failed_dir = dir.path().join("nonexistent");
        let download_dir = dir.path().join("download");
        let result = scan_failed_for_retry(&failed_dir, &download_dir);
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_failed_for_retry_ignores_extra_files() {
        let dir = tempdir().unwrap();
        let failed_dir = dir.path().join("JAV_failed");
        let download_dir = dir.path().join("download");
        std::fs::create_dir_all(&download_dir).unwrap();

        let subdir1 = failed_dir.join("NO_REASON");
        std::fs::create_dir_all(&subdir1).unwrap();
        std::fs::write(subdir1.join("orphan.mp4"), b"video").unwrap();

        let result = scan_failed_for_retry(&failed_dir, &download_dir);
        assert!(result.is_empty());
    }
}
