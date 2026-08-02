use thiserror::Error;

pub type Result<T> = std::result::Result<T, DatabaseError>;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("database pool error: {0}")]
    Pool(#[from] diesel::r2d2::PoolError),

    #[error("database query error: {0}")]
    Query(#[from] diesel::result::Error),

    #[error("database migration error: {0}")]
    Migration(String),

    #[error("invalid {field}: {message}")]
    Validation {
        field: &'static str,
        message: String,
    },

    #[error("{entity} `{id}` was not found")]
    NotFound { entity: &'static str, id: String },

    #[error("{entity} `{key}` already exists")]
    Conflict { entity: &'static str, key: String },

    #[error("invalid dependency kind stored in the database: `{0}`")]
    InvalidDependencyKind(String),

    #[error("invalid modpack manifest: {0}")]
    Manifest(#[from] toml::de::Error),
}

pub(crate) fn map_write_error(
    error: diesel::result::Error,
    entity: &'static str,
    key: impl Into<String>,
) -> DatabaseError {
    if matches!(
        &error,
        diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _)
    ) {
        DatabaseError::Conflict {
            entity,
            key: key.into(),
        }
    } else {
        DatabaseError::Query(error)
    }
}
