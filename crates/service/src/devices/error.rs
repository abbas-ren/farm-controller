//! Persistence and domain errors for the device repository surface.
//!
//! Handlers map these variants to HTTP status codes via [`http_support`].
//! Replaces the ad-hoc error strings thrown by Node `DeviceService`,
//! `RelayService`, and related Sequelize call sites.

/// Failure modes returned by device repository / store operations.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeviceRepositoryError {
    /// Caller-supplied input failed domain validation (maps to HTTP 400).
    #[error("invalid controller registration: {0}")]
    Validation(String),
    /// Requested entity does not exist or is soft-deleted (maps to HTTP 404).
    #[error("{0}")]
    NotFound(String),
    /// Operation conflicts with current state (maps to HTTP 409).
    #[error("{0}")]
    Conflict(String),
    /// Unexpected application failure not tied to SQL (maps to HTTP 500).
    #[error("{0}")]
    Internal(String),
    /// Underlying sqlx / database failure (maps to HTTP 500 after logging).
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl From<sea_orm::DbErr> for DeviceRepositoryError {
    /// PURPOSE: Convert SeaORM errors into the repository error enum.
    /// USE: Store modules that use SeaORM call sites use `?` without mapping manually.
    fn from(err: sea_orm::DbErr) -> Self {
        use sea_orm::{DbErr, RuntimeErr};
        // Prefer the original sqlx error when SeaORM wrapped one so callers see the real DB cause.
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
