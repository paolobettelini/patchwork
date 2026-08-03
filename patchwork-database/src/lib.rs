#![forbid(unsafe_code)]

#[cfg(all(feature = "sqlite", feature = "mysql"))]
compile_error!("Enable exactly one database backend: either `sqlite` or `mysql`, not both.");

#[cfg(not(any(feature = "sqlite", feature = "mysql")))]
compile_error!("Enable one database backend: `sqlite` or `mysql`.");

pub mod db;
pub mod error;
pub mod models;
pub mod schema;
mod validation;

mod ops;

pub use db::{Database, DatabaseConfig};
pub use error::{DatabaseError, Result};
pub use models::{
    Account, AppToken, CreateAccount, CreatePendingRegistration, CreateRegistryScan,
    CreateRegistryScanEntry, GithubAccount, GithubOAuthState, Mod, ModVersion,
    ModVersionDependency, Modpack, ModpackVersion, ModpackVersionDependency,
    OAuthAuthorizationCode, Pagination, PendingRegistration, PendingRegistrationVerification,
    PublishedMod, PublishedModpack, PublishedRegistryVersion, RegistryModState,
    RegistryModpackState, RegistryPublishResult, RegistryScan, RegistryScanEntry,
    RegistryScanWithEntries, Repository, WebSession,
};
