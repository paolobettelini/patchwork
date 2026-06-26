use base64::{Engine, engine::general_purpose};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};

const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_POINTER_FILE: &str = "settings-path.json";
const PATCHWORK_CONSOLE_EVENT: &str = "patchwork-console";
const DEFAULT_DESCRIPTION: &str = "A new Patchwork modpack.";
const ICON_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];
const COLOR_PALETTE: [&str; 8] = [
    "#02a9a9", "#fd614e", "#6268c8", "#fdb22c", "#7df9ff", "#77ff8a", "#ff6bd6", "#ff8a1c",
];

#[derive(Debug)]
struct AppState {
    settings_pointer_path: PathBuf,
    settings_path: Mutex<PathBuf>,
    settings: Mutex<LauncherSettings>,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
}

#[derive(Debug, Default)]
struct PatchworkTaskState {
    running: bool,
    action: Option<String>,
    child: Option<Child>,
    stop_requested: bool,
    output: String,
    core_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LauncherSettings {
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default)]
    cargo_target_dir: String,
    #[serde(default)]
    mod_cache: String,
    #[serde(default)]
    modpacks_cache: String,
    #[serde(default, alias = "modpacksDir")]
    profiles_dir: String,
    #[serde(default)]
    build_cache: String,
    #[serde(default)]
    settings_file: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherModpack {
    id: String,
    name: String,
    description: String,
    version: String,
    mods: usize,
    dependencies: usize,
    downloads: String,
    accent: String,
    icon_data_url: Option<String>,
    icon_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedIconFile {
    path: String,
    data_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherDependencyPage {
    kind: String,
    id: String,
    name: String,
    description: String,
    editable_profile: bool,
    distinct_dependency_count: usize,
    modpacks: Vec<patchwork::DependencyEntry>,
    mods: Vec<patchwork::DependencyEntry>,
    diagnostics: Vec<patchwork::DependencyDiagnostic>,
    icon_data_url: Option<String>,
    icon_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PatchworkConsoleEvent {
    profile_id: String,
    reset: bool,
    line: String,
    running: bool,
    action: Option<String>,
    runnable: Option<bool>,
    core_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PatchworkTaskStatus {
    profile_id: String,
    output: String,
    running: bool,
    action: Option<String>,
    runnable: bool,
    core_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LauncherModpackToml {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    modpacks: Vec<String>,
    #[serde(default)]
    mods: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NewModpackToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    modpacks: Vec<String>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    mods: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPointer {
    settings_file: String,
}

impl LauncherSettings {
    fn default_for(patchwork_data: &Path) -> Self {
        Self {
            theme: default_theme(),
            cargo_target_dir: display_path(&patchwork_data.join("target")),
            mod_cache: display_path(&patchwork_data.join("mods")),
            modpacks_cache: display_path(&patchwork_data.join("modpacks")),
            profiles_dir: display_path(&patchwork_data.join("profiles")),
            build_cache: display_path(&patchwork_data.join("build")),
            settings_file: display_path(&patchwork_data.join(SETTINGS_FILE)),
        }
    }

    fn fill_missing(mut self, defaults: &Self) -> Self {
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

    fn expand_paths(mut self) -> Self {
        self.cargo_target_dir = expand_env_vars(&self.cargo_target_dir);
        self.mod_cache = expand_env_vars(&self.mod_cache);
        self.modpacks_cache = expand_env_vars(&self.modpacks_cache);
        self.profiles_dir = expand_env_vars(&self.profiles_dir);
        self.build_cache = expand_env_vars(&self.build_cache);
        self.settings_file = expand_env_vars(&self.settings_file);
        self
    }

    fn directory_paths(&self) -> [&str; 5] {
        [
            &self.cargo_target_dir,
            &self.mod_cache,
            &self.modpacks_cache,
            &self.profiles_dir,
            &self.build_cache,
        ]
    }
}

fn default_theme() -> String {
    "dark".to_string()
}

#[tauri::command]
fn select_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.display().to_string())
}

#[tauri::command]
fn select_settings_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("JSON settings", &["json"])
        .set_file_name(SETTINGS_FILE)
        .save_file()
        .map(|path| path.display().to_string())
}

#[tauri::command]
fn select_icon_file() -> Result<Option<SelectedIconFile>, String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Images", &ICON_EXTENSIONS)
        .pick_file()
    else {
        return Ok(None);
    };

    Ok(Some(SelectedIconFile {
        path: path.display().to_string(),
        data_url: read_icon_data_url(&path).map_err(|error| error.to_string())?,
    }))
}

#[tauri::command]
fn load_launcher_settings(state: State<AppState>) -> Result<LauncherSettings, String> {
    Ok(state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone())
}

#[tauri::command]
fn update_launcher_path(
    state: State<AppState>,
    field: String,
    value: String,
) -> Result<LauncherSettings, String> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?;
    let value = expand_env_vars(value.trim());
    if value.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    match field.as_str() {
        "cargo_target_dir" => settings.cargo_target_dir = value,
        "mod_cache" => settings.mod_cache = value,
        "modpacks_cache" => settings.modpacks_cache = value,
        "profiles_dir" => settings.profiles_dir = value,
        "build_cache" => settings.build_cache = value,
        "settings_file" => {
            let new_settings_path = PathBuf::from(&value);
            settings.settings_file = display_path(&new_settings_path);
            ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;
            save_settings(&new_settings_path, &settings).map_err(|error| error.to_string())?;
            save_settings_pointer(&state.settings_pointer_path, &new_settings_path)
                .map_err(|error| error.to_string())?;
            *state
                .settings_path
                .lock()
                .map_err(|_| "launcher settings path lock is poisoned".to_string())? =
                new_settings_path;
            return Ok(settings.clone());
        }
        _ => return Err(format!("Unknown launcher setting path '{field}'")),
    }

    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;
    let settings_path = state
        .settings_path
        .lock()
        .map_err(|_| "launcher settings path lock is poisoned".to_string())?
        .clone();
    save_settings(&settings_path, &settings).map_err(|error| error.to_string())?;
    Ok(settings.clone())
}

#[tauri::command]
fn update_launcher_theme(
    state: State<AppState>,
    theme: String,
) -> Result<LauncherSettings, String> {
    let theme = theme.trim();
    if !matches!(
        theme,
        "dark" | "dim-white" | "aurora" | "volcanic" | "nebula" | "moss" | "bubblegum" | "terminal"
    ) {
        return Err(format!("Unknown theme '{theme}'"));
    }

    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?;
    settings.theme = theme.to_string();
    let settings_path = state
        .settings_path
        .lock()
        .map_err(|_| "launcher settings path lock is poisoned".to_string())?
        .clone();
    save_settings(&settings_path, &settings).map_err(|error| error.to_string())?;
    Ok(settings.clone())
}

#[tauri::command]
fn list_modpacks(state: State<AppState>) -> Result<Vec<LauncherModpack>, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;
    read_modpacks(Path::new(&settings.profiles_dir)).map_err(|error| error.to_string())
}

#[tauri::command]
fn update_profile_metadata(
    state: State<AppState>,
    profile_id: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<LauncherDependencyPage, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    if !profile_path.is_file() {
        return Err(format!("Profile '{profile_id}' does not exist"));
    }

    let source = fs::read_to_string(&profile_path).map_err(|error| {
        format!(
            "Failed to read profile '{}': {error}",
            profile_path.display()
        )
    })?;
    let mut table = source.parse::<toml::Table>().map_err(|error| {
        format!(
            "Failed to parse profile '{}': {error}",
            profile_path.display()
        )
    })?;

    let mut changed = false;
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty() {
            return Err("Profile name cannot be empty".to_string());
        }
        if table.get("name").and_then(toml::Value::as_str) != Some(name) {
            table.insert("name".to_string(), toml::Value::String(name.to_string()));
            changed = true;
        }
    }
    if let Some(description) = description {
        let description = description.trim();
        if table.get("description").and_then(toml::Value::as_str) != Some(description) {
            table.insert(
                "description".to_string(),
                toml::Value::String(description.to_string()),
            );
            changed = true;
        }
    }

    if changed {
        let toml = toml::to_string_pretty(&table).map_err(|error| error.to_string())?;
        fs::write(&profile_path, toml).map_err(|error| {
            format!(
                "Failed to update profile '{}': {error}",
                profile_path.display()
            )
        })?;
    }

    let page = patchwork::inspect_dependency_page(
        patchwork::DependencyTarget::Profile { id: &profile_id },
        Path::new(&settings.mod_cache),
        Path::new(&settings.modpacks_cache),
        Path::new(&settings.profiles_dir),
    )
    .map_err(|error| error.to_string())?;
    decorate_dependency_page(page, &settings)
}

#[tauri::command]
fn create_modpack(
    state: State<AppState>,
    id: String,
    name: String,
    description: String,
    icon_path: Option<String>,
) -> Result<LauncherModpack, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let profiles_dir = Path::new(&settings.profiles_dir);
    let id = slugify_modpack_id(&id);
    if id.is_empty() {
        return Err("Modpack ID must contain at least one letter or number".to_string());
    }

    let existing = read_modpacks(profiles_dir).map_err(|error| error.to_string())?;
    if existing
        .iter()
        .any(|modpack| modpack.id.eq_ignore_ascii_case(&id))
    {
        return Err(format!("A modpack with ID '{id}' already exists"));
    }

    let path = profiles_dir.join(format!("{id}.toml"));
    if path.exists() {
        return Err(format!("A modpack file named '{id}.toml' already exists"));
    }

    let modpack = NewModpackToml {
        color: None,
        name: non_empty_or(&name, &id),
        description: non_empty_or(&description, DEFAULT_DESCRIPTION),
        modpacks: Vec::new(),
        ignore: Vec::new(),
        mods: Vec::new(),
    };
    let toml = toml::to_string_pretty(&modpack).map_err(|error| error.to_string())?;
    fs::write(&path, toml)
        .map_err(|error| format!("Failed to create modpack '{}': {error}", path.display()))?;

    if let Some(icon_path) = icon_path.filter(|path| !path.trim().is_empty()) {
        copy_icon_to_profile(&PathBuf::from(icon_path), profiles_dir, &id)?;
    }

    read_modpack_file(&path).map_err(|error| error.to_string())
}

#[tauri::command]
fn import_modpack(state: State<AppState>) -> Result<Option<LauncherModpack>, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;
    let profiles_dir = Path::new(&settings.profiles_dir);

    let Some(source) = rfd::FileDialog::new()
        .add_filter("Patchwork modpacks", &["toml"])
        .pick_file()
    else {
        return Ok(None);
    };
    if source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("toml"))
    {
        return Err("Selected file is not a TOML modpack".to_string());
    }

    let id = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify_modpack_id)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Imported modpack file must have a valid filename".to_string())?;
    let destination = profiles_dir.join(format!("{id}.toml"));
    if destination.exists() {
        return Err(format!("A profile with ID '{id}' already exists"));
    }

    let source_icon = matching_icon_for_modpack_file(&source)?;
    fs::copy(&source, &destination).map_err(|error| {
        format!(
            "Failed to import modpack '{}' to '{}': {error}",
            source.display(),
            destination.display()
        )
    })?;
    if let Some(source_icon) = source_icon {
        copy_icon_to_profile(&source_icon, profiles_dir, &id)?;
    }

    read_modpack_file(&destination)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_modpack(state: State<AppState>, modpack_id: String) -> Result<(), String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let id = sanitize_existing_modpack_id(&modpack_id)?;
    let profiles_dir = Path::new(&settings.profiles_dir);
    let modpack_path = profiles_dir.join(format!("{id}.toml"));
    if !modpack_path.is_file() {
        return Err(format!("Modpack '{id}' does not exist"));
    }

    fs::remove_file(&modpack_path).map_err(|error| {
        format!(
            "Failed to delete modpack '{}': {error}",
            modpack_path.display()
        )
    })?;
    remove_existing_icons(profiles_dir, &id).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn load_dependency_page(
    state: State<AppState>,
    kind: String,
    id: String,
) -> Result<LauncherDependencyPage, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let id = sanitize_existing_modpack_id(&id)?;
    let kind = kind.trim();
    let target = match kind {
        "profile" => patchwork::DependencyTarget::Profile { id: &id },
        "modpack" => patchwork::DependencyTarget::Modpack { id: &id },
        "mod" => patchwork::DependencyTarget::Mod { id: &id },
        _ => return Err(format!("Unknown dependency page kind '{kind}'")),
    };
    let page = patchwork::inspect_dependency_page(
        target,
        Path::new(&settings.mod_cache),
        Path::new(&settings.modpacks_cache),
        Path::new(&settings.profiles_dir),
    )
    .map_err(|error| error.to_string())?;

    decorate_dependency_page(page, &settings)
}

#[tauri::command]
fn toggle_profile_ignore(
    state: State<AppState>,
    profile_id: String,
    mod_id: String,
) -> Result<LauncherDependencyPage, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let mod_id = sanitize_existing_modpack_id(&mod_id)?;
    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    if !profile_path.is_file() {
        return Err(format!("Profile '{profile_id}' does not exist"));
    }

    let source = fs::read_to_string(&profile_path).map_err(|error| {
        format!(
            "Failed to read profile '{}': {error}",
            profile_path.display()
        )
    })?;
    let mut profile = toml::from_str::<NewModpackToml>(&source).unwrap_or(NewModpackToml {
        color: None,
        name: profile_id.clone(),
        description: DEFAULT_DESCRIPTION.to_string(),
        modpacks: Vec::new(),
        ignore: Vec::new(),
        mods: Vec::new(),
    });
    if let Some(index) = profile.ignore.iter().position(|ignored| ignored == &mod_id) {
        profile.ignore.remove(index);
    } else {
        profile.ignore.push(mod_id);
        profile.ignore.sort();
        profile.ignore.dedup();
    }

    let toml = toml::to_string_pretty(&profile).map_err(|error| error.to_string())?;
    fs::write(&profile_path, toml).map_err(|error| {
        format!(
            "Failed to update profile '{}': {error}",
            profile_path.display()
        )
    })?;

    let page = patchwork::inspect_dependency_page(
        patchwork::DependencyTarget::Profile { id: &profile_id },
        Path::new(&settings.mod_cache),
        Path::new(&settings.modpacks_cache),
        Path::new(&settings.profiles_dir),
    )
    .map_err(|error| error.to_string())?;
    decorate_dependency_page(page, &settings)
}

#[tauri::command]
fn is_profile_runnable(
    state: State<AppState>,
    profile_id: String,
    build_mode: String,
) -> Result<bool, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let build_mode = sanitize_build_mode(&build_mode)?;
    Ok(profile_runnable(&settings, &profile_id, &build_mode))
}

#[tauri::command]
fn patchwork_task_status(
    state: State<AppState>,
    profile_id: String,
    build_mode: String,
) -> Result<PatchworkTaskStatus, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let build_mode = sanitize_build_mode(&build_mode)?;
    let runnable = profile_runnable(&settings, &profile_id, &build_mode);
    let tasks = state
        .tasks
        .lock()
        .map_err(|_| "patchwork task lock is poisoned".to_string())?;

    if let Some(task) = tasks.get(&profile_id) {
        Ok(PatchworkTaskStatus {
            profile_id,
            output: if task.output.is_empty() {
                "Console output will appear here.".to_string()
            } else {
                task.output.clone()
            },
            running: task.running,
            action: task.action.clone(),
            runnable,
            core_error: task.core_error.clone(),
        })
    } else {
        Ok(PatchworkTaskStatus {
            profile_id,
            output: "Console output will appear here.".to_string(),
            running: false,
            action: None,
            runnable,
            core_error: None,
        })
    }
}

#[tauri::command]
fn start_patchwork_action(
    app: AppHandle,
    state: State<AppState>,
    profile_id: String,
    action: String,
    build_mode: String,
) -> Result<bool, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let action = action.trim();
    if !matches!(action, "compose-build" | "compose" | "build" | "run") {
        return Err(format!("Unknown patchwork action '{action}'"));
    }
    let build_mode = sanitize_build_mode(&build_mode)?;

    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    if !profile_path.is_file() {
        return Err(format!("Profile '{profile_id}' does not exist"));
    }

    if action == "run" && !profile_runnable(&settings, &profile_id, &build_mode) {
        let expected = executable_path(&settings, &profile_id, &build_mode);
        return Err(format!(
            "Executable for '{profile_id}' does not exist yet at '{}'. Run Build first.",
            expected.display()
        ));
    }

    let tasks = state.tasks.clone();
    {
        let mut tasks = tasks
            .lock()
            .map_err(|_| "patchwork task lock is poisoned".to_string())?;
        let task = tasks.entry(profile_id.clone()).or_default();
        if task.running {
            return Err(format!(
                "Another Patchwork action is already running for '{profile_id}': {}",
                task.action.as_deref().unwrap_or("unknown")
            ));
        }
        task.running = true;
        task.action = Some(action.to_string());
        task.child = None;
        task.stop_requested = false;
        task.core_error = None;
        task.output = format!(
            "Patchwork action: {} ({})\nProfile: {profile_id}\n",
            compose_action_title(action),
            build_mode_label(&build_mode)
        );
    }

    let action = action.to_string();
    emit_console(
        &app,
        PatchworkConsoleEvent {
            profile_id: profile_id.clone(),
            reset: true,
            line: format!(
                "Patchwork action: {} ({})\nProfile: {profile_id}",
                compose_action_title(&action),
                build_mode_label(&build_mode)
            ),
            running: true,
            action: Some(action.clone()),
            runnable: Some(profile_runnable(&settings, &profile_id, &build_mode)),
            core_error: None,
        },
    );

    thread::spawn(move || {
        run_patchwork_task(
            app.clone(),
            tasks.clone(),
            settings.clone(),
            profile_id.clone(),
            profile_path,
            action.clone(),
            build_mode.clone(),
        );

        if let Ok(mut tasks) = tasks.lock() {
            let task = tasks.entry(profile_id.clone()).or_default();
            task.running = false;
            task.action = None;
            task.child = None;
            task.stop_requested = false;
        }
        emit_console(
            &app,
            PatchworkConsoleEvent {
                profile_id: profile_id.clone(),
                reset: false,
                line: String::new(),
                running: false,
                action: None,
                runnable: Some(profile_runnable(&settings, &profile_id, &build_mode)),
                core_error: None,
            },
        );
    });

    Ok(true)
}

#[tauri::command]
fn stop_patchwork_action(
    app: AppHandle,
    state: State<AppState>,
    profile_id: String,
) -> Result<bool, String> {
    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let mut tasks = state
        .tasks
        .lock()
        .map_err(|_| "patchwork task lock is poisoned".to_string())?;
    let Some(task) = tasks.get_mut(&profile_id) else {
        return Err(format!(
            "There is no running cargo process for '{profile_id}'."
        ));
    };
    if let Some(child) = task.child.as_mut() {
        child
            .kill()
            .map_err(|error| format!("Failed to stop running cargo process: {error}"))?;
        task.stop_requested = true;
        append_line_to_task(task, "[run] Stop requested.");
        emit_console(
            &app,
            PatchworkConsoleEvent {
                profile_id,
                reset: false,
                line: "[run] Stop requested.".to_string(),
                running: true,
                action: task.action.clone(),
                runnable: None,
                core_error: None,
            },
        );
        Ok(true)
    } else {
        Err(format!(
            "There is no running cargo process for '{profile_id}'."
        ))
    }
}

#[tauri::command]
fn select_modpack_icon(
    state: State<AppState>,
    modpack_id: String,
) -> Result<Option<LauncherModpack>, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

    let id = sanitize_existing_modpack_id(&modpack_id)?;
    let profiles_dir = Path::new(&settings.profiles_dir);
    let modpack_path = profiles_dir.join(format!("{id}.toml"));
    if !modpack_path.is_file() {
        return Err(format!("Modpack '{id}' does not exist"));
    }

    let Some(source) = rfd::FileDialog::new()
        .add_filter("Images", &ICON_EXTENSIONS)
        .pick_file()
    else {
        return Ok(None);
    };

    copy_icon_to_profile(&source, profiles_dir, &id)?;

    read_modpack_file(&modpack_path)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let tauri_data_dir = app.path().app_data_dir()?;
            fs::create_dir_all(&tauri_data_dir)?;
            let patchwork_data_dir = default_patchwork_data_dir(&tauri_data_dir);
            let default_settings_path = patchwork_data_dir.join(SETTINGS_FILE);
            let settings_pointer_path = tauri_data_dir.join(SETTINGS_POINTER_FILE);
            let settings_path =
                load_settings_pointer(&settings_pointer_path).unwrap_or(default_settings_path);
            let defaults = LauncherSettings::default_for(&patchwork_data_dir);
            let settings = load_settings(&settings_path, &defaults)?;
            ensure_settings_dirs(&settings)?;
            save_settings(&settings_path, &settings)?;
            save_settings_pointer(&settings_pointer_path, &settings_path)?;
            app.manage(AppState {
                settings_pointer_path,
                settings_path: Mutex::new(settings_path),
                settings: Mutex::new(settings),
                tasks: Arc::new(Mutex::new(HashMap::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            select_folder,
            select_settings_file,
            select_icon_file,
            load_launcher_settings,
            update_launcher_path,
            update_launcher_theme,
            list_modpacks,
            update_profile_metadata,
            create_modpack,
            import_modpack,
            delete_modpack,
            load_dependency_page,
            toggle_profile_ignore,
            is_profile_runnable,
            patchwork_task_status,
            start_patchwork_action,
            stop_patchwork_action,
            select_modpack_icon,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Patchwork");
}

fn load_settings(path: &Path, defaults: &LauncherSettings) -> Result<LauncherSettings, io::Error> {
    let mut settings = if path.is_file() {
        let bytes = fs::read(path)?;
        serde_json::from_slice::<LauncherSettings>(&bytes)
            .unwrap_or_else(|_| defaults.clone())
            .fill_missing(defaults)
            .expand_paths()
    } else {
        defaults.clone()
    };
    settings.settings_file = display_path(path);
    Ok(settings)
}

fn save_settings(path: &Path, settings: &LauncherSettings) -> Result<(), io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    fs::write(path, json)
}

fn ensure_settings_dirs(settings: &LauncherSettings) -> Result<(), io::Error> {
    for path in settings.directory_paths() {
        fs::create_dir_all(path)?;
    }
    if let Some(parent) = Path::new(&settings.settings_file).parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn load_settings_pointer(path: &Path) -> Option<PathBuf> {
    let bytes = fs::read(path).ok()?;
    let pointer = serde_json::from_slice::<SettingsPointer>(&bytes).ok()?;
    let expanded = expand_env_vars(pointer.settings_file.trim());
    let settings_file = expanded.trim();
    (!settings_file.is_empty()).then(|| PathBuf::from(settings_file))
}

fn save_settings_pointer(path: &Path, settings_path: &Path) -> Result<(), io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pointer = SettingsPointer {
        settings_file: display_path(settings_path),
    };
    let json = serde_json::to_vec_pretty(&pointer).map_err(io::Error::other)?;
    fs::write(path, json)
}

fn read_modpacks(modpacks_dir: &Path) -> Result<Vec<LauncherModpack>, io::Error> {
    fs::create_dir_all(modpacks_dir)?;
    let mut modpacks = Vec::new();

    for entry in fs::read_dir(modpacks_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("toml"))
        {
            continue;
        }
        match read_modpack_file(&path) {
            Ok(modpack) => modpacks.push(modpack),
            Err(error) => eprintln!("Skipping unreadable modpack '{}': {error}", path.display()),
        }
    }

    modpacks.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(modpacks)
}

fn read_modpack_file(path: &Path) -> Result<LauncherModpack, io::Error> {
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("modpack")
        .to_string();
    let source = fs::read_to_string(path)?;
    let parsed = toml::from_str::<LauncherModpackToml>(&source).unwrap_or(LauncherModpackToml {
        name: None,
        description: None,
        color: None,
        version: None,
        modpacks: Vec::new(),
        mods: Vec::new(),
    });
    let icon_path = matching_icon_for_modpack_file(path).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        )
    })?;
    let icon_data_url = icon_path
        .as_ref()
        .map(|path| read_icon_data_url(path))
        .transpose()?;
    let icon_version = icon_path
        .as_ref()
        .map(|path| icon_version_for(path))
        .transpose()?
        .unwrap_or_else(|| "none".to_string());

    let dependency_count = distinct_dependency_count(&parsed.modpacks, &parsed.mods);

    Ok(LauncherModpack {
        id: id.clone(),
        name: parsed.name.unwrap_or_else(|| id.clone()),
        description: parsed
            .description
            .unwrap_or_else(|| "No description provided yet.".to_string()),
        version: parsed.version.unwrap_or_else(|| "1.21.x".to_string()),
        mods: parsed.mods.len(),
        dependencies: dependency_count,
        downloads: fake_downloads_for(&id),
        accent: parsed
            .color
            .filter(|color| is_valid_hex_color(color))
            .unwrap_or_else(|| deterministic_color_for(&id).to_string()),
        icon_data_url,
        icon_version,
    })
}

fn decorate_dependency_page(
    page: patchwork::DependencyPage,
    settings: &LauncherSettings,
) -> Result<LauncherDependencyPage, String> {
    let icon_path = match page.kind {
        patchwork::DependencyPageKind::Profile => matching_icon_for_modpack_file(
            &Path::new(&settings.profiles_dir).join(format!("{}.toml", page.id)),
        )?,
        patchwork::DependencyPageKind::Modpack => matching_icon_for_modpack_file(
            &Path::new(&settings.modpacks_cache).join(format!("{}.toml", page.id)),
        )?,
        patchwork::DependencyPageKind::Mod => {
            matching_icon_named(&Path::new(&settings.mod_cache).join(&page.id), "favicon")?
        }
    };
    let icon_data_url = icon_path
        .as_ref()
        .map(|path| read_icon_data_url(path))
        .transpose()
        .map_err(|error| error.to_string())?;
    let icon_version = icon_path
        .as_ref()
        .map(|path| icon_version_for(path))
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| "none".to_string());

    let kind = match page.kind {
        patchwork::DependencyPageKind::Profile => "profile",
        patchwork::DependencyPageKind::Modpack => "modpack",
        patchwork::DependencyPageKind::Mod => "mod",
    }
    .to_string();

    Ok(LauncherDependencyPage {
        kind,
        id: page.id,
        name: page.name,
        description: page.description,
        editable_profile: page.editable_profile,
        distinct_dependency_count: page.distinct_dependency_count,
        modpacks: page.modpacks,
        mods: page.mods,
        diagnostics: page.diagnostics,
        icon_data_url,
        icon_version,
    })
}

fn run_patchwork_task(
    app: AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    settings: LauncherSettings,
    profile_id: String,
    profile_path: PathBuf,
    action: String,
    build_mode: String,
) {
    if matches!(action.as_str(), "compose-build" | "compose") {
        emit_console_line(
            &app,
            &tasks,
            &profile_id,
            &action,
            "[compose] Starting composition...",
        );
        match patchwork::compose_with_modpacks(
            &profile_path,
            Some(profile_id.clone()),
            Path::new(&settings.mod_cache),
            Path::new(&settings.modpacks_cache),
            Path::new(&settings.build_cache),
        ) {
            Ok(()) => {
                emit_console(
                    &app,
                    PatchworkConsoleEvent {
                        profile_id: profile_id.clone(),
                        reset: false,
                        line: "[compose] Done.".to_string(),
                        running: true,
                        action: Some(action.clone()),
                        runnable: Some(profile_runnable(&settings, &profile_id, &build_mode)),
                        core_error: None,
                    },
                );
                append_task_line(&tasks, &profile_id, "[compose] Done.", None);
            }
            Err(error) => {
                let error = error.to_string();
                append_task_line(
                    &tasks,
                    &profile_id,
                    &format!("[compose] Failed: {error}"),
                    Some(error.clone()),
                );
                emit_console(
                    &app,
                    PatchworkConsoleEvent {
                        profile_id: profile_id.clone(),
                        reset: false,
                        line: format!("[compose] Failed: {error}"),
                        running: true,
                        action: Some(action),
                        runnable: Some(profile_runnable(&settings, &profile_id, &build_mode)),
                        core_error: Some(error),
                    },
                );
                return;
            }
        }
    }

    if matches!(action.as_str(), "compose-build" | "build") {
        let project_dir = Path::new(&settings.build_cache).join(&profile_id);
        run_cargo_process(
            &app,
            tasks,
            &profile_id,
            &action,
            "build",
            &project_dir,
            &build_mode,
            &settings.cargo_target_dir,
        );
    } else if action == "run" {
        let project_dir = Path::new(&settings.build_cache).join(&profile_id);
        run_cargo_process(
            &app,
            tasks,
            &profile_id,
            &action,
            "run",
            &project_dir,
            &build_mode,
            &settings.cargo_target_dir,
        );
    }
}

fn run_cargo_process(
    app: &AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    cargo_action: &str,
    project_dir: &Path,
    build_mode: &str,
    cargo_target_dir: &str,
) {
    if !project_dir.join("Cargo.toml").is_file() {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!(
                "[{cargo_action}] Failed: composed project not found at '{}'. Run Compose first.",
                project_dir.display()
            ),
        );
        return;
    }

    let mut command = Command::new("cargo");
    command
        .arg(cargo_action)
        .current_dir(project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if build_mode == "release" {
        command.arg("--release");
    }
    if !cargo_target_dir.trim().is_empty() {
        command.env("CARGO_TARGET_DIR", cargo_target_dir);
    }

    emit_console_line(
        app,
        &tasks,
        profile_id,
        action,
        &format!(
            "[{cargo_action}] Command: cargo {cargo_action}{}",
            if build_mode == "release" {
                " --release"
            } else {
                ""
            }
        ),
    );
    if !cargo_target_dir.trim().is_empty() {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!("[{cargo_action}] CARGO_TARGET_DIR={cargo_target_dir}"),
        );
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[{cargo_action}] Failed to run cargo: {error}"),
            );
            return;
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Ok(mut tasks) = tasks.lock() {
        tasks.entry(profile_id.to_string()).or_default().child = Some(child);
    }

    let stdout_reader = stdout.map(|stdout| {
        let app = app.clone();
        let tasks = tasks.clone();
        let profile_id = profile_id.to_string();
        let action = action.to_string();
        thread::spawn(move || stream_process_output(app, tasks, profile_id, action, stdout))
    });
    let stderr_reader = stderr.map(|stderr| {
        let app = app.clone();
        let tasks = tasks.clone();
        let profile_id = profile_id.to_string();
        let action = action.to_string();
        thread::spawn(move || stream_process_output(app, tasks, profile_id, action, stderr))
    });

    loop {
        let status = {
            let mut tasks = match tasks.lock() {
                Ok(tasks) => tasks,
                Err(_) => {
                    emit_console_line(
                        app,
                        &tasks,
                        profile_id,
                        action,
                        "[cargo] Failed: task lock is poisoned.",
                    );
                    break;
                }
            };
            let task = tasks.entry(profile_id.to_string()).or_default();
            match task.child.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        let stopped = task.stop_requested;
                        task.child = None;
                        Some((status, stopped))
                    }
                    Ok(None) => None,
                    Err(error) => {
                        let line =
                            format!("[{cargo_action}] Failed while waiting for cargo: {error}");
                        append_line_to_task(task, &line);
                        emit_console(
                            app,
                            PatchworkConsoleEvent {
                                profile_id: profile_id.to_string(),
                                reset: false,
                                line,
                                running: true,
                                action: Some(action.to_string()),
                                runnable: None,
                                core_error: None,
                            },
                        );
                        task.child = None;
                        break;
                    }
                },
                None => {
                    append_line_to_task(task, &format!("[{cargo_action}] Stopped."));
                    emit_console(
                        app,
                        PatchworkConsoleEvent {
                            profile_id: profile_id.to_string(),
                            reset: false,
                            line: format!("[{cargo_action}] Stopped."),
                            running: false,
                            action: None,
                            runnable: None,
                            core_error: None,
                        },
                    );
                    break;
                }
            }
        };

        if let Some((status, stopped)) = status {
            if stopped {
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{cargo_action}] Stopped."),
                );
            } else if status.success() {
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{cargo_action}] Status: {status}"),
                );
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{cargo_action}] Done."),
                );
            } else {
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{cargo_action}] Status: {status}"),
                );
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{cargo_action}] Failed."),
                );
            }
            break;
        }

        thread::sleep(Duration::from_millis(120));
    }

    if let Some(reader) = stdout_reader {
        let _ = reader.join();
    }
    if let Some(reader) = stderr_reader {
        let _ = reader.join();
    }
}

fn stream_process_output<R>(
    app: AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: String,
    action: String,
    output: R,
) where
    R: io::Read,
{
    for line in BufReader::new(output).lines().map_while(Result::ok) {
        emit_console_line(&app, &tasks, &profile_id, &action, &line);
    }
}

fn emit_console_line(
    app: &AppHandle,
    tasks: &Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    line: &str,
) {
    append_task_line(tasks, profile_id, line, None);
    emit_console(
        app,
        PatchworkConsoleEvent {
            profile_id: profile_id.to_string(),
            reset: false,
            line: line.to_string(),
            running: true,
            action: Some(action.to_string()),
            runnable: None,
            core_error: None,
        },
    );
}

fn emit_console(app: &AppHandle, event: PatchworkConsoleEvent) {
    let _ = app.emit(PATCHWORK_CONSOLE_EVENT, event);
}

fn append_task_line(
    tasks: &Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    line: &str,
    core_error: Option<String>,
) {
    if let Ok(mut tasks) = tasks.lock() {
        let task = tasks.entry(profile_id.to_string()).or_default();
        append_line_to_task(task, line);
        if let Some(core_error) = core_error {
            task.core_error = Some(core_error);
        }
    }
}

fn append_line_to_task(task: &mut PatchworkTaskState, line: &str) {
    if !task.output.is_empty() && !task.output.ends_with('\n') {
        task.output.push('\n');
    }
    task.output.push_str(line);
    task.output.push('\n');
}

fn profile_runnable(settings: &LauncherSettings, profile_id: &str, build_mode: &str) -> bool {
    executable_path(settings, profile_id, build_mode).is_file()
}

fn executable_path(settings: &LauncherSettings, profile_id: &str, build_mode: &str) -> PathBuf {
    let project_dir = Path::new(&settings.build_cache).join(profile_id);
    let package_name =
        composed_package_name(&project_dir).unwrap_or_else(|| profile_id.to_string());
    let target_root = if settings.cargo_target_dir.trim().is_empty() {
        project_dir.join("target")
    } else {
        PathBuf::from(&settings.cargo_target_dir)
    };
    let profile_dir = if build_mode == "debug" {
        "debug"
    } else {
        "release"
    };
    let mut executable = target_root.join(profile_dir).join(package_name);
    if cfg!(windows) {
        executable.set_extension("exe");
    }
    executable
}

fn composed_package_name(project_dir: &Path) -> Option<String> {
    let source = fs::read_to_string(project_dir.join("Cargo.toml")).ok()?;
    let table = source.parse::<toml::Table>().ok()?;
    table
        .get("package")?
        .get("name")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn compose_action_title(action: &str) -> &'static str {
    match action {
        "compose" => "Compose",
        "build" => "Build",
        "run" => "Run",
        _ => "Compose & Build",
    }
}

fn build_mode_label(mode: &str) -> &'static str {
    match mode {
        "debug" => "Debug mode",
        _ => "Release mode",
    }
}

fn read_icon_data_url(path: &Path) -> Result<String, io::Error> {
    let bytes = fs::read(path)?;
    let mime = mime_for_icon_path(path);
    Ok(format!(
        "data:{mime};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

fn matching_icon_for_modpack_file(path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(parent) = path.parent() else {
        return Ok(None);
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(None);
    };

    matching_icon_named(parent, stem)
}

fn matching_icon_named(parent: &Path, stem: &str) -> Result<Option<PathBuf>, String> {
    let mut matches = fs::read_dir(parent)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| {
            candidate
                .file_stem()
                .and_then(|file_stem| file_stem.to_str())
                == Some(stem)
                && supported_icon_extension(candidate).is_some()
        })
        .collect::<Vec<_>>();

    if matches.len() > 1 {
        matches.sort();
        let names = matches
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Multiple favicon files found for '{stem}': {names}. Keep exactly one."
        ));
    }

    Ok(matches.pop())
}

fn copy_icon_to_profile(source: &Path, profiles_dir: &Path, id: &str) -> Result<(), String> {
    let extension = supported_icon_extension(source)
        .ok_or_else(|| "Selected favicon must be png, jpg, jpeg, webp, or gif".to_string())?;
    remove_existing_icons(profiles_dir, id).map_err(|error| error.to_string())?;
    let destination = profiles_dir.join(format!("{id}.{extension}"));
    fs::copy(source, &destination).map_err(|error| {
        format!(
            "Failed to copy favicon '{}' to '{}': {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn remove_existing_icons(profiles_dir: &Path, id: &str) -> Result<(), io::Error> {
    for extension in ICON_EXTENSIONS {
        let path = profiles_dir.join(format!("{id}.{extension}"));
        if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn supported_icon_extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    ICON_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}

fn mime_for_icon_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    }
}

fn icon_version_for(path: &Path) -> Result<String, io::Error> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    Ok(format!("{}:{modified}", metadata.len()))
}

fn default_patchwork_data_dir(tauri_data_dir: &Path) -> PathBuf {
    tauri_data_dir
        .parent()
        .map(|parent| parent.join("patchwork"))
        .unwrap_or_else(|| tauri_data_dir.join("patchwork"))
}

fn deterministic_color_for(id: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let index = hasher.finish() as usize % COLOR_PALETTE.len();
    COLOR_PALETTE[index]
}

fn fake_downloads_for(id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let value = 1.0 + (hasher.finish() % 24_900) as f32 / 1_000.0;
    format!("{value:.1}K")
}

fn distinct_dependency_count(modpacks: &[String], mods: &[String]) -> usize {
    let mut dependencies = modpacks
        .iter()
        .chain(mods.iter())
        .map(|dependency| dependency.trim())
        .filter(|dependency| !dependency.is_empty())
        .collect::<Vec<_>>();
    dependencies.sort_unstable();
    dependencies.dedup();
    dependencies.len()
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn is_valid_hex_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color
            .chars()
            .skip(1)
            .all(|character| character.is_ascii_hexdigit())
}

fn slugify_modpack_id(name: &str) -> String {
    let mut id = String::new();
    let mut previous_dash = false;

    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            id.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if matches!(character, '-' | '_' | ' ' | '.') && !previous_dash && !id.is_empty() {
            id.push('-');
            previous_dash = true;
        }
    }

    while id.ends_with('-') {
        id.pop();
    }
    id
}

fn sanitize_existing_modpack_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("Modpack id cannot be empty".to_string());
    }
    if trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        Ok(trimmed.to_string())
    } else {
        Err(format!("Invalid modpack id '{trimmed}'"))
    }
}

fn sanitize_build_mode(mode: &str) -> Result<String, String> {
    let mode = mode.trim();
    if matches!(mode, "release" | "debug") {
        Ok(mode.to_string())
    } else {
        Err(format!("Unknown build mode '{mode}'"))
    }
}

fn expand_env_vars(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '$' {
            expanded.push(character);
            continue;
        }

        if chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                name.push(next);
            }
            if name.is_empty() {
                expanded.push_str("${}");
            } else if let Ok(value) = std::env::var(&name) {
                expanded.push_str(&value);
            } else {
                expanded.push_str("${");
                expanded.push_str(&name);
                expanded.push('}');
            }
            continue;
        }

        let mut name = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_ascii_alphanumeric() || next == '_' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }

        if name.is_empty() {
            expanded.push('$');
        } else if let Ok(value) = std::env::var(&name) {
            expanded.push_str(&value);
        } else {
            expanded.push('$');
            expanded.push_str(&name);
        }
    }

    expanded
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}
