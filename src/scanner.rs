use std::path::PathBuf;
use tracing::{debug, warn};

pub(crate) const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "wmv", "flv", "ts", "mov", "webm"];

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub path: PathBuf,
    pub size: u64,
    pub filename: String,
}

pub fn scan(dir: &str, min_size: u64) -> Vec<VideoFile> {
    let mut files = Vec::new();
    scan_recursive(std::path::Path::new(dir), min_size, &mut files);
    debug!("scanned {} video files", files.len());
    files
}

fn scan_recursive(dir: &std::path::Path, min_size: u64, files: &mut Vec<VideoFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("cannot read directory {:?}: {}", dir, e);
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_recursive(&path, min_size, files);
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                    if let Ok(meta) = path.metadata() {
                        let size = meta.len();
                        if size >= min_size {
                            files.push(VideoFile {
                                filename: path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string(),
                                path,
                                size,
                            });
                        }
                    }
                }
            }
        }
    }
}
