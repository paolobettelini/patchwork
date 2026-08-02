#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryScanRequest {
    pub repository_url: String,
    #[serde(default)]
    pub base_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryRepository {
    pub github_repository_id: i64,
    pub owner: String,
    pub name: String,
    pub canonical_url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistryScanStatus {
    NewMod,
    NewVersion,
    Unchanged,
    VersionConflict,
    Error,
}

impl RegistryScanStatus {
    pub const fn is_publishable(self) -> bool {
        matches!(self, Self::NewMod | Self::NewVersion)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryDependencyKind {
    Init,
    Run,
    Ownership,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryDependency {
    pub kind: RegistryDependencyKind,
    pub target_id: String,
    pub available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryScanEntry {
    pub entry_id: String,
    pub mod_id: String,
    pub title: String,
    pub version: String,
    pub repository_path: String,
    pub manifest_path: String,
    pub source_tree_oid: String,
    pub manifest_blob_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub readme_blob_oid: Option<String>,
    pub image_path: Option<String>,
    pub image_blob_oid: Option<String>,
    pub status: RegistryScanStatus,
    pub dependencies: Vec<RegistryDependency>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl RegistryScanEntry {
    pub fn is_publishable(&self) -> bool {
        self.status.is_publishable() && self.errors.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryScan {
    pub scan_id: String,
    pub repository: RegistryRepository,
    pub base_path: String,
    pub requested_ref: String,
    pub resolved_commit: String,
    pub created_at: String,
    pub expires_at: String,
    pub published_at: Option<String>,
    pub entries: Vec<RegistryScanEntry>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistryScanPhase {
    Queued,
    Authorizing,
    IndexingRepository,
    FetchingManifests,
    ValidatingMods,
    Persisting,
    Complete,
    Failed,
}

impl RegistryScanPhase {
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Complete | Self::Failed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryScanJobStarted {
    pub job_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryScanProgress {
    pub job_id: String,
    pub phase: RegistryScanPhase,
    pub completed: u32,
    pub total: Option<u32>,
    pub entries: Vec<RegistryScanEntry>,
    pub scan: Option<RegistryScan>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPublishRequest {
    pub entry_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPublishedVersion {
    pub mod_id: String,
    pub version: String,
    pub version_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPublishResponse {
    pub scan_id: String,
    pub published: Vec<RegistryPublishedVersion>,
}
