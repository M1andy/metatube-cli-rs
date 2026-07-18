use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Client initialization error: {0}")]
    ClientInit(String),

    #[error("API error ({code}): {message}")]
    Api { code: u16, message: String },

    #[error("No search results for: {0}")]
    NoResults(String),

    #[error("No movie info found for: {provider}/{id}")]
    NoMovieInfo { provider: String, id: String },

    #[error("Failed to extract ID from filename: {0}")]
    IdExtraction(String),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Move failed: {src} -> {dst}: {reason}")]
    MoveFailed {
        src: PathBuf,
        dst: PathBuf,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_display_id_extraction() {
        let e = Error::IdExtraction("bad_file.txt".into());
        assert_eq!(
            e.to_string(),
            "Failed to extract ID from filename: bad_file.txt"
        );
    }

    #[test]
    fn test_display_no_results() {
        let e = Error::NoResults("ABC-123".into());
        assert_eq!(e.to_string(), "No search results for: ABC-123");
    }

    #[test]
    fn test_display_api_error() {
        let e = Error::Api {
            code: 404,
            message: "Not Found".into(),
        };
        assert_eq!(e.to_string(), "API error (404): Not Found");
    }

    #[test]
    fn test_display_file_not_found() {
        let e = Error::FileNotFound(PathBuf::from("/tmp/missing.mp4"));
        assert_eq!(e.to_string(), "File not found: /tmp/missing.mp4");
    }

    #[test]
    fn test_display_move_failed() {
        let e = Error::MoveFailed {
            src: PathBuf::from("/src/a.mp4"),
            dst: PathBuf::from("/dst/a.mp4"),
            reason: "permission denied".into(),
        };
        assert_eq!(
            e.to_string(),
            "Move failed: /src/a.mp4 -> /dst/a.mp4: permission denied"
        );
    }

    #[test]
    fn test_display_no_movie_info() {
        let e = Error::NoMovieInfo {
            provider: "fanza".into(),
            id: "ssis00123".into(),
        };
        assert_eq!(e.to_string(), "No movie info found for: fanza/ssis00123");
    }

    #[test]
    fn test_io_error_from() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file missing");
        let e = Error::from(io_err);
        assert!(e.to_string().contains("IO error"));
    }

    #[test]
    fn test_display_client_init() {
        let e = Error::ClientInit("bad proxy".into());
        assert_eq!(e.to_string(), "Client initialization error: bad proxy");
    }
}
