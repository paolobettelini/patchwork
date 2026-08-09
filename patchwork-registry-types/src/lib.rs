#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub fn is_generated_mod_id(id: &str) -> bool {
    id.contains("generated")
}

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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistryProjectKind {
    Mod,
    Modpack,
}

impl RegistryProjectKind {
    pub const fn route_segment(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::Modpack => "modpacks",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryProjectRef {
    pub project_kind: RegistryProjectKind,
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryBrowseRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_true")]
    pub include_mods: bool,
    #[serde(default = "default_true")]
    pub include_modpacks: bool,
}

impl Default for RegistryBrowseRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            include_mods: true,
            include_modpacks: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistryBrowseSource {
    Remote,
    Local,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryBrowseProject {
    pub project_kind: RegistryProjectKind,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub downloads: i64,
    pub source: RegistryBrowseSource,
    pub source_label: String,
    #[serde(default)]
    pub repository_url: Option<String>,
    #[serde(default)]
    pub repository_path: Option<String>,
    #[serde(default)]
    pub source_commit: Option<String>,
    #[serde(default)]
    pub source_tree_oid: Option<String>,
    #[serde(default)]
    pub manifest_sha256: Option<String>,
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default)]
    pub readme_url: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub local_manifest_path: Option<String>,
}

impl RegistryBrowseProject {
    pub fn project_ref(&self) -> RegistryProjectRef {
        RegistryProjectRef {
            project_kind: self.project_kind,
            project_id: self.project_id.clone(),
        }
    }
}

pub fn registry_search_rank(project: &RegistryBrowseProject, query: &str) -> u8 {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 0;
    }

    let id = project.project_id.to_lowercase();
    let title = project.title.to_lowercase();
    if id == query {
        0
    } else if title == query {
        1
    } else if id.starts_with(&query) {
        2
    } else if title.starts_with(&query) {
        3
    } else if id.contains(&query) {
        4
    } else if title.contains(&query) {
        5
    } else if project.description.to_lowercase().contains(&query) {
        6
    } else if project.source_label.to_lowercase().contains(&query) {
        7
    } else {
        8
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryBrowseResponse {
    pub projects: Vec<RegistryBrowseProject>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryProjectDetails {
    pub project_kind: RegistryProjectKind,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub downloads: Option<i64>,
    pub publisher_uuid: String,
    pub publisher_name: String,
    pub published_at: String,
    pub repository_url: String,
    pub repository_path: String,
    pub source_commit: String,
    pub source_tree_oid: String,
    pub manifest_sha256: String,
    pub manifest_url: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub readme_url: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<RegistryDependency>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryProfileOption {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryAddToProfileRequest {
    pub project: RegistryProjectRef,
    #[serde(default)]
    pub selected_project: Option<RegistryBrowseProject>,
    pub profile_id: String,
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
    Provides,
    Mod,
    Modpack,
    Ignore,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryDependency {
    pub kind: RegistryDependencyKind,
    pub target_kind: RegistryProjectKind,
    pub target_id: String,
    pub available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryScanEntry {
    pub entry_id: String,
    pub project_kind: RegistryProjectKind,
    pub project_id: String,
    pub title: String,
    pub description: String,
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
    ValidatingProjects,
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
    pub project_kind: RegistryProjectKind,
    pub project_id: String,
    pub version: String,
    pub version_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPublishResponse {
    pub scan_id: String,
    pub published: Vec<RegistryPublishedVersion>,
}

const fn default_true() -> bool {
    true
}
