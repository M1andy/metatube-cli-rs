/// End-to-end tests for the file watcher subsystem.
///
/// These tests verify that `notify` + `notify-debouncer-mini` correctly
/// detect file system changes on the current platform (Windows / Linux / macOS).
///
/// They do NOT require a running MetaTube server — only the file-watching
/// mechanism is exercised.
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};

use std::fs;
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Minimal set of video extensions the watcher recognizes.
const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "wmv", "flv", "ts", "mov", "webm"];

#[test]
fn test_debouncer_detects_new_file() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(1), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    // Create a new file.
    let file_path = dir.path().join("test.mp4");
    fs::write(&file_path, [0u8; 2048]).unwrap();

    // Wait up to 10 s for the debounced event.
    let deadline = Instant::now() + Duration::from_secs(10);
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            let canonical = file_path.canonicalize().unwrap();
            if event.path == file_path || event.path == canonical {
                assert!(matches!(
                    event.kind,
                    DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                ));
                return;
            }
        }
    }

    panic!("did not receive create event for {:?}", file_path);
}

#[test]
fn test_debouncer_detects_multiple_files() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(1), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    fs::write(dir.path().join("a.mp4"), [0u8; 1024]).unwrap();
    fs::write(dir.path().join("b.mkv"), [0u8; 1024]).unwrap();
    fs::write(dir.path().join("c.avi"), [0u8; 1024]).unwrap();

    let mut seen = std::collections::HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            if let Some(name) = event.path.file_name().and_then(|n| n.to_str()) {
                seen.insert(name.to_string());
            }
        }

        if seen.len() >= 3 {
            break;
        }
    }

    assert!(seen.contains("a.mp4"), "missing a.mp4, seen: {:?}", seen);
    assert!(seen.contains("b.mkv"), "missing b.mkv, seen: {:?}", seen);
    assert!(seen.contains("c.avi"), "missing c.avi, seen: {:?}", seen);
}

#[test]
fn test_debouncer_detects_file_rename() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(1), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    // Create a temp file then rename to .mp4 (mimics browser download behavior).
    let tmp = dir.path().join("download.tmp");
    fs::write(&tmp, [0u8; 4096]).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let final_path = dir.path().join("download.mp4");
    fs::rename(&tmp, &final_path).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            let canonical = final_path.canonicalize().unwrap();
            if event.path == final_path || event.path == canonical {
                assert!(matches!(
                    event.kind,
                    DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                ));
                return;
            }
        }
    }

    panic!("did not receive rename event for {:?}", final_path);
}

#[test]
fn test_debouncer_detects_nested_directory() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(1), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    let sub = dir.path().join("subdir");
    fs::create_dir(&sub).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let nested = sub.join("nested.mkv");
    fs::write(&nested, [0u8; 2048]).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            let canonical = nested.canonicalize().unwrap();
            if event.path == nested || event.path == canonical {
                assert!(matches!(
                    event.kind,
                    DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                ));
                return;
            }
        }
    }

    panic!("did not receive create event for nested file {:?}", nested);
}

/// Emulate a large-file copy: write chunks over several seconds.
/// Windows often reports partial writes before the final close.
#[test]
fn test_debouncer_stability_check() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(2), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    let file_path = dir.path().join("big.mp4");

    // Write initial chunk and wait — debouncer should NOT fire yet for
    // the "final" event since we'll keep writing.
    let mut f = fs::File::create(&file_path).unwrap();
    f.write_all(&[0u8; 1024]).unwrap();
    f.flush().unwrap();
    std::thread::sleep(Duration::from_millis(800));
    // Write more data — extends the debounce window.
    f.write_all(&[0u8; 2048]).unwrap();
    f.flush().unwrap();
    std::thread::sleep(Duration::from_millis(800));
    f.write_all(&[0u8; 4096]).unwrap();
    f.flush().unwrap();
    drop(f); // close the file handle (important on Windows)

    // After closing, the debouncer should fire within its 2 s window.
    let deadline = Instant::now() + Duration::from_secs(10);
    let final_size: u64 = 1024 + 2048 + 4096;
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            let canonical = file_path.canonicalize().unwrap();
            if (event.path == file_path || event.path == canonical) && event.path.is_file() {
                let meta = std::fs::metadata(&event.path).unwrap();
                assert_eq!(meta.len(), final_size, "file should have complete size");
                assert!(matches!(
                    event.kind,
                    DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
                ));
                return;
            }
        }
    }

    panic!("did not receive final write event for {:?}", file_path);
}

/// Verify that non-video files do NOT cause the watcher to fire
/// relevant events (the watcher filters by extension elsewhere,
/// but we test the raw debouncer here to confirm it emits all
/// file-system events indiscriminately).
#[test]
fn test_debouncer_receives_non_video_files() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(1), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    let txt = dir.path().join("readme.txt");
    let jpg = dir.path().join("cover.jpg");
    fs::write(&txt, b"hello").unwrap();
    fs::write(&jpg, [0u8; 512]).unwrap();

    let mut found_txt = false;
    let mut found_jpg = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            let path = &event.path;
            let path_buf = path.to_path_buf();
            if path_buf == txt || path_buf == txt.canonicalize().unwrap() {
                found_txt = true;
            }
            if path_buf == jpg || path_buf == jpg.canonicalize().unwrap() {
                found_jpg = true;
            }
        }

        if found_txt && found_jpg {
            return;
        }
    }

    if !found_txt {
        panic!("did not receive event for readme.txt");
    }
    if !found_jpg {
        panic!("did not receive event for cover.jpg");
    }
}

/// Integration test: simulate what `run_watch` does with the debouncer
/// pipeline — receive debounced events and check extension filtering.
#[test]
fn test_watch_pipeline_extension_filter() {
    let dir = tempfile::tempdir().unwrap();

    let (tx, rx) = mpsc::channel();
    let mut debouncer =
        new_debouncer(Duration::from_secs(1), tx).expect("debouncer creation should succeed");
    debouncer
        .watcher()
        .watch(dir.path(), notify::RecursiveMode::Recursive)
        .expect("watch should succeed");

    // Create both video and non-video files.
    fs::write(dir.path().join("valid.mp4"), vec![0u8; 1024 * 1024]).unwrap();
    fs::write(dir.path().join("valid.avi"), vec![0u8; 1024 * 1024]).unwrap();
    fs::write(dir.path().join("skip.txt"), [0u8; 512]).unwrap();
    fs::write(dir.path().join("skip.jpg"), [0u8; 512]).unwrap();

    let mut video_count = 0;
    let mut non_video_count = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while deadline > Instant::now() {
        let events = rx
            .recv_timeout(deadline - Instant::now())
            .expect("should receive events within timeout")
            .expect("should not receive errors");

        for event in &events {
            let ext = event
                .path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());
            if ext.as_deref().is_some_and(|e| VIDEO_EXTS.contains(&e)) {
                video_count += 1;
            } else {
                non_video_count += 1;
            }
        }

        if video_count >= 2 && non_video_count >= 2 {
            break;
        }
    }

    assert!(video_count >= 2, "should detect at least 2 video files");
    assert!(
        non_video_count >= 2,
        "should detect at least 2 non-video files (debouncer sees all files)"
    );
}
