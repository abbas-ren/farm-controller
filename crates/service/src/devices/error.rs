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

impl From<sea_orm::DbErr> for DeviceRepositoryError {
    fn from(err: sea_orm::DbErr) -> Self {
        use sea_orm::{DbErr, RuntimeErr};
        let sqlx_err = match err {
            DbErr::Query(RuntimeErr::SqlxError(err))
            | DbErr::Exec(RuntimeErr::SqlxError(err))
            | DbErr::Conn(RuntimeErr::SqlxError(err)) => match std::sync::Arc::try_unwrap(err) {
                Ok(err) => err,
                Err(err) => sqlx::Error::Protocol(err.to_string()),
            },
            other => sqlx::Error::Protocol(other.to_string()),
        };
        Self::Database(sqlx_err)
    }
}
