use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppTab {
    Home,
    Browse,
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
    pub(crate) editable_profile: bool,
    pub(crate) distinct_dependency_count: usize,
    pub(crate) modpacks: Vec<DependencyEntry>,
    pub(crate) mods: Vec<DependencyEntry>,
    pub(crate) diagnostics: Vec<DependencyDiagnostic>,
    pub(crate) icon_data_url: Option<String>,
    pub(crate) icon_version: String,
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

pub(crate) const THEMES: [(&str, &str); 8] = [
    ("dark", "Dark"),
    ("dim-white", "Bianco scuro"),
    ("aurora", "Aurora"),
    ("volcanic", "Volcanic"),
    ("nebula", "Nebula"),
    ("moss", "Moss"),
    ("bubblegum", "Bubblegum"),
    ("terminal", "Terminal"),
];
