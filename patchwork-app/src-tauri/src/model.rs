use portable_pty::{Child as PtyChild, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::paths::{display_path, expand_env_vars};

pub(crate) const SETTINGS_FILE: &str = "settings.json";
pub(crate) const SETTINGS_POINTER_FILE: &str = "settings-path.json";
pub(crate) const AUTH_FILE: &str = "auth.json";
pub(crate) const CONFIG_DIR: &str = "config";
pub(crate) const PATCHWORK_CONSOLE_EVENT: &str = "patchwork-console";
pub(crate) const PATCHWORK_AUTH_EVENT: &str = "patchwork-auth";
pub(crate) const DEFAULT_AUTH_SERVER_URL: &str = "http://127.0.0.1:8080";
pub(crate) const DEFAULT_DESCRIPTION: &str = "A new Patchwork modpack.";
pub(crate) const DEFAULT_TERMINAL_ROWS: u16 = 24;
pub(crate) const DEFAULT_TERMINAL_COLS: u16 = 100;
pub(crate) const MAX_CONSOLE_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct AppState {
    pub(crate) settings_pointer_path: PathBuf,
    pub(crate) settings_path: Mutex<PathBuf>,
    pub(crate) settings: Mutex<LauncherSettings>,
    pub(crate) auth_path: PathBuf,
    pub(crate) auth: Mutex<StoredAuthState>,
    pub(crate) tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
}

#[derive(Default)]
pub(crate) struct PatchworkTaskState {
    pub(crate) running: bool,
    pub(crate) action: Option<String>,
    pub(crate) child: Option<Box<dyn PtyChild + Send + Sync>>,
    pub(crate) pty_master: Option<Box<dyn MasterPty + Send>>,
    pub(crate) pty_writer: Option<Box<dyn Write + Send>>,
    pub(crate) terminal_size: Option<PtySize>,
    pub(crate) stop_requested: bool,
    pub(crate) output: String,
    pub(crate) output_bytes: Vec<u8>,
    pub(crate) output_cursor: u64,
    pub(crate) core_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherSettings {
    #[serde(default = "default_theme")]
    pub(crate) theme: String,
    #[serde(default)]
    pub(crate) cargo_target_dir: String,
    #[serde(default)]
    pub(crate) mod_cache: String,
    #[serde(default)]
    pub(crate) modpacks_cache: String,
    #[serde(default, alias = "modpacksDir")]
    pub(crate) profiles_dir: String,
    #[serde(default)]
    pub(crate) build_cache: String,
    #[serde(default)]
    pub(crate) settings_file: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAuthState {
    #[serde(default = "default_auth_server_url")]
    pub(crate) server_url: String,
    #[serde(default)]
    pub(crate) access_token: Option<String>,
    #[serde(default)]
    pub(crate) profile: Option<AuthProfile>,
}

impl Default for StoredAuthState {
    fn default() -> Self {
        Self {
            server_url: default_auth_server_url(),
            access_token: None,
            profile: None,
        }
    }
}

impl StoredAuthState {
    pub(crate) fn status(&self) -> LauncherAuthStatus {
        LauncherAuthStatus {
            server_url: self.server_url.clone(),
            profile: self.profile.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherAuthStatus {
    pub(crate) server_url: String,
    pub(crate) profile: Option<AuthProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

impl LauncherSettings {
    pub(crate) fn default_for(patchwork_data: &Path) -> Self {
        let config_dir = patchwork_data.join(CONFIG_DIR);
        let cache_dir = patchwork_data.join("cache");
        Self {
            theme: default_theme(),
            cargo_target_dir: display_path(&patchwork_data.join("target")),
            mod_cache: display_path(&cache_dir.join("mods")),
            modpacks_cache: display_path(&cache_dir.join("modpacks")),
            profiles_dir: display_path(&patchwork_data.join("profiles")),
            build_cache: display_path(&cache_dir.join("build")),
            settings_file: display_path(&config_dir.join(SETTINGS_FILE)),
        }
    }

    pub(crate) fn fill_missing(mut self, defaults: &Self) -> Self {
        if self.theme.trim().is_empty() {
            self.theme = defaults.theme.clone();
        }
        if self.cargo_target_dir.trim().is_empty() {
            self.cargo_target_dir = defaults.cargo_target_dir.clone();
        }
        if self.mod_cache.trim().is_empty() {
            self.mod_cache = defaults.mod_cache.clone();
        }
        if self.modpacks_cache.trim().is_empty() {
            self.modpacks_cache = defaults.modpacks_cache.clone();
        }
        if self.profiles_dir.trim().is_empty() {
            self.profiles_dir = defaults.profiles_dir.clone();
        }
        if self.build_cache.trim().is_empty() {
            self.build_cache = defaults.build_cache.clone();
        }
        if self.settings_file.trim().is_empty() {
            self.settings_file = defaults.settings_file.clone();
        }
        self
    }

    pub(crate) fn expand_paths(mut self) -> Self {
        self.cargo_target_dir = expand_env_vars(&self.cargo_target_dir);
        self.mod_cache = expand_env_vars(&self.mod_cache);
        self.modpacks_cache = expand_env_vars(&self.modpacks_cache);
        self.profiles_dir = expand_env_vars(&self.profiles_dir);
        self.build_cache = expand_env_vars(&self.build_cache);
        self.settings_file = expand_env_vars(&self.settings_file);
        self
    }

    pub(crate) fn directory_paths(&self) -> [&str; 5] {
        [
            &self.cargo_target_dir,
            &self.mod_cache,
            &self.modpacks_cache,
            &self.profiles_dir,
            &self.build_cache,
        ]
    }
}

#[derive(Clone, Debug, Serialize)]
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
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectedIconFile {
    pub(crate) path: String,
    pub(crate) data_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherDependencyPage {
    pub(crate) kind: String,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) editable_profile: bool,
    pub(crate) distinct_dependency_count: usize,
    pub(crate) modpacks: Vec<patchwork::DependencyEntry>,
    pub(crate) mods: Vec<patchwork::DependencyEntry>,
    pub(crate) diagnostics: Vec<patchwork::DependencyDiagnostic>,
    pub(crate) icon_data_url: Option<String>,
    pub(crate) icon_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchworkConsoleEvent {
    pub(crate) profile_id: String,
    pub(crate) reset: bool,
    pub(crate) line: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) chunk: Option<String>,
    pub(crate) running: bool,
    pub(crate) action: Option<String>,
    pub(crate) runnable: Option<bool>,
    pub(crate) core_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchworkConsoleChunk {
    pub(crate) profile_id: String,
    pub(crate) start_offset: u64,
    pub(crate) end_offset: u64,
    pub(crate) bytes: String,
    pub(crate) reset: bool,
    pub(crate) running: bool,
    pub(crate) action: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LauncherModpackToml {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) color: Option<String>,
    #[serde(default)]
    pub(crate) version: Option<String>,
    #[serde(default)]
    pub(crate) modpacks: Vec<String>,
    #[serde(default)]
    pub(crate) mods: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct NewModpackToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) modpacks: Vec<String>,
    #[serde(default)]
    pub(crate) ignore: Vec<String>,
    #[serde(default)]
    pub(crate) mods: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsPointer {
    pub(crate) settings_file: String,
}

pub(crate) fn default_theme() -> String {
    "dark".to_string()
}

pub(crate) fn default_auth_server_url() -> String {
    DEFAULT_AUTH_SERVER_URL.to_string()
}
