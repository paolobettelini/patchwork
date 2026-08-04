use base64::{Engine, engine::general_purpose};
use patchwork_registry_types::{RegistryBrowseRequest, RegistryProjectKind, RegistryProjectRef};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
#[cfg(unix)]
use std::{
    fs::OpenOptions,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
};
use tauri::{AppHandle, Emitter, Manager, State};

mod assets;
mod auth;
mod installer;
mod model;
mod paths;
mod profile_options;
mod registry;

use assets::{
    ICON_EXTENSIONS, copy_icon_to_profile, deterministic_color_for, icon_version_for,
    matching_icon_for_modpack_file, matching_icon_named, read_icon_data_url, remove_existing_icons,
};
use model::{
    AUTH_FILE, AppState, CONFIG_DIR, DEFAULT_DESCRIPTION, DEFAULT_TERMINAL_COLS,
    DEFAULT_TERMINAL_ROWS, LauncherCacheUsage, LauncherDependencyPage, LauncherInstallResult,
    LauncherModpack, LauncherModpackToml, LauncherSettings, MAX_CONSOLE_SNAPSHOT_BYTES,
    NewModpackToml, PATCHWORK_AUTH_EVENT, PATCHWORK_CONSOLE_EVENT, PatchworkAuthEvent,
    PatchworkConsoleChunk, PatchworkConsoleEvent, PatchworkTaskState, PatchworkTaskStatus,
    RegistryDownloadEvent, RegistryDownloadStatus, RegistryInstallReport, SETTINGS_FILE,
    SETTINGS_POINTER_FILE, SelectedIconFile, SettingsPointer,
};
use paths::{
    default_patchwork_data_dir, display_path, distinct_dependency_count, expand_env_vars,
    is_valid_hex_color, non_empty_or, sanitize_build_mode, sanitize_existing_modpack_id,
    slugify_modpack_id,
};
use profile_options::{read_profile_options, validate_profile_options};

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
async fn launcher_cache_usage(state: State<'_, AppState>) -> Result<LauncherCacheUsage, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || calculate_cache_usage(&settings))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn clear_launcher_cache(
    state: State<'_, AppState>,
    cache: String,
) -> Result<LauncherCacheUsage, String> {
    if state
        .tasks
        .lock()
        .map_err(|_| "Patchwork task lock is poisoned".to_string())?
        .values()
        .any(|task| task.running || task.child.is_some())
    {
        return Err("Stop the active Patchwork task before clearing caches.".to_owned());
    }

    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        match cache.as_str() {
            "cargo" => {
                for path in cargo_cache_paths()? {
                    remove_cache_path(&path, false)?;
                }
            }
            "target" => {
                remove_cache_path(Path::new(&settings.cargo_target_dir), true)?;
            }
            "build" => {
                remove_cache_path(Path::new(&settings.build_cache), true)?;
            }
            "bin" => {
                remove_cache_path(Path::new(&settings.bin_cache), true)?;
            }
            _ => return Err(format!("Unknown launcher cache '{cache}'")),
        }
        calculate_cache_usage(&settings)
    })
    .await
    .map_err(|error| error.to_string())?
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
        "bin_cache" => settings.bin_cache = value,
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
fn update_launcher_backend(
    app: AppHandle,
    state: State<AppState>,
    backend: String,
) -> Result<LauncherSettings, String> {
    let backend = auth::normalize_server_url(&backend)?;
    let updated_settings = {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| "launcher settings lock is poisoned".to_string())?;
        settings.backend = backend.clone();
        let settings_path = state
            .settings_path
            .lock()
            .map_err(|_| "launcher settings path lock is poisoned".to_string())?
            .clone();
        save_settings(&settings_path, &settings).map_err(|error| error.to_string())?;
        settings.clone()
    };

    let status = {
        let mut auth = state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        if auth.server_url != backend {
            auth.server_url = backend;
            auth.access_token = None;
            auth.profile = None;
            auth::save_auth_state(&state.auth_path, &auth).map_err(|error| error.to_string())?;
        }
        auth.status()
    };
    let _ = app.emit(
        PATCHWORK_AUTH_EVENT,
        PatchworkAuthEvent {
            status,
            error: None,
        },
    );
    Ok(updated_settings)
}

#[tauri::command]
fn update_launcher_local_folders(
    state: State<AppState>,
    folders: Vec<String>,
) -> Result<LauncherSettings, String> {
    let mut normalized = Vec::new();
    for folder in folders {
        let folder = expand_env_vars(folder.trim());
        if folder.is_empty() || normalized.contains(&folder) {
            continue;
        }
        normalized.push(folder);
    }

    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?;
    settings.local_folders = normalized;
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
    read_modpacks(&settings).map_err(|error| error.to_string())
}

#[tauri::command]
fn registry_download_status(
    state: State<'_, AppState>,
) -> Result<Option<RegistryDownloadEvent>, String> {
    state
        .download_status
        .lock()
        .map_err(|_| "download status lock is poisoned".to_owned())
        .map(|status| status.visible_event())
}

#[tauri::command]
async fn refresh_profiles(state: State<'_, AppState>) -> Result<Vec<LauncherModpack>, String> {
    if state.download_running.load(Ordering::Acquire) {
        return Err("Wait for the current download before refreshing profiles".to_owned());
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut profiles = read_modpacks(&settings).map_err(|error| error.to_string())?;
        for profile in &mut profiles {
            if let Ok((updates, _)) = installer::check_profile_updates(&settings, &profile.id) {
                profile.updates_available = updates;
            }
        }
        Ok(profiles)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn refresh_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<LauncherModpack, String> {
    if state.download_running.load(Ordering::Acquire) {
        return Err("Wait for the current download before refreshing the profile".to_owned());
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile_id = sanitize_existing_modpack_id(&profile_id)?;
        let path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
        let mut profile = read_modpack_file(&path, &settings).map_err(|error| error.to_string())?;
        let (updates, errors) = installer::check_profile_updates(&settings, &profile_id)?;
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        profile.updates_available = updates;
        Ok(profile)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn download_profile_updates(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<LauncherInstallResult, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    let download_running = state.download_running.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile_id = sanitize_existing_modpack_id(&profile_id)?;
        let report = installer::install_profile_dependencies(
            &app,
            &settings,
            &download_running,
            &profile_id,
            true,
        )?;
        let path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
        let mut profile = read_modpack_file(&path, &settings).map_err(|error| error.to_string())?;
        let (updates, _) = installer::check_profile_updates(&settings, &profile_id)?;
        profile.updates_available = updates;
        Ok(LauncherInstallResult { profile, report })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn download_profile_dependencies(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<RegistryInstallReport, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    let download_running = state.download_running.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let profile_id = sanitize_existing_modpack_id(&profile_id)?;
        installer::install_profile_dependencies(
            &app,
            &settings,
            &download_running,
            &profile_id,
            false,
        )
    })
    .await
    .map_err(|error| error.to_string())?
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

    let existing = read_modpacks(&settings).map_err(|error| error.to_string())?;
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
        version: "0.1.0".to_owned(),
        modpacks: Vec::new(),
        ignore: Vec::new(),
        mods: Vec::new(),
        options: patchwork::ProfileOptions::default(),
    };
    let toml = toml::to_string_pretty(&modpack).map_err(|error| error.to_string())?;
    fs::write(&path, toml)
        .map_err(|error| format!("Failed to create modpack '{}': {error}", path.display()))?;

    if let Some(icon_path) = icon_path.filter(|path| !path.trim().is_empty()) {
        copy_icon_to_profile(&PathBuf::from(icon_path), profiles_dir, &id)?;
    }

    read_modpack_file(&path, &settings).map_err(|error| error.to_string())
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

    read_modpack_file(&destination, &settings)
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
    registry::remove_profile_origin(&settings, &id)?;
    Ok(())
}

#[tauri::command]
async fn load_dependency_page(
    state: State<'_, AppState>,
    kind: String,
    id: String,
) -> Result<LauncherDependencyPage, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;

        let id = sanitize_existing_modpack_id(&id)?;
        let kind = kind.trim();
        if kind == "mod" && patchwork_registry_types::is_generated_mod_id(&id) {
            return Err(
                "Generated mods are created during compose and do not have project pages"
                    .to_owned(),
            );
        }
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
    })
    .await
    .map_err(|error| error.to_string())?
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
        version: "0.1.0".to_owned(),
        modpacks: Vec::new(),
        ignore: Vec::new(),
        mods: Vec::new(),
        options: patchwork::ProfileOptions::default(),
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
    include_output: Option<bool>,
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
    let include_output = include_output.unwrap_or(true);
    let tasks = state
        .tasks
        .lock()
        .map_err(|_| "patchwork task lock is poisoned".to_string())?;

    if let Some(task) = tasks.get(&profile_id) {
        Ok(PatchworkTaskStatus {
            profile_id,
            output: if !include_output {
                String::new()
            } else if task.output.is_empty() {
                "Console output will appear here.".to_string()
            } else {
                task.output.clone()
            },
            output_bytes: if include_output {
                encode_console_bytes(&task.output_bytes)
            } else {
                String::new()
            },
            running: task.running,
            action: task.action.clone(),
            runnable,
            core_error: task.core_error.clone(),
        })
    } else {
        Ok(PatchworkTaskStatus {
            profile_id,
            output: if include_output {
                "Console output will appear here.".to_string()
            } else {
                String::new()
            },
            output_bytes: String::new(),
            running: false,
            action: None,
            runnable,
            core_error: None,
        })
    }
}

#[tauri::command]
fn patchwork_console_chunk(
    state: State<AppState>,
    profile_id: String,
    offset: Option<u64>,
) -> Result<PatchworkConsoleChunk, String> {
    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let requested_offset = offset.unwrap_or(0);
    let tasks = state
        .tasks
        .lock()
        .map_err(|_| "patchwork task lock is poisoned".to_string())?;

    let Some(task) = tasks.get(&profile_id) else {
        return Ok(PatchworkConsoleChunk {
            profile_id,
            start_offset: 0,
            end_offset: 0,
            bytes: String::new(),
            reset: requested_offset != 0,
            running: false,
            action: None,
        });
    };

    let end_offset = task.output_cursor;
    let available_len = task.output_bytes.len() as u64;
    let available_start = end_offset.saturating_sub(available_len);
    let reset = requested_offset < available_start || requested_offset > end_offset;
    let start_offset = if reset {
        available_start
    } else {
        requested_offset
    };
    let slice_start = start_offset.saturating_sub(available_start) as usize;

    Ok(PatchworkConsoleChunk {
        profile_id,
        start_offset,
        end_offset,
        bytes: encode_console_bytes(&task.output_bytes[slice_start..]),
        reset,
        running: task.running,
        action: task.action.clone(),
    })
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
    if !matches!(
        action,
        "download" | "compose-build" | "compose" | "build" | "run"
    ) {
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
    let profile_options = read_profile_options(&profile_path)?;
    validate_profile_options(&profile_options)?;

    let tasks = state.tasks.clone();
    let download_running = state.download_running.clone();
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
        task.pty_master = None;
        task.pty_writer = None;
        task.stop_requested = false;
        task.core_error = None;
        task.output.clear();
        task.output_bytes.clear();
        task.output_cursor = 0;
        append_line_to_task(
            task,
            &format!(
                "Patchwork action: {} ({})",
                compose_action_title(action),
                build_mode_label(&build_mode)
            ),
        );
        append_line_to_task(task, &format!("Profile: {profile_id}"));
        let initial_chunk = encode_console_text(&format!(
            "Patchwork action: {} ({})\r\nProfile: {profile_id}\r\n",
            compose_action_title(action),
            build_mode_label(&build_mode)
        ));

        drop(tasks);

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
                chunk: Some(initial_chunk),
                running: true,
                action: Some(action.to_string()),
                runnable: Some(profile_runnable(&settings, &profile_id, &build_mode)),
                core_error: None,
            },
        );
    }

    let action = action.to_string();

    thread::spawn(move || {
        run_patchwork_task(
            app.clone(),
            tasks.clone(),
            settings.clone(),
            download_running,
            profile_id.clone(),
            profile_path,
            profile_options,
            action.clone(),
            build_mode.clone(),
        );

        if let Ok(mut tasks) = tasks.lock() {
            let task = tasks.entry(profile_id.clone()).or_default();
            task.running = false;
            task.action = None;
            task.child = None;
            task.pty_master = None;
            task.pty_writer = None;
            task.stop_requested = false;
        }
        emit_console(
            &app,
            PatchworkConsoleEvent {
                profile_id: profile_id.clone(),
                reset: false,
                line: String::new(),
                chunk: None,
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
            "There is no running Patchwork process for '{profile_id}'."
        ));
    };
    if let Some(child) = task.child.as_mut() {
        child
            .kill()
            .map_err(|error| format!("Failed to stop running Patchwork process: {error}"))?;
        task.stop_requested = true;
        append_line_to_task(task, "[run] Stop requested.");
        let chunk = encode_console_line("[run] Stop requested.");
        emit_console(
            &app,
            PatchworkConsoleEvent {
                profile_id,
                reset: false,
                line: "[run] Stop requested.".to_string(),
                chunk: Some(chunk),
                running: true,
                action: task.action.clone(),
                runnable: None,
                core_error: None,
            },
        );
        Ok(true)
    } else {
        Err(format!(
            "There is no running Patchwork process for '{profile_id}'."
        ))
    }
}

#[tauri::command]
fn resize_patchwork_terminal(
    state: State<AppState>,
    profile_id: String,
    rows: u16,
    cols: u16,
) -> Result<bool, String> {
    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let size = terminal_size(rows, cols);
    let mut tasks = state
        .tasks
        .lock()
        .map_err(|_| "patchwork task lock is poisoned".to_string())?;
    let task = tasks.entry(profile_id).or_default();
    task.terminal_size = Some(size);
    if let Some(master) = task.pty_master.as_ref() {
        master
            .resize(size)
            .map_err(|error| format!("Failed to resize terminal: {error}"))?;
    }
    Ok(true)
}

#[tauri::command]
fn write_patchwork_terminal(
    state: State<AppState>,
    profile_id: String,
    data: String,
) -> Result<bool, String> {
    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let bytes = general_purpose::STANDARD
        .decode(data)
        .map_err(|error| format!("Invalid terminal input payload: {error}"))?;
    let mut tasks = state
        .tasks
        .lock()
        .map_err(|_| "patchwork task lock is poisoned".to_string())?;
    let Some(task) = tasks.get_mut(&profile_id) else {
        return Ok(false);
    };
    let Some(writer) = task.pty_writer.as_mut() else {
        return Ok(false);
    };
    writer
        .write_all(&bytes)
        .and_then(|_| writer.flush())
        .map_err(|error| format!("Failed to write to terminal: {error}"))?;
    Ok(true)
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

    read_modpack_file(&modpack_path, &settings)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let tauri_data_dir = app.path().app_data_dir()?;
            let patchwork_data_dir = default_patchwork_data_dir(&tauri_data_dir);
            fs::create_dir_all(&patchwork_data_dir)?;
            let patchwork_config_dir = patchwork_data_dir.join(CONFIG_DIR);
            fs::create_dir_all(&patchwork_config_dir)?;
            let default_settings_path = patchwork_config_dir.join(SETTINGS_FILE);
            let settings_pointer_path = patchwork_config_dir.join(SETTINGS_POINTER_FILE);
            let settings_path =
                load_settings_pointer(&settings_pointer_path).unwrap_or(default_settings_path);
            let defaults = LauncherSettings::default_for(&patchwork_data_dir);
            let mut settings = load_settings(&settings_path, &defaults)?;
            settings.backend = auth::normalize_server_url(&settings.backend)
                .unwrap_or_else(|_| model::default_auth_server_url());
            ensure_settings_dirs(&settings)?;
            save_settings(&settings_path, &settings)?;
            save_settings_pointer(&settings_pointer_path, &settings_path)?;
            let auth_path = patchwork_config_dir.join(AUTH_FILE);
            let mut auth = auth::load_auth_state(&auth_path)?;
            if auth.server_url != settings.backend {
                auth.server_url = settings.backend.clone();
                auth.access_token = None;
                auth.profile = None;
            }
            auth::save_auth_state(&auth_path, &auth)?;
            app.manage(AppState {
                settings_pointer_path,
                settings_path: Mutex::new(settings_path),
                settings: Mutex::new(settings),
                auth_path,
                auth: Mutex::new(auth),
                tasks: Arc::new(Mutex::new(HashMap::new())),
                download_running: Arc::new(AtomicBool::new(false)),
                download_status: Mutex::new(RegistryDownloadStatus::default()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            select_folder,
            select_settings_file,
            select_icon_file,
            auth::auth_status,
            auth::start_oauth_login,
            auth::refresh_auth_profile,
            auth::start_github_connect,
            auth::disconnect_github,
            auth::logout_auth,
            auth::update_auth_nickname,
            registry::registry_create_scan,
            registry::registry_browse,
            registry::registry_project_details,
            registry::registry_add_to_profile,
            registry::registry_download_modpack_as_profile,
            registry::registry_start_scan,
            registry::registry_scan_progress,
            registry::registry_get_scan,
            registry::registry_publish_scan,
            registry::registry_rescan_mod,
            registry::registry_start_rescan,
            load_launcher_settings,
            launcher_cache_usage,
            clear_launcher_cache,
            update_launcher_path,
            update_launcher_theme,
            update_launcher_backend,
            update_launcher_local_folders,
            list_modpacks,
            registry_download_status,
            refresh_profiles,
            refresh_profile,
            download_profile_dependencies,
            download_profile_updates,
            update_profile_metadata,
            profile_options::load_profile_options,
            profile_options::update_profile_options,
            create_modpack,
            import_modpack,
            delete_modpack,
            load_dependency_page,
            toggle_profile_ignore,
            is_profile_runnable,
            patchwork_task_status,
            patchwork_console_chunk,
            start_patchwork_action,
            stop_patchwork_action,
            resize_patchwork_terminal,
            write_patchwork_terminal,
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

fn calculate_cache_usage(settings: &LauncherSettings) -> Result<LauncherCacheUsage, String> {
    let cargo_cache_bytes = cargo_cache_paths()?.iter().try_fold(0_u64, |total, path| {
        directory_size(path).map(|size| total.saturating_add(size))
    })?;
    Ok(LauncherCacheUsage {
        cargo_cache_bytes,
        target_cache_bytes: directory_size(Path::new(&settings.cargo_target_dir))?,
        build_cache_bytes: directory_size(Path::new(&settings.build_cache))?,
        bin_cache_bytes: directory_size(Path::new(&settings.bin_cache))?,
    })
}

fn cargo_cache_paths() -> Result<[PathBuf; 2], String> {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .ok_or_else(|| "Cannot determine Cargo home directory".to_owned())?;
    Ok([cargo_home.join("registry"), cargo_home.join("git")])
}

fn directory_size(path: &Path) -> Result<u64, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("Failed to inspect '{}': {error}", path.display())),
    };
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    fs::read_dir(path)
        .map_err(|error| format!("Failed to read '{}': {error}", path.display()))?
        .try_fold(0_u64, |total, entry| {
            let entry =
                entry.map_err(|error| format!("Failed to read '{}': {error}", path.display()))?;
            directory_size(&entry.path()).map(|size| total.saturating_add(size))
        })
}

fn remove_cache_path(path: &Path, recreate: bool) -> Result<(), String> {
    validate_clearable_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "Refusing to clear symlinked cache path '{}'",
                path.display()
            ));
        }
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)
            .map_err(|error| format!("Failed to clear '{}': {error}", path.display()))?,
        Ok(_) => fs::remove_file(path)
            .map_err(|error| format!("Failed to clear '{}': {error}", path.display()))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Failed to inspect '{}': {error}", path.display())),
    }
    if recreate {
        fs::create_dir_all(path)
            .map_err(|error| format!("Failed to recreate '{}': {error}", path.display()))?;
    }
    Ok(())
}

fn validate_clearable_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!(
            "Refusing to clear unsafe cache path '{}'",
            path.display()
        ));
    }
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if resolved.parent().is_none() {
        return Err("Refusing to clear a filesystem root".to_owned());
    }
    if env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| home.canonicalize().ok())
        .as_deref()
        .is_some_and(|home| home == resolved)
    {
        return Err("Refusing to clear the home directory".to_owned());
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

fn read_modpacks(settings: &LauncherSettings) -> Result<Vec<LauncherModpack>, io::Error> {
    let modpacks_dir = Path::new(&settings.profiles_dir);
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
        match read_modpack_file(&path, settings) {
            Ok(modpack) => modpacks.push(modpack),
            Err(error) => eprintln!("Skipping unreadable modpack '{}': {error}", path.display()),
        }
    }

    modpacks.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(modpacks)
}

fn read_modpack_file(
    path: &Path,
    settings: &LauncherSettings,
) -> Result<LauncherModpack, io::Error> {
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

    let dependency_count = patchwork::inspect_dependency_page(
        patchwork::DependencyTarget::Profile { id: &id },
        Path::new(&settings.mod_cache),
        Path::new(&settings.modpacks_cache),
        Path::new(&settings.profiles_dir),
    )
    .map(|page| page.distinct_dependency_count)
    .unwrap_or_else(|_| distinct_dependency_count(&parsed.modpacks, &parsed.mods));
    let downloads = profile_downloads(settings, &id)
        .map(format_download_count)
        .unwrap_or_else(|| "-".to_owned());

    Ok(LauncherModpack {
        id: id.clone(),
        name: parsed.name.unwrap_or_else(|| id.clone()),
        description: parsed
            .description
            .unwrap_or_else(|| "No description provided yet.".to_string()),
        version: parsed.version.unwrap_or_else(|| "1.21.x".to_string()),
        mods: parsed.mods.len(),
        dependencies: dependency_count,
        downloads,
        accent: parsed
            .color
            .filter(|color| is_valid_hex_color(color))
            .unwrap_or_else(|| deterministic_color_for(&id).to_string()),
        icon_data_url,
        icon_version,
        updates_available: 0,
    })
}

fn profile_downloads(settings: &LauncherSettings, id: &str) -> Option<i64> {
    let origin = registry::load_profile_origin(settings, id)?;
    if origin.source == patchwork_registry_types::RegistryBrowseSource::Local {
        return None;
    }
    registry::fetch_project_details(
        &settings.backend,
        RegistryProjectRef {
            project_kind: RegistryProjectKind::Modpack,
            project_id: id.to_owned(),
        },
    )
    .ok()
    .and_then(|details| details.downloads)
    .or(Some(origin.downloads))
}

fn format_download_count(downloads: i64) -> String {
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}

fn decorate_dependency_page(
    page: patchwork::DependencyPage,
    settings: &LauncherSettings,
) -> Result<LauncherDependencyPage, String> {
    let (source_kind, registry_details) = dependency_page_source(&page, settings);
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
        version: page.version,
        editable_profile: page.editable_profile,
        distinct_dependency_count: page.distinct_dependency_count,
        modpacks: page.modpacks,
        mods: page.mods,
        diagnostics: page.diagnostics,
        icon_data_url,
        icon_version,
        publisher_name: registry_details
            .as_ref()
            .map(|details| details.publisher_name.clone()),
        published_at: registry_details
            .as_ref()
            .map(|details| details.published_at.clone()),
        downloads: registry_details
            .as_ref()
            .and_then(|details| details.downloads),
        repository_url: registry_details
            .as_ref()
            .map(|details| details.repository_url.clone()),
        repository_path: registry_details
            .as_ref()
            .map(|details| details.repository_path.clone()),
        source_commit: registry_details
            .as_ref()
            .map(|details| details.source_commit.clone()),
        source_tree_oid: registry_details
            .as_ref()
            .map(|details| details.source_tree_oid.clone()),
        manifest_sha256: registry_details
            .as_ref()
            .map(|details| details.manifest_sha256.clone()),
        source_kind,
    })
}

fn dependency_page_source(
    page: &patchwork::DependencyPage,
    settings: &LauncherSettings,
) -> (
    String,
    Option<patchwork_registry_types::RegistryProjectDetails>,
) {
    let project_kind = match page.kind {
        patchwork::DependencyPageKind::Profile => RegistryProjectKind::Modpack,
        patchwork::DependencyPageKind::Modpack => RegistryProjectKind::Modpack,
        patchwork::DependencyPageKind::Mod => RegistryProjectKind::Mod,
    };
    let request = RegistryBrowseRequest {
        query: page.id.clone(),
        include_mods: project_kind == RegistryProjectKind::Mod,
        include_modpacks: project_kind == RegistryProjectKind::Modpack,
    };
    let stored_origin = matches!(page.kind, patchwork::DependencyPageKind::Profile)
        .then(|| registry::load_profile_origin(settings, &page.id))
        .flatten();
    if matches!(page.kind, patchwork::DependencyPageKind::Profile) {
        match stored_origin {
            Some(origin)
                if origin.source == patchwork_registry_types::RegistryBrowseSource::Local =>
            {
                return ("local-registry".to_owned(), None);
            }
            Some(_) => {
                let details = registry::fetch_project_details(
                    &settings.backend,
                    RegistryProjectRef {
                        project_kind,
                        project_id: page.id.clone(),
                    },
                )
                .ok();
                return ("remote-registry".to_owned(), details);
            }
            None => return ("profile".to_owned(), None),
        }
    }

    let is_local = settings.local_folders.iter().any(|folder| {
        registry::browse_local_folder(Path::new(folder), &request).is_ok_and(|projects| {
            projects.into_iter().any(|project| {
                project.project_kind == project_kind && project.project_id == page.id
            })
        })
    });
    if is_local {
        return ("local-registry".to_owned(), None);
    }

    let details = registry::fetch_project_details(
        &settings.backend,
        RegistryProjectRef {
            project_kind,
            project_id: page.id.clone(),
        },
    )
    .ok();
    if details.is_some() {
        ("remote-registry".to_owned(), details)
    } else {
        ("remote-registry".to_owned(), None)
    }
}

fn run_patchwork_task(
    app: AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    settings: LauncherSettings,
    download_running: Arc<AtomicBool>,
    profile_id: String,
    profile_path: PathBuf,
    profile_options: patchwork::ProfileOptions,
    action: String,
    build_mode: String,
) {
    if action != "run" {
        emit_console_line(
            &app,
            &tasks,
            &profile_id,
            &action,
            "[download] Resolving profile dependencies...",
        );
        match installer::install_profile_dependencies(
            &app,
            &settings,
            &download_running,
            &profile_id,
            false,
        ) {
            Ok(report) if report.errors.is_empty() => emit_console_line(
                &app,
                &tasks,
                &profile_id,
                &action,
                &format!(
                    "[download] Done: {} installed, {} already available.",
                    report.installed, report.up_to_date
                ),
            ),
            Ok(report) => {
                let error = format!("Dependency download failed:\n{}", report.errors.join("\n"));
                emit_console_line_with_error(
                    &app,
                    &tasks,
                    &profile_id,
                    &action,
                    &format!("[download] Failed: {error}"),
                    Some(error),
                );
                return;
            }
            Err(error) => {
                emit_console_line_with_error(
                    &app,
                    &tasks,
                    &profile_id,
                    &action,
                    &format!("[download] Failed: {error}"),
                    Some(error),
                );
                return;
            }
        }
    }

    if matches!(action.as_str(), "compose-build" | "compose" | "build") {
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
                emit_console_line(&app, &tasks, &profile_id, &action, "[compose] Done.");
            }
            Err(error) => {
                let error = error.to_string();
                emit_console_line_with_error(
                    &app,
                    &tasks,
                    &profile_id,
                    &action,
                    &format!("[compose] Failed: {error}"),
                    Some(error),
                );
                return;
            }
        }
    }

    if matches!(action.as_str(), "compose-build" | "build") {
        let project_dir = Path::new(&settings.build_cache).join(&profile_id);
        if run_cargo_build(
            &app,
            tasks.clone(),
            &profile_id,
            &action,
            &project_dir,
            &build_mode,
            &settings,
            &profile_options.build,
        ) {
            match archive_built_executable(&settings, &profile_id, &build_mode) {
                Ok(path) => emit_console_line(
                    &app,
                    &tasks,
                    &profile_id,
                    &action,
                    &format!("[build] Executable stored at '{}'.", path.display()),
                ),
                Err(error) => emit_console_line_with_error(
                    &app,
                    &tasks,
                    &profile_id,
                    &action,
                    &format!("[build] Failed to store executable: {error}"),
                    Some(error),
                ),
            }
        }
    } else if action == "run" {
        run_profile_executable(
            &app,
            tasks,
            &profile_id,
            &action,
            &settings,
            &build_mode,
            &profile_options.run,
        );
    }
}

fn run_cargo_build(
    app: &AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    project_dir: &Path,
    build_mode: &str,
    settings: &LauncherSettings,
    options: &patchwork::ProcessOptions,
) -> bool {
    if !project_dir.join("Cargo.toml").is_file() {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!(
                "[build] Failed: composed project not found at '{}'. Run Compose first.",
                project_dir.display()
            ),
        );
        return false;
    }

    let cargo_executable = cargo_executable_path(settings, profile_id, build_mode);
    if fs::symlink_metadata(&cargo_executable)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
        && !cargo_executable.is_file()
        && let Err(error) = fs::remove_file(&cargo_executable)
    {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!(
                "[build] Failed to remove broken executable symlink '{}': {error}",
                cargo_executable.display()
            ),
        );
        return false;
    }

    let mut command = CommandBuilder::new("cargo");
    command.arg("build");
    command.cwd(project_dir.as_os_str());
    if build_mode == "release" {
        command.arg("--release");
    }
    let custom_arguments = match options.expanded_args() {
        Ok(arguments) => arguments,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[build] Failed to parse custom arguments: {error}"),
            );
            return false;
        }
    };
    for argument in &custom_arguments {
        command.arg(argument);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("CARGO_TERM_COLOR", "always");
    if !settings.cargo_target_dir.trim().is_empty() {
        command.env("CARGO_TARGET_DIR", &settings.cargo_target_dir);
    }
    for (name, value) in &options.env {
        command.env(name, value);
    }

    emit_console_line(
        app,
        &tasks,
        profile_id,
        action,
        &format!(
            "[build] Command: cargo build{}{}",
            if build_mode == "release" {
                " --release"
            } else {
                ""
            },
            format_command_arguments(&custom_arguments),
        ),
    );
    if !settings.cargo_target_dir.trim().is_empty() {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!("[build] CARGO_TARGET_DIR={}", settings.cargo_target_dir),
        );
    }

    run_pty_process(
        app,
        tasks,
        profile_id,
        action,
        "build",
        ProcessLaunch::Command(command),
    )
}

fn format_command_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| format!(" {argument:?}"))
        .collect()
}

fn run_profile_executable(
    app: &AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    settings: &LauncherSettings,
    build_mode: &str,
    options: &patchwork::ProcessOptions,
) -> bool {
    let executable = executable_path(settings, profile_id, build_mode);
    if !executable.is_file() {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!(
                "[run] Failed: executable not found at '{}'. Run Build first.",
                executable.display()
            ),
        );
        return false;
    }

    if let Err(error) = sync_cached_assets_link(settings, profile_id, build_mode) {
        emit_console_line(
            app,
            &tasks,
            profile_id,
            action,
            &format!("[run] Failed to prepare assets: {error}"),
        );
        return false;
    }

    let custom_arguments = match options.expanded_args() {
        Ok(arguments) => arguments,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[run] Failed to parse custom arguments: {error}"),
            );
            return false;
        }
    };
    let mut command = CommandBuilder::new(executable.as_os_str());
    for argument in &custom_arguments {
        command.arg(argument);
    }
    let working_dir = executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    command.cwd(working_dir.as_os_str());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("BACKEND_ADDR", &settings.backend);
    for (name, value) in &options.env {
        command.env(name, value);
    }
    let launch_ticket = match request_game_launch_ticket(app) {
        Ok(ticket) => ticket,
        Err(error) => {
            emit_console_line_with_error(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[auth] Failed: {error}"),
                Some(error),
            );
            return false;
        }
    };
    emit_console_line(
        app,
        &tasks,
        profile_id,
        action,
        &format!(
            "[run] Command: {}{}",
            executable.display(),
            format_command_arguments(&custom_arguments)
        ),
    );
    let launch = if let Some(ticket) = launch_ticket {
        #[cfg(unix)]
        {
            ProcessLaunch::AuthenticatedGame(AuthenticatedGameLaunch {
                executable,
                working_dir,
                backend: settings.backend.clone(),
                args: custom_arguments,
                env: options.env.clone(),
                ticket,
            })
        }
        #[cfg(not(unix))]
        {
            emit_console_line_with_error(
                app,
                &tasks,
                profile_id,
                action,
                "[auth] Failed: authenticated game launch is not supported on this platform.",
                Some("authenticated game launch is not supported on this platform".to_owned()),
            );
            return false;
        }
    } else {
        ProcessLaunch::Command(command)
    };
    run_pty_process(app, tasks, profile_id, action, "run", launch)
}

enum ProcessLaunch {
    Command(CommandBuilder),
    #[cfg(unix)]
    AuthenticatedGame(AuthenticatedGameLaunch),
}

#[cfg(unix)]
struct AuthenticatedGameLaunch {
    executable: PathBuf,
    working_dir: PathBuf,
    backend: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    ticket: String,
}

#[derive(serde::Deserialize)]
struct GameLaunchTicketResponse {
    launch_ticket: String,
}

fn request_game_launch_ticket(app: &AppHandle) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let (server_url, access_token) = {
        let auth = state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_owned())?;
        (auth.server_url.clone(), auth.access_token.clone())
    };
    let Some(access_token) = access_token else {
        return Ok(None);
    };

    let url = auth::endpoint_url(&server_url, "/game/launch-ticket")?;
    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(game_auth_http_error)?
        .into_json::<GameLaunchTicketResponse>()
        .map_err(|error| format!("invalid launch ticket response: {error}"))?;
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(&response.launch_ticket)
        .map_err(|_| "backend returned a malformed launch ticket".to_owned())?;
    if decoded.len() != 32 {
        return Err("backend returned a launch ticket with an invalid length".to_owned());
    }
    Ok(Some(response.launch_ticket))
}

fn game_auth_http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, response) => response
            .into_json::<serde_json::Value>()
            .ok()
            .and_then(|body| {
                body.get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("backend rejected launch ticket request ({status})")),
        ureq::Error::Transport(error) => format!("could not reach Patchwork backend: {error}"),
    }
}

#[cfg(unix)]
fn spawn_authenticated_game_process(
    pair: &portable_pty::PtyPair,
    launch: AuthenticatedGameLaunch,
) -> Result<Box<dyn portable_pty::Child + Send + Sync>, String> {
    let terminal_path = pair
        .master
        .tty_name()
        .ok_or_else(|| "PTY does not expose its slave terminal".to_owned())?;
    let terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&terminal_path)
        .map_err(|error| {
            format!(
                "failed to open PTY slave '{}': {error}",
                terminal_path.display()
            )
        })?;
    let terminal_stdin = terminal
        .try_clone()
        .map_err(|error| format!("failed to clone PTY stdin: {error}"))?;
    let terminal_stdout = terminal
        .try_clone()
        .map_err(|error| format!("failed to clone PTY stdout: {error}"))?;
    let (ticket_reader, mut ticket_writer) =
        os_pipe::pipe().map_err(|error| format!("failed to create auth pipe: {error}"))?;
    let ticket_reader_fd = ticket_reader.as_raw_fd();

    let mut command = Command::new(&launch.executable);
    command
        .current_dir(&launch.working_dir)
        .args(&launch.args)
        .stdin(Stdio::from(terminal_stdin))
        .stdout(Stdio::from(terminal_stdout))
        .stderr(Stdio::from(terminal))
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("BACKEND_ADDR", &launch.backend)
        .env("PATCHWORK_AUTH_FD", "3")
        .env("PATCHWORK_AUTH_PIPE_VERSION", "1")
        .envs(&launch.env);

    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            if ticket_reader_fd != 3 && libc::dup2(ticket_reader_fd, 3) == -1 {
                return Err(io::Error::last_os_error());
            }
            let flags = libc::fcntl(3, libc::F_GETFD);
            if flags == -1 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start authenticated game process: {error}"))?;
    drop(ticket_reader);
    let ticket = launch.ticket.as_bytes();
    let length = u32::try_from(ticket.len())
        .map_err(|_| "launch ticket is too large for the auth pipe".to_owned())?;
    if let Err(error) = ticket_writer
        .write_all(&length.to_be_bytes())
        .and_then(|()| ticket_writer.write_all(ticket))
        .and_then(|()| ticket_writer.flush())
    {
        let _ = child.kill();
        return Err(format!(
            "failed to write launch ticket to auth pipe: {error}"
        ));
    }
    drop(ticket_writer);
    Ok(Box::new(child))
}

fn run_pty_process(
    app: &AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    process_label: &str,
    launch: ProcessLaunch,
) -> bool {
    let initial_size = tasks
        .lock()
        .ok()
        .and_then(|tasks| tasks.get(profile_id).and_then(|task| task.terminal_size))
        .unwrap_or_else(default_terminal_size);

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(initial_size) {
        Ok(pair) => pair,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[{process_label}] Failed to open PTY: {error}"),
            );
            return false;
        }
    };

    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[{process_label}] Failed to open PTY reader: {error}"),
            );
            return false;
        }
    };

    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[{process_label}] Failed to open PTY writer: {error}"),
            );
            return false;
        }
    };

    let child = match launch {
        ProcessLaunch::Command(command) => pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string()),
        #[cfg(unix)]
        ProcessLaunch::AuthenticatedGame(launch) => spawn_authenticated_game_process(&pair, launch),
    };
    let child = match child {
        Ok(child) => child,
        Err(error) => {
            emit_console_line(
                app,
                &tasks,
                profile_id,
                action,
                &format!("[{process_label}] Failed to start process: {error}"),
            );
            return false;
        }
    };
    drop(pair.slave);

    if let Ok(mut tasks) = tasks.lock() {
        let task = tasks.entry(profile_id.to_string()).or_default();
        task.child = Some(child);
        task.pty_master = Some(pair.master);
        task.pty_writer = Some(writer);
    }

    let reader = {
        let app = app.clone();
        let tasks = tasks.clone();
        let profile_id = profile_id.to_string();
        let action = action.to_string();
        thread::spawn(move || stream_pty_output(app, tasks, profile_id, action, reader))
    };

    let mut succeeded = false;
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
                        &format!("[{process_label}] Failed: task lock is poisoned."),
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
                        task.pty_master = None;
                        task.pty_writer = None;
                        Some((status, stopped))
                    }
                    Ok(None) => None,
                    Err(error) => {
                        let line =
                            format!("[{process_label}] Failed while waiting for process: {error}");
                        append_line_to_task(task, &line);
                        let chunk = encode_console_line(&line);
                        emit_console(
                            app,
                            PatchworkConsoleEvent {
                                profile_id: profile_id.to_string(),
                                reset: false,
                                line,
                                chunk: Some(chunk),
                                running: true,
                                action: Some(action.to_string()),
                                runnable: None,
                                core_error: None,
                            },
                        );
                        task.child = None;
                        task.pty_master = None;
                        task.pty_writer = None;
                        break;
                    }
                },
                None => {
                    let line = format!("[{process_label}] Stopped.");
                    append_line_to_task(task, &line);
                    let chunk = encode_console_line(&line);
                    emit_console(
                        app,
                        PatchworkConsoleEvent {
                            profile_id: profile_id.to_string(),
                            reset: false,
                            line,
                            chunk: Some(chunk),
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
                    &format!("[{process_label}] Stopped."),
                );
            } else if status.success() {
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{process_label}] Status: {status}"),
                );
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{process_label}] Done."),
                );
                succeeded = true;
            } else {
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{process_label}] Status: {status}"),
                );
                emit_console_line(
                    app,
                    &tasks,
                    profile_id,
                    action,
                    &format!("[{process_label}] Failed."),
                );
            }
            break;
        }

        thread::sleep(Duration::from_millis(120));
    }

    let _ = reader.join();
    succeeded
}

fn stream_pty_output<R>(
    app: AppHandle,
    tasks: Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: String,
    action: String,
    mut output: R,
) where
    R: Read,
{
    let mut buffer = [0_u8; 8192];
    loop {
        match output.read(&mut buffer) {
            Ok(0) => break,
            Ok(len) => emit_console_bytes(&app, &tasks, &profile_id, &action, &buffer[..len]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn emit_console_line(
    app: &AppHandle,
    tasks: &Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    line: &str,
) {
    emit_console_line_with_error(app, tasks, profile_id, action, line, None);
}

fn emit_console_line_with_error(
    app: &AppHandle,
    tasks: &Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    line: &str,
    core_error: Option<String>,
) {
    append_task_line(tasks, profile_id, line, core_error.clone());
    emit_console(
        app,
        PatchworkConsoleEvent {
            profile_id: profile_id.to_string(),
            reset: false,
            line: line.to_string(),
            chunk: Some(encode_console_line(line)),
            running: true,
            action: Some(action.to_string()),
            runnable: None,
            core_error,
        },
    );
}

fn emit_console_bytes(
    app: &AppHandle,
    tasks: &Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    action: &str,
    bytes: &[u8],
) {
    append_task_bytes(tasks, profile_id, bytes);
    emit_console(
        app,
        PatchworkConsoleEvent {
            profile_id: profile_id.to_string(),
            reset: false,
            line: String::new(),
            chunk: Some(encode_console_bytes(bytes)),
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

fn append_task_bytes(
    tasks: &Arc<Mutex<HashMap<String, PatchworkTaskState>>>,
    profile_id: &str,
    bytes: &[u8],
) {
    if let Ok(mut tasks) = tasks.lock() {
        let task = tasks.entry(profile_id.to_string()).or_default();
        append_bytes_to_task(task, bytes);
    }
}

fn append_line_to_task(task: &mut PatchworkTaskState, line: &str) {
    if !task.output.is_empty() && !task.output.ends_with('\n') {
        task.output.push('\n');
    }
    task.output.push_str(line);
    task.output.push('\n');
    append_output_bytes_to_task(task, encode_console_terminal_line(line).as_bytes());
    trim_console_snapshot(task);
}

fn append_bytes_to_task(task: &mut PatchworkTaskState, bytes: &[u8]) {
    append_output_bytes_to_task(task, bytes);
    task.output.push_str(&String::from_utf8_lossy(bytes));
    trim_console_snapshot(task);
}

fn append_output_bytes_to_task(task: &mut PatchworkTaskState, bytes: &[u8]) {
    task.output_cursor = task.output_cursor.saturating_add(bytes.len() as u64);
    task.output_bytes.extend_from_slice(bytes);
}

fn trim_console_snapshot(task: &mut PatchworkTaskState) {
    if task.output_bytes.len() > MAX_CONSOLE_SNAPSHOT_BYTES {
        let excess = task.output_bytes.len() - MAX_CONSOLE_SNAPSHOT_BYTES;
        task.output_bytes.drain(..excess);
    }
    if task.output.len() > MAX_CONSOLE_SNAPSHOT_BYTES {
        let excess = task.output.len() - MAX_CONSOLE_SNAPSHOT_BYTES;
        task.output.drain(..excess);
    }
}

fn encode_console_line(line: &str) -> String {
    encode_console_text(&encode_console_terminal_line(line))
}

fn encode_console_terminal_line(line: &str) -> String {
    format!("{line}\r\n")
}

fn encode_console_text(text: &str) -> String {
    encode_console_bytes(text.as_bytes())
}

fn encode_console_bytes(bytes: &[u8]) -> String {
    general_purpose::STANDARD.encode(bytes)
}

fn default_terminal_size() -> PtySize {
    terminal_size(DEFAULT_TERMINAL_ROWS, DEFAULT_TERMINAL_COLS)
}

fn terminal_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.clamp(1, 2000),
        cols: cols.clamp(2, 1000),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn profile_runnable(settings: &LauncherSettings, profile_id: &str, build_mode: &str) -> bool {
    executable_path(settings, profile_id, build_mode).is_file()
}

fn executable_path(settings: &LauncherSettings, profile_id: &str, build_mode: &str) -> PathBuf {
    let project_dir = Path::new(&settings.build_cache).join(profile_id);
    let package_name =
        composed_package_name(&project_dir).unwrap_or_else(|| profile_id.to_string());
    let profile_dir = if build_mode == "debug" {
        "debug"
    } else {
        "release"
    };
    let mut executable = Path::new(&settings.bin_cache)
        .join(profile_id)
        .join(profile_dir)
        .join(package_name);
    if cfg!(windows) {
        executable.set_extension("exe");
    }
    executable
}

fn cargo_executable_path(
    settings: &LauncherSettings,
    profile_id: &str,
    build_mode: &str,
) -> PathBuf {
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

fn archive_built_executable(
    settings: &LauncherSettings,
    profile_id: &str,
    build_mode: &str,
) -> Result<PathBuf, String> {
    let source = cargo_executable_path(settings, profile_id, build_mode);
    let destination = executable_path(settings, profile_id, build_mode);

    if fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        if source.canonicalize().ok() == destination.canonicalize().ok() && destination.is_file() {
            sync_cached_assets_link(settings, profile_id, build_mode)?;
            return Ok(destination);
        }
        fs::remove_file(&source).map_err(|error| {
            format!(
                "Failed to remove stale executable symlink '{}': {error}",
                source.display()
            )
        })?;
    }
    if !source.is_file() {
        return Err(format!(
            "Cargo did not produce the expected executable at '{}'",
            source.display()
        ));
    }

    let destination_parent = destination
        .parent()
        .ok_or_else(|| "Binary cache destination has no parent directory".to_owned())?;
    fs::create_dir_all(destination_parent).map_err(|error| {
        format!(
            "Failed to create binary cache '{}': {error}",
            destination_parent.display()
        )
    })?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("executable");
    let temporary = destination_parent.join(format!(".{file_name}.patchwork-new"));
    if fs::symlink_metadata(&temporary).is_ok() {
        fs::remove_file(&temporary).map_err(|error| {
            format!(
                "Failed to remove temporary executable '{}': {error}",
                temporary.display()
            )
        })?;
    }
    fs::copy(&source, &temporary).map_err(|error| {
        format!(
            "Failed to copy executable '{}' to binary cache: {error}",
            source.display()
        )
    })?;
    if fs::symlink_metadata(&destination).is_ok() {
        fs::remove_file(&destination).map_err(|error| {
            format!(
                "Failed to replace cached executable '{}': {error}",
                destination.display()
            )
        })?;
    }
    fs::rename(&temporary, &destination).map_err(|error| {
        format!(
            "Failed to publish cached executable '{}': {error}",
            destination.display()
        )
    })?;
    fs::remove_file(&source).map_err(|error| {
        format!(
            "Failed to remove Cargo executable '{}': {error}",
            source.display()
        )
    })?;
    create_executable_symlink(&destination, &source)?;
    sync_cached_assets_link(settings, profile_id, build_mode)?;
    Ok(destination)
}

fn sync_cached_assets_link(
    settings: &LauncherSettings,
    profile_id: &str,
    build_mode: &str,
) -> Result<(), String> {
    let source = Path::new(&settings.build_cache)
        .join(profile_id)
        .join("assets");
    let executable = executable_path(settings, profile_id, build_mode);
    let parent = executable
        .parent()
        .ok_or_else(|| "Binary cache executable has no parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed to create binary cache directory '{}': {error}",
            parent.display()
        )
    })?;
    let link = parent.join("assets");

    match fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if source.is_dir() && link.canonicalize().ok() == source.canonicalize().ok() {
                return Ok(());
            }
            fs::remove_file(&link).map_err(|error| {
                format!(
                    "Failed to remove stale assets symlink '{}': {error}",
                    link.display()
                )
            })?;
        }
        Ok(_) => {
            return Err(format!(
                "Refusing to replace non-symlink assets path '{}'",
                link.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Failed to inspect assets path '{}': {error}",
                link.display()
            ));
        }
    }

    if !source.is_dir() {
        return Ok(());
    }
    let source = source.canonicalize().map_err(|error| {
        format!(
            "Failed to canonicalize composed assets '{}': {error}",
            source.display()
        )
    })?;
    create_directory_symlink(&source, &link)
}

#[cfg(unix)]
fn create_executable_symlink(destination: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(destination, link).map_err(|error| {
        format!(
            "Failed to create executable symlink '{}' -> '{}': {error}",
            link.display(),
            destination.display()
        )
    })
}

#[cfg(unix)]
fn create_directory_symlink(destination: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(destination, link).map_err(|error| {
        format!(
            "Failed to create assets symlink '{}' -> '{}': {error}",
            link.display(),
            destination.display()
        )
    })
}

#[cfg(windows)]
fn create_directory_symlink(destination: &Path, link: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(destination, link).map_err(|error| {
        format!(
            "Failed to create assets symlink '{}' -> '{}': {error}",
            link.display(),
            destination.display()
        )
    })
}

#[cfg(windows)]
fn create_executable_symlink(destination: &Path, link: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_file(destination, link).map_err(|error| {
        format!(
            "Failed to create executable symlink '{}' -> '{}': {error}",
            link.display(),
            destination.display()
        )
    })
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
        "download" => "Download",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn archives_built_executable_and_leaves_target_symlink() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let mut settings = LauncherSettings::default_for(root);
        settings.cargo_target_dir = display_path(&root.join("target"));
        settings.build_cache = display_path(&root.join("cache/build"));
        settings.bin_cache = display_path(&root.join("cache/bin"));

        let project_dir = Path::new(&settings.build_cache).join("example-profile");
        fs::create_dir_all(&project_dir).unwrap();
        fs::create_dir_all(project_dir.join("assets")).unwrap();
        fs::write(project_dir.join("assets/example.txt"), b"asset").unwrap();
        fs::write(
            project_dir.join("Cargo.toml"),
            "[package]\nname = \"example-game\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let cargo_executable = cargo_executable_path(&settings, "example-profile", "release");
        fs::create_dir_all(cargo_executable.parent().unwrap()).unwrap();
        fs::write(&cargo_executable, b"executable").unwrap();

        let cached = archive_built_executable(&settings, "example-profile", "release").unwrap();

        assert_eq!(fs::read(&cached).unwrap(), b"executable");
        assert!(
            fs::symlink_metadata(&cargo_executable)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            cargo_executable.canonicalize().unwrap(),
            cached.canonicalize().unwrap()
        );
        let assets = cached.parent().unwrap().join("assets");
        assert!(
            fs::symlink_metadata(&assets)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(assets.join("example.txt")).unwrap(), b"asset");
    }
}
