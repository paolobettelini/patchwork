use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use patchwork::{
    RegistryWorkspaceManifest, parse_registry_mod_manifest, parse_registry_modpack_manifest,
};
use patchwork_registry_types::{
    RegistryAddToProfileRequest, RegistryBrowseProject, RegistryBrowseRequest,
    RegistryBrowseResponse, RegistryBrowseSource, RegistryProjectDetails, RegistryProjectKind,
    RegistryProjectRef, RegistryPublishRequest, RegistryPublishResponse, RegistryScan,
    RegistryScanJobStarted, RegistryScanProgress, RegistryScanRequest, is_generated_mod_id,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};

use crate::assets::read_icon_data_url;
use crate::auth::{authenticated_server_and_token, endpoint_url};
use crate::installer::{
    copy_cached_modpack_to_profile, install_selected_project, install_selected_project_root,
};
use crate::model::{AppState, LauncherInstallResult, LauncherSettings, PublicProfile};
use crate::{ensure_settings_dirs, read_modpack_file};

const MAX_LOCAL_FILES: usize = 100_000;
const MAX_LOCAL_DEPTH: usize = 64;
const MAX_REMOTE_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const PROFILE_ORIGIN_DIR: &str = ".patchwork";

#[tauri::command]
pub(crate) async fn registry_browse(
    state: State<'_, AppState>,
    input: RegistryBrowseRequest,
) -> Result<RegistryBrowseResponse, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    tauri::async_runtime::spawn_blocking(move || browse_registry(&settings, input))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_project_details(
    state: State<'_, AppState>,
    project: RegistryProjectRef,
) -> Result<RegistryProjectDetails, String> {
    let backend = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .backend
        .clone();
    tauri::async_runtime::spawn_blocking(move || fetch_project_details(&backend, project))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_publisher_profile(
    state: State<'_, AppState>,
    nickname: String,
) -> Result<PublicProfile, String> {
    let backend = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .backend
        .clone();
    tauri::async_runtime::spawn_blocking(move || fetch_publisher_profile(&backend, &nickname))
        .await
        .map_err(|error| error.to_string())?
}

fn fetch_publisher_profile(backend: &str, nickname: &str) -> Result<PublicProfile, String> {
    let mut url = url::Url::parse(&endpoint_url(backend, "/api/profiles/")?)
        .map_err(|error| error.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "backend URL cannot contain profile path segments".to_owned())?
        .push(nickname);
    parse_json_response(
        ureq::get(url.as_str()).call(),
        "could not load publisher profile",
    )
}

pub(crate) fn fetch_project_details(
    backend: &str,
    project: RegistryProjectRef,
) -> Result<RegistryProjectDetails, String> {
    let path = format!(
        "/registry/projects/{}/{}",
        project.project_kind.route_segment(),
        project.project_id
    );
    let url = endpoint_url(backend, &path)?;
    let mut details: RegistryProjectDetails =
        parse_json_response(ureq::get(&url).call(), "could not load registry project")?;
    if details.manifest_url.starts_with('/') {
        details.manifest_url = endpoint_url(backend, &details.manifest_url)?;
    }
    for value in [
        &mut details.source_url,
        &mut details.readme_url,
        &mut details.image_url,
    ] {
        if let Some(path) = value.as_deref() {
            if path.starts_with('/') {
                *value = Some(endpoint_url(backend, path)?);
            }
        }
    }
    Ok(details)
}

#[tauri::command]
pub(crate) async fn registry_add_to_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    input: RegistryAddToProfileRequest,
) -> Result<LauncherInstallResult, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    let download_running = state.download_running.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_settings_dirs(&settings).map_err(|error| error.to_string())?;
        let profile_id = add_project_to_profile(&settings, &input)?;
        let report = install_selected_project(
            &app,
            &settings,
            &download_running,
            input.project,
            input.selected_project,
        )?;
        let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
        let profile =
            read_modpack_file(&profile_path, &settings).map_err(|error| error.to_string())?;
        Ok(LauncherInstallResult { profile, report })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_download_modpack_as_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    project: RegistryBrowseProject,
) -> Result<LauncherInstallResult, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "launcher settings lock is poisoned".to_owned())?
        .clone();
    let download_running = state.download_running.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if project.project_kind != RegistryProjectKind::Modpack {
            return Err("Only a modpack can be downloaded as a profile".to_owned());
        }
        let id = sanitize_id(&project.project_id)?;
        let origin = project.clone();
        let destination = Path::new(&settings.profiles_dir).join(format!("{id}.toml"));
        if destination.exists() {
            return Err(format!("A profile with ID '{id}' already exists"));
        }
        let report = install_selected_project_root(
            &app,
            &settings,
            &download_running,
            project.project_ref(),
            project,
        )?;
        if !report.errors.is_empty() {
            return Err(report.errors.join("\n"));
        }
        copy_cached_modpack_to_profile(&settings, &id)?;
        save_profile_origin(&settings, &id, &origin)?;
        let profile =
            read_modpack_file(&destination, &settings).map_err(|error| error.to_string())?;
        Ok(LauncherInstallResult { profile, report })
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn load_profile_origin(
    settings: &LauncherSettings,
    id: &str,
) -> Option<RegistryBrowseProject> {
    let path = profile_origin_path(settings, id).ok()?;
    let source = fs::read_to_string(path).ok()?;
    serde_json::from_str(&source).ok()
}

fn save_profile_origin<T: Serialize>(
    settings: &LauncherSettings,
    id: &str,
    origin: &T,
) -> Result<(), String> {
    let path = profile_origin_path(settings, id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "profile origin path has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let source = serde_json::to_vec_pretty(origin).map_err(|error| error.to_string())?;
    fs::write(path, source).map_err(|error| error.to_string())
}

pub(crate) fn remove_profile_origin(settings: &LauncherSettings, id: &str) -> Result<(), String> {
    let path = profile_origin_path(settings, id)?;
    if path.is_file() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn profile_origin_path(settings: &LauncherSettings, id: &str) -> Result<PathBuf, String> {
    let id = sanitize_id(id)?;
    Ok(Path::new(&settings.profiles_dir)
        .join(PROFILE_ORIGIN_DIR)
        .join(format!("{id}.origin.json")))
}

fn add_project_to_profile(
    settings: &LauncherSettings,
    input: &RegistryAddToProfileRequest,
) -> Result<String, String> {
    let profile_id = sanitize_id(&input.profile_id)?;
    let project_id = sanitize_id(&input.project.project_id)?;
    let profile_path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    if !profile_path.is_file() {
        return Err(format!("Profile '{profile_id}' does not exist"));
    }
    let source = fs::read_to_string(&profile_path)
        .map_err(|error| format!("Failed to read '{}': {error}", profile_path.display()))?;
    let mut profile = toml::from_str::<toml::Table>(&source)
        .map_err(|error| format!("Failed to parse '{}': {error}", profile_path.display()))?;
    let dependency_key = match input.project.project_kind {
        RegistryProjectKind::Mod => "mods",
        RegistryProjectKind::Modpack => "modpacks",
    };
    let dependencies = profile
        .entry(dependency_key)
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| format!("Profile field '{dependency_key}' must be an array"))?;
    let mut dependencies = dependencies
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("Profile field '{dependency_key}' must contain strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !dependencies.contains(&project_id) {
        dependencies.push(project_id);
        dependencies.sort();
        dependencies.dedup();
        profile.insert(
            dependency_key.to_owned(),
            toml::Value::Array(dependencies.into_iter().map(toml::Value::String).collect()),
        );
        let source = toml::to_string_pretty(&profile).map_err(|error| error.to_string())?;
        fs::write(&profile_path, source)
            .map_err(|error| format!("Failed to update '{}': {error}", profile_path.display()))?;
    }
    Ok(profile_id)
}

#[tauri::command]
pub(crate) async fn registry_create_scan(
    state: State<'_, AppState>,
    input: RegistryScanRequest,
) -> Result<RegistryScan, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, "/registry/scans")?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(input).map_err(|error| error.to_string())?);
        parse_json_response(request, "repository scan failed")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_start_scan(
    state: State<'_, AppState>,
    input: RegistryScanRequest,
) -> Result<RegistryScanJobStarted, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, "/registry/scan-jobs")?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(input).map_err(|error| error.to_string())?);
        parse_json_response(request, "could not start repository scan")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_scan_progress(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<RegistryScanProgress, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, &format!("/registry/scan-jobs/{job_id}"))?;
        let request = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "could not load repository scan progress")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_get_scan(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<RegistryScan, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, &format!("/registry/scans/{scan_id}"))?;
        let request = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "could not load registry scan")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_publish_scan(
    state: State<'_, AppState>,
    scan_id: String,
    input: RegistryPublishRequest,
) -> Result<RegistryPublishResponse, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, &format!("/registry/scans/{scan_id}/publish"))?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(input).map_err(|error| error.to_string())?);
        parse_json_response(request, "registry publish failed")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_rescan_mod(
    state: State<'_, AppState>,
    mod_id: String,
) -> Result<RegistryScan, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let encoded_id: String = url::form_urlencoded::byte_serialize(mod_id.as_bytes()).collect();
        let url = endpoint_url(&server_url, &format!("/registry/mods/{encoded_id}/rescan"))?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "registry rescan failed")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_start_rescan(
    state: State<'_, AppState>,
    project: RegistryProjectRef,
) -> Result<RegistryScanJobStarted, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let encoded_id: String =
            url::form_urlencoded::byte_serialize(project.project_id.as_bytes()).collect();
        let url = endpoint_url(
            &server_url,
            &format!(
                "/registry/projects/{}/{encoded_id}/rescan-job",
                project.project_kind.route_segment()
            ),
        )?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "could not start registry rescan")
    })
    .await
    .map_err(|error| error.to_string())?
}

fn browse_registry(
    settings: &LauncherSettings,
    input: RegistryBrowseRequest,
) -> Result<RegistryBrowseResponse, String> {
    let mut projects = Vec::new();
    let mut warnings = Vec::new();

    match remote_registry_search(&settings.backend, &input) {
        Ok(mut response) => {
            for project in &mut response.projects {
                absolutize_project_urls(project, &settings.backend)?;
            }
            projects.append(&mut response.projects);
            warnings.append(&mut response.warnings);
        }
        Err(error) => warnings.push(format!("Backend search failed: {error}")),
    }

    for folder in &settings.local_folders {
        match browse_local_folder(Path::new(folder), &input) {
            Ok(mut local) => projects.append(&mut local),
            Err(error) => warnings.push(format!("Local registry '{folder}': {error}")),
        }
    }
    projects.retain(|project| {
        project.project_kind != RegistryProjectKind::Mod
            || !is_generated_mod_id(&project.project_id)
    });
    projects.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.project_id.cmp(&right.project_id))
            .then_with(|| source_order(left.source).cmp(&source_order(right.source)))
    });
    Ok(RegistryBrowseResponse { projects, warnings })
}

fn remote_registry_search(
    backend: &str,
    input: &RegistryBrowseRequest,
) -> Result<RegistryBrowseResponse, String> {
    let mut url = url::Url::parse(&endpoint_url(backend, "/registry/search")?)
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("q", input.query.trim())
        .append_pair("mods", if input.include_mods { "true" } else { "false" })
        .append_pair(
            "modpacks",
            if input.include_modpacks {
                "true"
            } else {
                "false"
            },
        );
    parse_json_response(ureq::get(url.as_str()).call(), "registry search failed")
}

fn absolutize_project_urls(
    project: &mut RegistryBrowseProject,
    backend: &str,
) -> Result<(), String> {
    for value in [
        &mut project.manifest_url,
        &mut project.readme_url,
        &mut project.image_url,
    ] {
        if let Some(path) = value.as_deref() {
            if path.starts_with('/') {
                *value = Some(endpoint_url(backend, path)?);
            }
        }
    }
    Ok(())
}

pub(crate) fn browse_local_folder(
    folder: &Path,
    input: &RegistryBrowseRequest,
) -> Result<Vec<RegistryBrowseProject>, String> {
    let root = folder
        .canonicalize()
        .map_err(|error| format!("cannot open folder: {error}"))?;
    if !root.is_dir() {
        return Err("path is not a directory".to_owned());
    }
    let mut paths = Vec::new();
    collect_registry_files(&root, 0, &mut paths)?;
    let mut cargo_sources = HashMap::<PathBuf, String>::new();
    for path in paths
        .iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml"))
    {
        if let Ok(source) = fs::read_to_string(path) {
            cargo_sources.insert(path.clone(), source);
        }
    }
    let workspace_manifests = cargo_sources
        .iter()
        .map(|(path, source)| RegistryWorkspaceManifest {
            path,
            source: source.as_str(),
        })
        .collect::<Vec<_>>();
    let mut projects = Vec::new();

    if input.include_mods {
        for (path, source) in &cargo_sources {
            let Ok(Some(manifest)) =
                parse_registry_mod_manifest(source, path, &workspace_manifests)
            else {
                continue;
            };
            if is_generated_mod_id(&manifest.id) {
                continue;
            }
            let description = "Local Patchwork mod".to_owned();
            if !matches_query(
                &input.query,
                [
                    manifest.id.as_str(),
                    manifest.title.as_str(),
                    description.as_str(),
                ],
            ) {
                continue;
            }
            let parent = path.parent().unwrap_or(&root);
            let manifest_sha256 = sha256_hex(source.as_bytes());
            projects.push(local_project(
                RegistryProjectKind::Mod,
                manifest.id,
                manifest.title,
                description,
                manifest.version,
                path,
                parent,
                &root,
                manifest_sha256,
            ));
        }
    }

    if input.include_modpacks {
        for path in paths.iter().filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
                && path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml")
        }) {
            let source = match fs::read_to_string(path) {
                Ok(source) => source,
                Err(_) => continue,
            };
            let Ok(Some(manifest)) = parse_registry_modpack_manifest(&source, path) else {
                continue;
            };
            if !matches_query(
                &input.query,
                [
                    manifest.id.as_str(),
                    manifest.title.as_str(),
                    manifest.modpack.description.as_str(),
                ],
            ) {
                continue;
            }
            let parent = path.parent().unwrap_or(&root);
            projects.push(local_project(
                RegistryProjectKind::Modpack,
                manifest.id,
                manifest.title,
                manifest.modpack.description,
                manifest.version,
                path,
                parent,
                &root,
                sha256_hex(source.as_bytes()),
            ));
        }
    }
    Ok(projects)
}

#[allow(clippy::too_many_arguments)]
fn local_project(
    project_kind: RegistryProjectKind,
    project_id: String,
    title: String,
    description: String,
    version: String,
    manifest_path: &Path,
    project_dir: &Path,
    root: &Path,
    manifest_sha256: String,
) -> RegistryBrowseProject {
    let image = local_project_image(project_dir, &project_id);
    RegistryBrowseProject {
        project_kind,
        project_id: project_id.clone(),
        title,
        description,
        version,
        downloads: 0,
        source: RegistryBrowseSource::Local,
        source_label: root.display().to_string(),
        repository_url: None,
        repository_path: project_dir
            .strip_prefix(root)
            .ok()
            .map(|path| path.display().to_string()),
        source_commit: None,
        source_tree_oid: None,
        manifest_sha256: Some(manifest_sha256),
        manifest_url: None,
        readme_url: local_project_readme(project_kind, project_dir, &project_id)
            .map(|path| path.display().to_string()),
        image_url: image
            .as_deref()
            .and_then(|path| read_icon_data_url(path).ok()),
        local_manifest_path: Some(manifest_path.display().to_string()),
    }
}

fn collect_registry_files(
    directory: &Path,
    depth: usize,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > MAX_LOCAL_DEPTH {
        return Err(format!(
            "directory nesting exceeds {MAX_LOCAL_DEPTH} levels"
        ));
    }
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let name = entry.file_name();
            if name == ".git" || name == "target" {
                continue;
            }
            collect_registry_files(&path, depth + 1, paths)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let is_candidate = path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"));
        if is_candidate {
            paths.push(path);
            if paths.len() > MAX_LOCAL_FILES {
                return Err(format!(
                    "folder contains more than {MAX_LOCAL_FILES} manifests"
                ));
            }
        }
    }
    Ok(())
}

fn matches_query<'a>(query: &str, values: impl IntoIterator<Item = &'a str>) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || values
            .into_iter()
            .any(|value| value.to_lowercase().contains(&query))
}

pub(crate) fn local_project_image(directory: &Path, id: &str) -> Option<PathBuf> {
    ["png", "webp", "jpg", "jpeg"]
        .into_iter()
        .map(|extension| directory.join(format!("{id}.{extension}")))
        .find(|path| path.is_file())
}

pub(crate) fn local_project_readme(
    kind: RegistryProjectKind,
    directory: &Path,
    id: &str,
) -> Option<PathBuf> {
    match kind {
        RegistryProjectKind::Modpack => {
            let path = directory.join(format!("{id}.md"));
            path.is_file().then_some(path)
        }
        RegistryProjectKind::Mod => fs::read_dir(directory)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
            }),
    }
}

pub(crate) struct DownloadedArtifact {
    pub(crate) bytes: Vec<u8>,
    pub(crate) filename: String,
}

pub(crate) fn fetch_artifact(url: &str) -> Result<DownloadedArtifact, String> {
    let response = ureq::get(url).call().map_err(|error| error.to_string())?;
    let filename = response
        .header("X-Patchwork-Filename")
        .unwrap_or("artifact")
        .to_owned();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_REMOTE_ARTIFACT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_REMOTE_ARTIFACT_BYTES {
        return Err("Published artifact is too large".to_owned());
    }
    Ok(DownloadedArtifact { bytes, filename })
}

pub(crate) fn sanitize_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.is_empty()
        || id.len() > 128
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || id.bytes().any(|byte| {
            !byte.is_ascii_lowercase()
                && !byte.is_ascii_digit()
                && !matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(format!("Invalid Patchwork project ID '{id}'"));
    }
    Ok(id.to_owned())
}

pub(crate) fn image_extension(path: &str) -> Option<&'static str> {
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("png"),
        "webp" => Some("webp"),
        "jpg" => Some("jpg"),
        "jpeg" => Some("jpeg"),
        _ => None,
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const fn source_order(source: RegistryBrowseSource) -> u8 {
    match source {
        RegistryBrowseSource::Local => 0,
        RegistryBrowseSource::Remote => 1,
    }
}

fn parse_json_response<T: DeserializeOwned>(
    response: Result<ureq::Response, ureq::Error>,
    fallback: &str,
) -> Result<T, String> {
    match response {
        Ok(response) => response.into_json().map_err(|error| error.to_string()),
        Err(ureq::Error::Status(_, response)) => Err(response
            .into_string()
            .unwrap_or_else(|_| fallback.to_owned())),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_browse_finds_mods_and_modpacks_and_applies_filters() {
        let directory = tempfile::tempdir().unwrap();
        let mod_directory = directory.path().join("mods/example-mod");
        fs::create_dir_all(&mod_directory).unwrap();
        fs::write(
            mod_directory.join("Cargo.toml"),
            r#"[package]
name = "example-mod"
version = "1.2.3"

[package.metadata.mod]
title = "Example Mod"
support = true
"#,
        )
        .unwrap();
        fs::write(
            directory.path().join("example-pack.toml"),
            r#"version = "2.0.0"
name = "Example Pack"
description = "A local example"
mods = ["example-mod"]
"#,
        )
        .unwrap();

        let all = browse_local_folder(directory.path(), &RegistryBrowseRequest::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|project| {
            project.project_kind == RegistryProjectKind::Mod
                && project.project_id == "example-mod"
                && project
                    .manifest_sha256
                    .as_ref()
                    .is_some_and(|hash| hash.len() == 64)
        }));
        assert!(all.iter().any(|project| {
            project.project_kind == RegistryProjectKind::Modpack
                && project.project_id == "example-pack"
        }));

        let modpacks_only = browse_local_folder(
            directory.path(),
            &RegistryBrowseRequest {
                query: "example".to_owned(),
                include_mods: false,
                include_modpacks: true,
            },
        )
        .unwrap();
        assert_eq!(modpacks_only.len(), 1);
        assert_eq!(modpacks_only[0].project_id, "example-pack");
    }
}
