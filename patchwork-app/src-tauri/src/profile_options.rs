use std::{collections::BTreeMap, fs, path::Path};

use tauri::{State, command};

use crate::{
    model::{AppState, LauncherSettings, ProfileOptionsView},
    paths::{sanitize_build_mode, sanitize_existing_modpack_id},
};

#[command]
pub(crate) fn load_profile_options(
    state: State<AppState>,
    profile_id: String,
    build_mode: String,
) -> Result<ProfileOptionsView, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let build_mode = sanitize_build_mode(&build_mode)?;
    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    let options = read_profile_options(&profile_path)?;
    validate_profile_options(&options)?;

    Ok(ProfileOptionsView {
        options,
        defaults: default_profile_options(&settings, &build_mode),
    })
}

#[command]
pub(crate) fn update_profile_options(
    state: State<AppState>,
    profile_id: String,
    options: patchwork::ProfileOptions,
) -> Result<(), String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_string())?
        .clone();
    let profile_id = sanitize_existing_modpack_id(&profile_id)?;
    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    if !profile_path.is_file() {
        return Err(format!("Profile '{profile_id}' does not exist"));
    }
    validate_profile_options(&options)?;

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
    if options.is_empty() {
        table.remove("options");
    } else {
        let value = toml::Value::try_from(&options)
            .map_err(|error| format!("Failed to serialize profile options: {error}"))?;
        table.insert("options".to_owned(), value);
    }
    let source = toml::to_string_pretty(&table).map_err(|error| {
        format!(
            "Failed to serialize profile '{}': {error}",
            profile_path.display()
        )
    })?;
    fs::write(&profile_path, source).map_err(|error| {
        format!(
            "Failed to update profile '{}': {error}",
            profile_path.display()
        )
    })
}

pub(crate) fn read_profile_options(
    profile_path: &Path,
) -> Result<patchwork::ProfileOptions, String> {
    let source = fs::read_to_string(profile_path).map_err(|error| {
        format!(
            "Failed to read profile '{}': {error}",
            profile_path.display()
        )
    })?;
    let table = source.parse::<toml::Table>().map_err(|error| {
        format!(
            "Failed to parse profile '{}': {error}",
            profile_path.display()
        )
    })?;
    table
        .get("options")
        .cloned()
        .map(toml::Value::try_into)
        .transpose()
        .map_err(|error| {
            format!(
                "Failed to parse options in profile '{}': {error}",
                profile_path.display()
            )
        })
        .map(Option::unwrap_or_default)
}

fn default_profile_options(
    settings: &LauncherSettings,
    build_mode: &str,
) -> patchwork::ProfileOptions {
    let mut build_env = BTreeMap::from([
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("CARGO_TERM_COLOR".to_owned(), "always".to_owned()),
    ]);
    if !settings.cargo_target_dir.trim().is_empty() {
        build_env.insert(
            "CARGO_TARGET_DIR".to_owned(),
            settings.cargo_target_dir.clone(),
        );
    }
    let mut build_args = vec!["build".to_owned()];
    if build_mode == "release" {
        build_args.push("--release".to_owned());
    }

    patchwork::ProfileOptions {
        build: patchwork::ProcessOptions {
            args: build_args,
            env: build_env,
        },
        run: patchwork::ProcessOptions {
            args: Vec::new(),
            env: BTreeMap::from([
                ("TERM".to_owned(), "xterm-256color".to_owned()),
                ("COLORTERM".to_owned(), "truecolor".to_owned()),
                ("BACKEND_ADDR".to_owned(), settings.backend.clone()),
                ("PATCHWORK_AUTH_FD".to_owned(), "3".to_owned()),
                ("PATCHWORK_AUTH_PIPE_VERSION".to_owned(), "1".to_owned()),
            ]),
        },
    }
}

pub(crate) fn validate_profile_options(options: &patchwork::ProfileOptions) -> Result<(), String> {
    options.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_launcher_managed_environment() {
        let mut options = patchwork::ProfileOptions::default();
        options.run.env.insert(
            "backend_addr".to_owned(),
            "https://unexpected.test".to_owned(),
        );

        let error = validate_profile_options(&options).unwrap_err();

        assert!(error.contains("managed by Patchwork"));
    }

    #[test]
    fn reads_nested_options() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("example.toml");
        fs::write(
            &path,
            r#"
version = "0.1.0"
mods = []

[options.build]
args = ["--config", "/tmp/config.toml"]

[options.run.env]
GAME_LOG = "trace"
"#,
        )
        .unwrap();

        let options = read_profile_options(&path).unwrap();

        assert_eq!(options.build.args, ["--config", "/tmp/config.toml"]);
        assert_eq!(options.run.env.get("GAME_LOG"), Some(&"trace".to_owned()));
    }
}
