#[derive(Debug, thiserror::Error)]
pub(crate) enum DeviceRepositoryError {
    #[error("invalid controller registration: {0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
