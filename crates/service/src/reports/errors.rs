//! Typed failures exposed by report generation and Confluence uploads.

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("{0}")]
    Validation(String),
    #[error("report queue is full")]
    QueueFull,
    #[error("report worker is unavailable")]
    WorkerUnavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ConfluenceError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Configuration(String),
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
