use crate::api::Client;
use crate::config::Config;
use crate::error::Error;
use crate::number;
use crate::scanner::{scan, VideoFile};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

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
        info!("no video files found in {:?}", config.jav_download);
        return Ok(());
    }

    info!("found {} video files to process", videos.len());

    let mut handles = Vec::new();
    for video in videos {
        let client = client.clone();
        let sem = semaphore.clone();
        let jav_output = config.jav_output.clone();
        let dry_run = config.dry_run;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if let Err(e) = process_one(&client, &jav_output, dry_run, &video).await {
                error!("{}: {}", video.filename, e);
            }
        }));
    }

    for handle in handles {
        handle.await?;
    }

    Ok(())
}

pub async fn process_one(
    client: &Client,
    jav_output: &PathBuf,
    dry_run: bool,
    video: &VideoFile,
) -> anyhow::Result<()> {
    let id = number::trim(&video.filename);
    if id.is_empty() {
        warn!("could not extract ID from: {}", video.filename);
        return Err(Error::IdExtraction(video.filename.clone()).into());
    }
    info!("extracted ID \"{}\" from \"{}\"", id, video.filename);

    let search_result = client.search_movie(&id).await?;
    info!(
        "found: {} ({}) via {}",
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
                info!("gfriends: {} -> {}", actress, info.name);
                actresses.push(info.name);
            }
            Err(_) => {
                warn!("gfriends miss for: {}, using raw", actress);
                actresses.push(actress.clone());
            }
        }
    }

    if actresses.is_empty() {
        warn!("no actresses for: {}", movie_number);
        return Ok(());
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
        info!("[DRY-RUN] {:?} -> {:?}", video.path, dest);
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        info!("moving {:?} -> {:?}", video.path, dest);
        std::fs::rename(&video.path, &dest).map_err(|e| Error::MoveFailed {
            src: video.path.clone(),
            dst: dest.clone(),
            reason: e.to_string(),
        })?;
    }

    Ok(())
}
