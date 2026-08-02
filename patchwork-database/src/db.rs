#[cfg(feature = "sqlite")]
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use crate::error::{DatabaseError, Result};

#[cfg(feature = "sqlite")]
pub(crate) type DbConnection = diesel::sqlite::SqliteConnection;
#[cfg(feature = "mysql")]
pub(crate) type DbConnection = diesel::mysql::MysqlConnection;

pub(crate) type DbPool = Pool<ConnectionManager<DbConnection>>;
pub(crate) type DbPooledConnection = PooledConnection<ConnectionManager<DbConnection>>;

#[cfg(feature = "sqlite")]
const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite/");
#[cfg(feature = "mysql")]
const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/mysql/");

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub database_url: String,
    pub max_pool_size: u32,
    pub test_on_check_out: bool,
}

impl DatabaseConfig {
    pub fn new(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            max_pool_size: 15,
            test_on_check_out: true,
        }
    }
}

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: DbPool,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Database")
            .field("pool_state", &self.pool.state())
            .finish()
    }
}

impl Database {
    pub fn connect(database_url: impl Into<String>) -> Result<Self> {
        Self::connect_with_config(DatabaseConfig::new(database_url))
    }

    pub fn connect_with_config(config: DatabaseConfig) -> Result<Self> {
        let manager = ConnectionManager::<DbConnection>::new(config.database_url);
        let builder = Pool::builder()
            .max_size(config.max_pool_size)
            .test_on_check_out(config.test_on_check_out);

        #[cfg(feature = "sqlite")]
        let builder = builder.connection_customizer(Box::new(SqliteConnectionCustomizer));

        let pool = builder.build(manager)?;
        let database = Self { pool };
        database.run_migrations()?;
        Ok(database)
    }

    pub fn run_migrations(&self) -> Result<()> {
        let mut connection = self.connection()?;
        (&mut *connection)
            .run_pending_migrations(MIGRATIONS)
            .map_err(|error| DatabaseError::Migration(error.to_string()))?;
        Ok(())
    }

    pub(crate) fn connection(&self) -> Result<DbPooledConnection> {
        Ok(self.pool.get()?)
    }
}

#[cfg(feature = "sqlite")]
#[derive(Debug)]
struct SqliteConnectionCustomizer;

#[cfg(feature = "sqlite")]
impl diesel::r2d2::CustomizeConnection<DbConnection, diesel::r2d2::Error>
    for SqliteConnectionCustomizer
{
    fn on_acquire(
        &self,
        connection: &mut DbConnection,
    ) -> std::result::Result<(), diesel::r2d2::Error> {
        diesel::sql_query("PRAGMA foreign_keys = ON")
            .execute(connection)
            .map_err(diesel::r2d2::Error::QueryError)?;
        diesel::sql_query("PRAGMA busy_timeout = 5000")
            .execute(connection)
            .map_err(diesel::r2d2::Error::QueryError)?;
        Ok(())
    }
}
