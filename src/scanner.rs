use std::path::PathBuf;
use tracing::{debug, warn};

pub(crate) const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "wmv", "flv", "ts", "mov", "webm"];

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub path: PathBuf,
    #[allow(dead_code)]
    pub size: u64,
    pub filename: String,
}

pub async fn scan(dir: &str, min_size: u64) -> Vec<VideoFile> {
    let dir = dir.to_string();
    tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        scan_recursive(std::path::Path::new(&dir), min_size, &mut files);
        debug!("→ 扫描完成，共 {} 个视频文件", files.len());
        files
    })
    .await
    .expect("scan spawn_blocking panicked")
}

fn scan_recursive(dir: &std::path::Path, min_size: u64, files: &mut Vec<VideoFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_e) => {
            warn!("⚠ 无法读取目录: {}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            scan_recursive(&path, min_size, files);
        } else if file_type.is_file() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn make_file(dir: &std::path::Path, name: &str, size: usize) {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(&vec![0u8; size]).unwrap();
    }

    #[test]
    fn test_video_extensions_count() {
        assert_eq!(VIDEO_EXTENSIONS.len(), 8);
        assert!(VIDEO_EXTENSIONS.contains(&"mp4"));
        assert!(VIDEO_EXTENSIONS.contains(&"mkv"));
    }

    #[tokio::test]
    async fn test_scan_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let files = scan(dir.path().to_str().unwrap(), 0).await;
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn test_scan_filters_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        make_file(dir.path(), "video.mp4", 1024);
        make_file(dir.path(), "readme.txt", 1024);
        make_file(dir.path(), "movie.mkv", 2048);
        make_file(dir.path(), "image.jpg", 512);

        let files = scan(dir.path().to_str().unwrap(), 0).await;
        assert_eq!(files.len(), 2);
        let names: Vec<&str> = files.iter().map(|f| f.filename.as_str()).collect();
        assert!(names.contains(&"video.mp4"));
        assert!(names.contains(&"movie.mkv"));
    }

    #[tokio::test]
    async fn test_scan_filters_by_min_size() {
        let dir = tempfile::tempdir().unwrap();
        make_file(dir.path(), "small.mp4", 100);
        make_file(dir.path(), "big.mp4", 5000);

        let files = scan(dir.path().to_str().unwrap(), 1024).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "big.mp4");
    }

    #[tokio::test]
    async fn test_scan_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        make_file(dir.path(), "root.mp4", 1024);
        make_file(&sub, "nested.avi", 1024);

        let files = scan(dir.path().to_str().unwrap(), 0).await;
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn test_scan_case_insensitive_extension() {
        let dir = tempfile::tempdir().unwrap();
        make_file(dir.path(), "video.MP4", 1024);
        make_file(dir.path(), "movie.MKV", 1024);
        make_file(dir.path(), "clip.Mp4", 1024);

        let files = scan(dir.path().to_str().unwrap(), 0).await;
        assert_eq!(files.len(), 3);
    }

    #[tokio::test]
    async fn test_video_file_fields() {
        let dir = tempfile::tempdir().unwrap();
        make_file(dir.path(), "test.mp4", 4321);

        let files = scan(dir.path().to_str().unwrap(), 0).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "test.mp4");
        assert_eq!(files[0].size, 4321);
        assert_eq!(files[0].path, dir.path().join("test.mp4"));
    }
}
