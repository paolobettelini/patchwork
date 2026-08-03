use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppTab {
    Home,
    Browse,
    Upload,
    Project,
    Profile,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsTab {
    General,
    Registries,
    Installation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherSettings {
    pub(crate) theme: String,
    pub(crate) backend: String,
    #[serde(default)]
    pub(crate) local_folders: Vec<String>,
    pub(crate) cargo_target_dir: String,
    pub(crate) mod_cache: String,
    pub(crate) modpacks_cache: String,
    #[serde(alias = "modpacksDir")]
    pub(crate) profiles_dir: String,
    pub(crate) build_cache: String,
    pub(crate) settings_file: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherModpack {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) version: String,
    pub(crate) mods: usize,
    pub(crate) dependencies: usize,
    pub(crate) downloads: String,
    pub(crate) accent: String,
    pub(crate) icon_data_url: Option<String>,
    pub(crate) icon_version: String,
    pub(crate) updates_available: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryInstallReport {
    pub(crate) installed: usize,
    pub(crate) up_to_date: usize,
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherInstallResult {
    pub(crate) profile: LauncherModpack,
    pub(crate) report: RegistryInstallReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryDownloadEvent {
    pub(crate) running: bool,
    pub(crate) phase: String,
    pub(crate) completed: usize,
    pub(crate) total: usize,
    pub(crate) current: Option<String>,
    pub(crate) message: String,
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectedIconFile {
    pub(crate) path: String,
    pub(crate) data_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DependencyPage {
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) version: String,
    pub(crate) editable_profile: bool,
    pub(crate) distinct_dependency_count: usize,
    pub(crate) modpacks: Vec<DependencyEntry>,
    pub(crate) mods: Vec<DependencyEntry>,
    pub(crate) diagnostics: Vec<DependencyDiagnostic>,
    pub(crate) icon_data_url: Option<String>,
    pub(crate) icon_version: String,
    pub(crate) source_kind: String,
    pub(crate) publisher_name: Option<String>,
    pub(crate) published_at: Option<String>,
    pub(crate) downloads: Option<i64>,
    pub(crate) repository_url: Option<String>,
    pub(crate) repository_path: Option<String>,
    pub(crate) source_commit: Option<String>,
    pub(crate) source_tree_oid: Option<String>,
    pub(crate) manifest_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DependencyEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) found: bool,
    pub(crate) ignored: bool,
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DependencyDiagnostic {
    pub(crate) kind: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchworkConsoleEvent {
    pub(crate) profile_id: String,
    pub(crate) reset: bool,
    pub(crate) line: String,
    pub(crate) chunk: Option<String>,
    pub(crate) running: bool,
    pub(crate) action: Option<String>,
    pub(crate) runnable: Option<bool>,
    pub(crate) core_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchworkTaskStatus {
    pub(crate) profile_id: String,
    pub(crate) output: String,
    pub(crate) output_bytes: String,
    pub(crate) running: bool,
    pub(crate) action: Option<String>,
    pub(crate) runnable: bool,
    pub(crate) core_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherAuthStatus {
    pub(crate) server_url: String,
    pub(crate) profile: Option<AuthProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchworkAuthEvent {
    pub(crate) status: LauncherAuthStatus,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthProfile {
    pub(crate) account: AuthAccount,
    #[serde(default)]
    pub(crate) github: Option<GithubAccount>,
    pub(crate) mods: Vec<PublishedProject>,
    pub(crate) modpacks: Vec<PublishedProject>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GithubAccount {
    pub(crate) github_user_id: i64,
    pub(crate) github_login: String,
    pub(crate) github_avatar_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthAccount {
    pub(crate) uuid: String,
    pub(crate) nickname: String,
    pub(crate) email: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishedProject {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    pub(crate) downloads: i64,
    #[serde(default)]
    pub(crate) latest_version: Option<String>,
    #[serde(default)]
    pub(crate) repository_url: Option<String>,
    #[serde(default)]
    pub(crate) repository_path: Option<String>,
    #[serde(default)]
    pub(crate) can_rescan: bool,
}
