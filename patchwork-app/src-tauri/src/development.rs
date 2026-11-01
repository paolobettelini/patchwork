use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::State;

use crate::{
    model::{AppState, LauncherSettings},
    paths::sanitize_existing_modpack_id,
};

#[tauri::command]
pub(crate) async fn select_development_modpack_file() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("Patchwork modpack", &["toml"])
            .pick_file()
            .map(|path| path.display().to_string())
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn discover_development_checkout(
    state: State<'_, AppState>,
    profile_id: String,
    path: String,
) -> Result<patchwork::DevelopmentConfig, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let id = sanitize_existing_modpack_id(&profile_id)?;
        let root = Path::new(&path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !Path::new(&settings.profiles_dir)
            .join(format!("{id}.toml"))
            .is_file()
        {
            return Err("Profile does not exist".into());
        }
        if !root.is_dir() {
            return Err("Checkout must be a directory".into());
        }
        let mut git = std::process::Command::new("git");
        git.arg("-C")
            .arg(&root)
            .args(["config", "--get", "remote.origin.url"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            git.creation_flags(0x08000000);
        }
        let source = git
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        let config = patchwork::DevelopmentConfig {
            enabled: true,
            checkouts: vec![patchwork::DevelopmentCheckout {
                path: root,
                git: source,
            }],
            ..Default::default()
        };
        Ok(config)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn config_path(
    settings: &LauncherSettings,
    profile_id: &str,
) -> Result<PathBuf, String> {
    let id = sanitize_existing_modpack_id(profile_id)?;
    Ok(Path::new(&settings.profiles_dir)
        .join(".patchwork")
        .join(format!("{id}.development.toml")))
}

pub(crate) fn read_config(
    settings: &LauncherSettings,
    profile_id: &str,
) -> Result<patchwork::DevelopmentConfig, String> {
    let path = config_path(settings, profile_id)?;
    match fs::read_to_string(&path) {
        Ok(source) => toml::from_str(&source).map_err(|error| {
            format!(
                "Invalid development configuration '{}': {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn resolved_config(
    settings: &LauncherSettings,
    profile_id: &str,
) -> Result<patchwork::DevelopmentConfig, String> {
    let config = read_config(settings, profile_id)?;
    if !config.enabled {
        return Ok(Default::default());
    }
    let path = config_path(settings, profile_id)?;
    config
        .resolved(path.parent().expect("profile sidecar parent"))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn load_profile_development(
    state: State<AppState>,
    profile_id: String,
) -> Result<patchwork::DevelopmentConfig, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned")?
        .clone();
    read_config(&settings, &profile_id)
}

#[tauri::command]
pub(crate) fn update_profile_development(
    state: State<AppState>,
    profile_id: String,
    config: patchwork::DevelopmentConfig,
) -> Result<(), String> {
    let tasks = state.tasks.lock().map_err(|_| "task lock poisoned")?;
    if tasks.get(&profile_id).is_some_and(|task| task.running)
        || state
            .download_running
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(
            "Wait for the active build/download before changing development sources".into(),
        );
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned")?
        .clone();
    let path = config_path(&settings, &profile_id)?;
    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    let source = fs::read_to_string(&profile_path).map_err(|error| error.to_string())?;
    let mut profile: toml::Table = toml::from_str(&source).map_err(|error| error.to_string())?;
    let resolved = if config.enabled {
        config
            .resolved(path.parent().expect("sidecar parent"))
            .map_err(|error| error.to_string())?
    } else {
        Default::default()
    };
    let old = read_config(&settings, &profile_id)?;
    // Linking a new mod also selects it. Removing an override keeps the selection,
    // so a published version can be used instead without changing the modpack.
    if config.enabled {
        let existing = patchwork::inspect_profile_dependencies_with_sources(
            &profile_path,
            Path::new(&settings.mod_cache),
            Path::new(&settings.modpacks_cache),
            &resolved.mods,
            &resolved.modpacks,
        )
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|entry| entry.kind == "mod")
        .map(|entry| entry.id)
        .collect::<std::collections::HashSet<_>>();
        let mods = profile
            .entry("mods")
            .or_insert_with(|| toml::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("profile mods must be an array")?;
        for id in config
            .mods
            .keys()
            .filter(|id| !old.mods.contains_key(*id) || !old.enabled)
        {
            if !existing.contains(id) && !mods.iter().any(|value| value.as_str() == Some(id)) {
                mods.push(toml::Value::String(id.clone()));
            }
        }
    }
    fs::create_dir_all(path.parent().expect("sidecar parent"))
        .map_err(|error| error.to_string())?;
    let config_source = toml::to_string_pretty(&config).map_err(|error| error.to_string())?;
    let profile_source = toml::to_string_pretty(&profile).map_err(|error| error.to_string())?;
    fs::write(&path, config_source).map_err(|error| error.to_string())?;
    fs::write(&profile_path, profile_source).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn load_profile_dependencies(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<Vec<patchwork::ProfileDependency>, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let id = sanitize_existing_modpack_id(&profile_id)?;
        let config = resolved_config(&settings, &id)?;
        let mut entries = patchwork::inspect_profile_dependencies_with_sources(
            &Path::new(&settings.profiles_dir).join(format!("{id}.toml")),
            Path::new(&settings.mod_cache),
            Path::new(&settings.modpacks_cache),
            &config.mods,
            &config.modpacks,
        )
        .map_err(|error| error.to_string())?;
        for entry in &mut entries {
            let icon = if entry.kind == "mod" {
                let directory = config
                    .mods
                    .get(&entry.id)
                    .cloned()
                    .unwrap_or_else(|| Path::new(&settings.mod_cache).join(&entry.id));
                crate::assets::matching_icon_named(&directory, &entry.id)
                    .ok()
                    .flatten()
            } else {
                let path = config.modpacks.get(&entry.id).cloned().unwrap_or_else(|| {
                    Path::new(&settings.modpacks_cache).join(format!("{}.toml", entry.id))
                });
                crate::assets::matching_icon_for_modpack_file(&path)
                    .ok()
                    .flatten()
            };
            entry.icon_data_url =
                icon.and_then(|path| crate::assets::read_icon_data_url(&path).ok());
        }
        Ok(entries)
    })
    .await
    .map_err(|error| error.to_string())?
}
