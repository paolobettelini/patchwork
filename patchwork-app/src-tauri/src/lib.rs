use base64::{Engine, engine::general_purpose};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};

mod assets;
mod model;
mod paths;

use assets::{
    ICON_EXTENSIONS, copy_icon_to_profile, deterministic_color_for, fake_downloads_for,
    icon_version_for, matching_icon_for_modpack_file, matching_icon_named, read_icon_data_url,
    remove_existing_icons,
};
use model::{
    AppState, DEFAULT_DESCRIPTION, DEFAULT_TERMINAL_COLS, DEFAULT_TERMINAL_ROWS,
    LauncherDependencyPage, LauncherModpack, LauncherModpackToml, LauncherSettings,
    MAX_CONSOLE_SNAPSHOT_BYTES, NewModpackToml, PATCHWORK_CONSOLE_EVENT, PatchworkConsoleChunk,
    PatchworkConsoleEvent, PatchworkTaskState, PatchworkTaskStatus, SETTINGS_FILE,
    SETTINGS_POINTER_FILE, SelectedIconFile, SettingsPointer,
};
use paths::{
    default_patchwork_data_dir, display_path, distinct_dependency_count, expand_env_vars,
    is_valid_hex_color, non_empty_or, sanitize_build_mode, sanitize_existing_modpack_id,
    slugify_modpack_id,
};

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
    read_modpacks(&settings).map_err(|error| error.to_string())
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
            "There is no running cargo process for '{profile_id}'."
        ));
    };
    if let Some(child) = task.child.as_mut() {
        child
            .kill()
            .map_err(|error| format!("Failed to stop running cargo process: {error}"))?;
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
            "There is no running cargo process for '{profile_id}'."
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
                emit_console_line(
                    &app,
                    &tasks,
                    &profile_id,
                    &action,
                    "[compose] Done.",
                );
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

    let mut command = CommandBuilder::new("cargo");
    command.arg(cargo_action);
    command.cwd(project_dir.as_os_str());
    if build_mode == "release" {
        command.arg("--release");
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("CARGO_TERM_COLOR", "always");
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
                &format!("[{cargo_action}] Failed to open PTY: {error}"),
            );
            return;
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
                &format!("[{cargo_action}] Failed to open PTY reader: {error}"),
            );
            return;
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
                &format!("[{cargo_action}] Failed to open PTY writer: {error}"),
            );
            return;
        }
    };

    let child = match pair.slave.spawn_command(command) {
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
                        task.pty_master = None;
                        task.pty_writer = None;
                        Some((status, stopped))
                    }
                    Ok(None) => None,
                    Err(error) => {
                        let line =
                            format!("[{cargo_action}] Failed while waiting for cargo: {error}");
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
                    let line = format!("[{cargo_action}] Stopped.");
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

    let _ = reader.join();
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
            Ok(len) => emit_console_bytes(
                &app,
                &tasks,
                &profile_id,
                &action,
                &buffer[..len],
            ),
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
