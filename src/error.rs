use std::path::PathBuf;

impl Error {
    pub fn error_type(&self) -> &str {
        match self {
            Error::Io(_) => "Io",
            Error::Http(_) => "Http",
            Error::ClientInit(_) => "ClientInit",
            Error::Api { .. } => "Api",
            Error::NoResults(_) => "NoResults",
            Error::NoMovieInfo { .. } => "NoMovieInfo",
            Error::IdExtraction(_) => "IdExtraction",
            Error::FileNotFound(_) => "FileNotFound",
            Error::MoveFailed { .. } => "MoveFailed",
        }
    }

    #[allow(dead_code)]
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Io(e) => {
                matches!(e.kind(), std::io::ErrorKind::PermissionDenied)
                    || (e.kind() == std::io::ErrorKind::Other
                        && e.to_string().to_lowercase().contains("no space"))
            }
            Error::Http(_) => true,
            Error::ClientInit(_) => true,
            Error::Api { code, .. } => *code >= 500,
            Error::NoResults(_) => false,
            Error::NoMovieInfo { .. } => false,
            Error::IdExtraction(_) => false,
            Error::FileNotFound(_) => true,
            Error::MoveFailed { reason, .. } => {
                let r = reason.to_lowercase();
                r.contains("permission denied")
                    || r.contains("access is denied")
                    || r.contains("cross-device")
            }
        }
    }
}

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

    #[test]
    fn test_error_type() {
        assert_eq!(
            Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "")).error_type(),
            "Io"
        );
        assert_eq!(Error::ClientInit("x".into()).error_type(), "ClientInit");
        assert_eq!(
            Error::Api {
                code: 500,
                message: "x".into()
            }
            .error_type(),
            "Api"
        );
        assert_eq!(Error::NoResults("x".into()).error_type(), "NoResults");
        assert_eq!(
            Error::NoMovieInfo {
                provider: "x".into(),
                id: "x".into()
            }
            .error_type(),
            "NoMovieInfo"
        );
        assert_eq!(Error::IdExtraction("x".into()).error_type(), "IdExtraction");
        assert_eq!(
            Error::FileNotFound(PathBuf::from("x")).error_type(),
            "FileNotFound"
        );
        assert_eq!(
            Error::MoveFailed {
                src: PathBuf::from("a"),
                dst: PathBuf::from("b"),
                reason: "x".into()
            }
            .error_type(),
            "MoveFailed"
        );
    }

    #[test]
    fn test_is_retryable_api_5xx() {
        assert!(Error::Api {
            code: 500,
            message: "Internal Server Error".into()
        }
        .is_retryable());
        assert!(Error::Api {
            code: 502,
            message: "Bad Gateway".into()
        }
        .is_retryable());
        assert!(Error::Api {
            code: 503,
            message: "Service Unavailable".into()
        }
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_api_4xx_not_retryable() {
        assert!(!Error::Api {
            code: 400,
            message: "Bad Request".into()
        }
        .is_retryable());
        assert!(!Error::Api {
            code: 401,
            message: "Unauthorized".into()
        }
        .is_retryable());
        assert!(!Error::Api {
            code: 404,
            message: "Not Found".into()
        }
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_io_permission() {
        assert!(Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied"
        ))
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_io_nospace() {
        assert!(Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "no space left on device"
        ))
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_io_notfound_not_retryable() {
        assert!(!Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found"
        ))
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_move_failed_permission() {
        assert!(Error::MoveFailed {
            src: PathBuf::from("a"),
            dst: PathBuf::from("b"),
            reason: "Permission denied".into()
        }
        .is_retryable());
        assert!(Error::MoveFailed {
            src: PathBuf::from("a"),
            dst: PathBuf::from("b"),
            reason: "Access is denied".into()
        }
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_move_failed_cross_device() {
        assert!(Error::MoveFailed {
            src: PathBuf::from("a"),
            dst: PathBuf::from("b"),
            reason: "Invalid cross-device link".into()
        }
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_non_retryable_errors() {
        assert!(!Error::IdExtraction("x".into()).is_retryable());
        assert!(!Error::NoResults("x".into()).is_retryable());
        assert!(!Error::NoMovieInfo {
            provider: "x".into(),
            id: "x".into()
        }
        .is_retryable());
    }

    #[test]
    fn test_is_retryable_client_init() {
        assert!(Error::ClientInit("bad proxy config".into()).is_retryable());
    }

    #[test]
    fn test_is_retryable_file_not_found() {
        assert!(Error::FileNotFound(PathBuf::from("missing.mp4")).is_retryable());
    }
}
