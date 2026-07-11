use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

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

