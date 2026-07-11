use crate::api::Client;
use crate::config::Config;
use crate::error::Error;
use crate::number;
use crate::scanner::{scan, VideoFile};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

pub async fn run(config: &Config) -> anyhow::Result<()> {
    let client = Arc::new(Client::new(
        config.server_url.clone(),
        config.token.clone(),
        config.proxy.as_deref(),
    ));
    let semaphore = Arc::new(Semaphore::new(config.concurrency));

    let videos = scan(
        config.jav_download.to_str().unwrap_or("."),
        config.min_size_bytes(),
    );

    if videos.is_empty() {
        info!("✓ 在 {} 中没有找到视频文件", config.jav_download.display());
        return Ok(());
    }

    let total = videos.len();
    info!("→ 找到 {} 个视频文件，开始处理...", total);

    let dry_run = config.dry_run;
    let mut handles = Vec::new();
    for (i, video) in videos.into_iter().enumerate() {
        let client = client.clone();
        let sem = semaphore.clone();
        let jav_output = config.jav_output.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let result = process_one(&client, &jav_output, dry_run, &video).await;
            (i + 1, video.filename, result)
        }));
    }

    let mut success = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for handle in handles {
        let (idx, filename, result) = handle.await?;
        match result {
            Ok(Some(dest)) => {
                if dry_run {
                    info!(
                        "[{}/{}] [预览] {} → {}",
                        idx, total, filename, dest.display()
                    );
                } else {
                    info!(
                        "[{}/{}] ✓ {} → {}",
                        idx, total, filename, dest.display()
                    );
                }
                success += 1;
            }
            Ok(None) => {
                skipped += 1;
            }
            Err(_e) => {
                failed += 1;
                error!("[{}/{}] ✗ {} 处理失败", idx, total, filename);
            }
        }
    }

    info!("══════════════════════════════════════");
    info!(
        "  处理完成: {} 成功, {} 跳过, {} 失败",
        success, skipped, failed
    );
    info!("══════════════════════════════════════");

    Ok(())
}

/// Returns Ok(None) when the file was skipped (no actresses found).
/// Returns Ok(Some(dest_path)) on success.
/// Returns Err on failure.
pub async fn process_one(
    client: &Client,
    jav_output: &PathBuf,
    dry_run: bool,
    video: &VideoFile,
) -> anyhow::Result<Option<PathBuf>> {
    let id = number::trim(&video.filename);
    if id.is_empty() {
        warn!("⚠ 无法识别番号，已跳过: {}", video.filename);
        return Err(Error::IdExtraction(video.filename.clone()).into());
    }
    debug!("→ 识别番号 \"{}\" ← \"{}\"", id, video.filename);

    let search_result = client.search_movie(&id).await?;
    debug!(
        "→ 找到: {} ({}) 来源: {}",
        search_result.number, search_result.title, search_result.provider
    );

    let movie_info = client
        .get_movie_info(&search_result.provider, &search_result.id)
        .await?;

    let movie_number = movie_info.number.clone();

    let mut actresses: Vec<String> = Vec::new();
    for actress in &movie_info.actors {
        match client.get_gfriends_actor(actress).await {
            Ok(info) => {
                debug!("→ 演员标准化: {} → {}", actress, info.name);
                actresses.push(info.name);
            }
            Err(_) => {
                debug!("→ 演员未标准化: {}, 使用原始名称", actress);
                actresses.push(actress.clone());
            }
        }
    }

    if actresses.is_empty() {
        warn!("⚠ {} — 未找到演员", movie_number);
        return Ok(None);
    }

    let suffix = if number::is_uncensored(&movie_number) {
        "UC"
    } else {
        "C"
    };

    let actress_dir = actresses.join(",");
    let new_filename = format!("{}-{}.mp4", movie_number, suffix);
    let mut dest = PathBuf::from(jav_output);
    dest.push(&actress_dir);
    dest.push(&movie_number);
    dest.push(&new_filename);

    if dry_run {
        return Ok(Some(dest));
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&video.path, &dest).map_err(|e| Error::MoveFailed {
        src: video.path.clone(),
        dst: dest.clone(),
        reason: e.to_string(),
    })?;

    Ok(Some(dest))
}
