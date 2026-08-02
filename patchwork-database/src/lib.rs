#![forbid(unsafe_code)]

#[cfg(all(feature = "sqlite", feature = "mysql"))]
compile_error!("Enable exactly one database backend: either `sqlite` or `mysql`, not both.");

#[cfg(not(any(feature = "sqlite", feature = "mysql")))]
compile_error!("Enable one database backend: `sqlite` or `mysql`.");

pub mod db;
pub mod error;
pub mod manifest;
pub mod models;
pub mod schema;
mod validation;

mod ops;

pub use db::{Database, DatabaseConfig};
pub use error::{DatabaseError, Result};
pub use manifest::ModpackManifest;
pub use models::{
    Account, AppToken, CreateAccount, CreatePendingRegistration, CreateRegistryScan,
    CreateRegistryScanEntry, DependencyInput, DependencyKind, GithubAccount, GithubOAuthState, Mod,
    ModVersion, ModVersionDependency, Modpack, ModpackDependency, ModpackWithDependencies,
    OAuthAuthorizationCode, Pagination, PendingRegistration, PendingRegistrationVerification,
    PublishModpack, PublishedMod, PublishedRegistryVersion, RegistryModState,
    RegistryPublishResult, RegistryScan, RegistryScanEntry, RegistryScanWithEntries, Repository,
    WebSession,
};
