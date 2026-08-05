use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use flate2::read::GzDecoder;
use patchwork::{
    RegistryDependencyTargetKind, RegistryWorkspaceManifest, parse_registry_dependency,
    parse_registry_mod_manifest, parse_registry_modpack_manifest,
};
use patchwork_registry_types::{
    RegistryBrowseProject, RegistryBrowseRequest, RegistryBrowseSource, RegistryDependencyKind,
    RegistryProjectDetails, RegistryProjectKind, RegistryProjectRef, is_generated_mod_id,
};
use semver::Version;
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    auth::endpoint_url,
    ensure_settings_dirs,
    model::{
        AppState, LauncherSettings, PATCHWORK_DOWNLOAD_EVENT, RegistryDownloadEvent,
        RegistryInstallReport,
    },
    registry::{
        browse_local_folder, fetch_artifact, image_extension, local_project_image,
        local_project_readme, sanitize_id, sha256_hex,
    },
};

const MAX_SOURCE_ARCHIVE_BYTES: usize = 192 * 1024 * 1024;
const MAX_EXTRACTED_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SOURCE_FILES: usize = 20_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ProjectKey {
    kind: RegistryProjectKind,
    id: String,
}

impl ProjectKey {
    fn new(kind: RegistryProjectKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }

    fn label(&self) -> String {
        format!(
            "{} {}",
            match self.kind {
                RegistryProjectKind::Mod => "mod",
                RegistryProjectKind::Modpack => "modpack",
            },
            self.id
        )
    }

    fn is_generated_mod(&self) -> bool {
        self.kind == RegistryProjectKind::Mod && is_generated_mod_id(&self.id)
    }
}

#[derive(Clone)]
struct LocalCandidate {
    project: RegistryBrowseProject,
    dependencies: Vec<ProjectKey>,
}

#[derive(Clone)]
enum CandidateSource {
    Cached,
    Local(RegistryBrowseProject),
    Remote(RegistryProjectDetails),
}

#[derive(Clone)]
struct Candidate {
    key: ProjectKey,
    version: String,
    dependencies: Vec<ProjectKey>,
    source: CandidateSource,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum InstallMode {
    Missing,
    Updates,
    CheckUpdates,
}

struct DownloadGuard<'a> {
    running: &'a AtomicBool,
}

impl<'a> DownloadGuard<'a> {
    fn acquire(running: &'a AtomicBool) -> Result<Self, String> {
        running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Another Patchwork download is already running".to_owned())?;
        Ok(Self { running })
    }
}

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}

pub(crate) fn profile_roots(
    settings: &LauncherSettings,
    profile_id: &str,
) -> Result<Vec<RegistryProjectRef>, String> {
    let profile_id = sanitize_id(profile_id)?;
    let path = Path::new(&settings.profiles_dir).join(format!("{profile_id}.toml"));
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read profile '{}': {error}", path.display()))?;
    let profile = toml::from_str::<patchwork::Modpack>(&source)
        .map_err(|error| format!("Failed to parse profile '{}': {error}", path.display()))?;
    let ignored = profile.ignore.into_iter().collect::<HashSet<_>>();
    Ok(profile
        .modpacks
        .into_iter()
        .map(|id| RegistryProjectRef {
            project_kind: RegistryProjectKind::Modpack,
            project_id: id,
        })
        .chain(
            profile
                .mods
                .into_iter()
                .filter(|id| !ignored.contains(id))
                .map(|id| RegistryProjectRef {
                    project_kind: RegistryProjectKind::Mod,
                    project_id: id,
                }),
        )
        .collect())
}

pub(crate) fn install_profile_dependencies(
    app: &AppHandle,
    settings: &LauncherSettings,
    download_running: &AtomicBool,
    profile_id: &str,
    updates_only: bool,
) -> Result<RegistryInstallReport, String> {
    let roots = profile_roots(settings, profile_id)?;
    install_projects(
        app,
        settings,
        download_running,
        roots,
        Vec::new(),
        if updates_only {
            InstallMode::Updates
        } else {
            InstallMode::Missing
        },
        true,
    )
}

pub(crate) fn install_selected_project(
    app: &AppHandle,
    settings: &LauncherSettings,
    download_running: &AtomicBool,
    project: RegistryProjectRef,
    selected: Option<RegistryBrowseProject>,
) -> Result<RegistryInstallReport, String> {
    install_projects(
        app,
        settings,
        download_running,
        vec![project],
        selected.into_iter().collect(),
        InstallMode::Missing,
        true,
    )
}

pub(crate) fn install_selected_project_root(
    app: &AppHandle,
    settings: &LauncherSettings,
    download_running: &AtomicBool,
    project: RegistryProjectRef,
    selected: RegistryBrowseProject,
) -> Result<RegistryInstallReport, String> {
    install_projects(
        app,
        settings,
        download_running,
        vec![project],
        vec![selected],
        InstallMode::Updates,
        false,
    )
}

pub(crate) fn check_profile_updates(
    settings: &LauncherSettings,
    profile_id: &str,
) -> Result<(usize, Vec<String>), String> {
    let roots = profile_roots(settings, profile_id)?;
    let mut installer = Installer::new(settings, Vec::new())?;
    let report = installer.process(None, roots, InstallMode::CheckUpdates, true);
    Ok((report.installed, report.errors))
}

fn install_projects(
    app: &AppHandle,
    settings: &LauncherSettings,
    download_running: &AtomicBool,
    roots: Vec<RegistryProjectRef>,
    selected: Vec<RegistryBrowseProject>,
    mode: InstallMode,
    traverse_dependencies: bool,
) -> Result<RegistryInstallReport, String> {
    ensure_settings_dirs(settings).map_err(|error| error.to_string())?;
    let _guard = DownloadGuard::acquire(download_running)?;
    let roots = roots
        .into_iter()
        .filter(|project| {
            project.project_kind != RegistryProjectKind::Mod
                || !is_generated_mod_id(&project.project_id)
        })
        .collect::<Vec<_>>();
    emit_progress(
        app,
        true,
        mode,
        0,
        roots.len(),
        None,
        "Preparing dependency download".to_owned(),
        Vec::new(),
    );
    let mut installer = Installer::new(settings, selected)?;
    let report = installer.process(Some(app), roots, mode, traverse_dependencies);
    emit_progress(
        app,
        false,
        mode,
        report.installed + report.up_to_date + report.errors.len(),
        report.installed + report.up_to_date + report.errors.len(),
        None,
        if report.errors.is_empty() {
            format!("Installed {} project(s)", report.installed)
        } else {
            format!("Download finished with {} error(s)", report.errors.len())
        },
        report.errors.clone(),
    );
    Ok(report)
}

struct Installer<'a> {
    settings: &'a LauncherSettings,
    local: HashMap<ProjectKey, LocalCandidate>,
    selected: HashMap<ProjectKey, RegistryBrowseProject>,
    remote: HashMap<ProjectKey, RegistryProjectDetails>,
}

impl<'a> Installer<'a> {
    fn new(
        settings: &'a LauncherSettings,
        selected: Vec<RegistryBrowseProject>,
    ) -> Result<Self, String> {
        let mut local = HashMap::new();
        for folder in &settings.local_folders {
            let root = Path::new(folder);
            let projects = match browse_local_folder(root, &RegistryBrowseRequest::default()) {
                Ok(projects) => projects,
                Err(_) => continue,
            };
            for project in projects {
                let key = ProjectKey::new(project.project_kind, project.project_id.clone());
                if local.contains_key(&key) {
                    continue;
                }
                if let Ok(dependencies) = local_dependencies(&project, root) {
                    local.insert(
                        key,
                        LocalCandidate {
                            project,
                            dependencies,
                        },
                    );
                }
            }
        }
        let selected = selected
            .into_iter()
            .map(|project| {
                (
                    ProjectKey::new(project.project_kind, project.project_id.clone()),
                    project,
                )
            })
            .collect();
        Ok(Self {
            settings,
            local,
            selected,
            remote: HashMap::new(),
        })
    }

    fn process(
        &mut self,
        app: Option<&AppHandle>,
        roots: Vec<RegistryProjectRef>,
        mode: InstallMode,
        traverse_dependencies: bool,
    ) -> RegistryInstallReport {
        let mut report = RegistryInstallReport::default();
        let mut scheduled = HashSet::new();
        let mut queue = roots
            .into_iter()
            .map(|project| ProjectKey::new(project.project_kind, project.project_id))
            .filter(|project| !project.is_generated_mod())
            .filter(|project| scheduled.insert(project.clone()))
            .collect::<VecDeque<_>>();
        let mut visited = HashSet::new();
        let mut processed = 0_usize;
        while let Some(key) = queue.pop_front() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let total = scheduled.len();
            if let Some(app) = app {
                emit_progress(
                    app,
                    true,
                    mode,
                    processed,
                    total,
                    Some(key.id.clone()),
                    format!("Resolving {}", key.label()),
                    report.errors.clone(),
                );
            }
            let candidate = match self.resolve_candidate(&key, mode) {
                Ok(candidate) => candidate,
                Err(error) => {
                    report.errors.push(format!("{}: {error}", key.label()));
                    processed += 1;
                    if let Some(app) = app {
                        emit_progress(
                            app,
                            true,
                            mode,
                            processed,
                            scheduled.len(),
                            Some(key.id.clone()),
                            format!("Failed to resolve {}", key.label()),
                            report.errors.clone(),
                        );
                    }
                    continue;
                }
            };
            if traverse_dependencies {
                for dependency in &candidate.dependencies {
                    if dependency.is_generated_mod() {
                        continue;
                    }
                    if scheduled.insert(dependency.clone()) {
                        queue.push_back(dependency.clone());
                    }
                }
            }

            let operation = match (&candidate.source, mode) {
                (_, InstallMode::CheckUpdates) => "Checking",
                (CandidateSource::Cached, _) => "Checking cache for",
                (CandidateSource::Local(_), _) => "Copying",
                (CandidateSource::Remote(_), _) => "Downloading",
            };
            if let Some(app) = app {
                emit_progress(
                    app,
                    true,
                    mode,
                    processed,
                    scheduled.len(),
                    Some(key.id.clone()),
                    format!("{operation} {}", key.label()),
                    report.errors.clone(),
                );
            }

            let should_install = !matches!(&candidate.source, CandidateSource::Cached);
            let mut succeeded = true;
            if should_install {
                if mode == InstallMode::CheckUpdates {
                    report.installed += 1;
                } else {
                    match self.install_candidate(&candidate) {
                        Ok(()) => report.installed += 1,
                        Err(error) => {
                            report
                                .errors
                                .push(format!("{}: {error}", candidate.key.label()));
                            succeeded = false;
                        }
                    }
                }
            } else {
                report.up_to_date += 1;
            }
            processed += 1;
            if let Some(app) = app {
                emit_progress(
                    app,
                    true,
                    mode,
                    processed,
                    scheduled.len(),
                    Some(key.id.clone()),
                    if succeeded {
                        format!("Processed {}", key.label())
                    } else {
                        format!("Failed to install {}", key.label())
                    },
                    report.errors.clone(),
                );
            }
        }
        report
    }

    fn resolve_candidate(
        &mut self,
        key: &ProjectKey,
        mode: InstallMode,
    ) -> Result<Candidate, String> {
        let cached = cached_candidate(self.settings, key).ok();
        if mode == InstallMode::Missing {
            if let Some(cached) = cached {
                return Ok(cached);
            }
            return self.download_candidate(key);
        }

        let available = self.download_candidate(key);
        match (cached, available) {
            (None, Ok(available)) => Ok(available),
            (None, Err(error)) => Err(error),
            (Some(cached), Ok(available)) => {
                if newer_than(&available.version, &cached.version) {
                    Ok(available)
                } else {
                    Ok(cached)
                }
            }
            (Some(cached), Err(_)) => Ok(cached),
        }
    }

    fn download_candidate(&mut self, key: &ProjectKey) -> Result<Candidate, String> {
        if let Some(local) = self.local.get(key) {
            return Ok(Candidate {
                key: key.clone(),
                version: local.project.version.clone(),
                dependencies: local.dependencies.clone(),
                source: CandidateSource::Local(local.project.clone()),
            });
        }
        if let Some(selected) = self.selected.get(key) {
            if selected.source == RegistryBrowseSource::Local {
                return Ok(Candidate {
                    key: key.clone(),
                    version: selected.version.clone(),
                    dependencies: local_dependencies(selected, Path::new("."))?,
                    source: CandidateSource::Local(selected.clone()),
                });
            }
        }
        let details = self.remote_details(key)?.clone();
        let dependencies = details
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind != RegistryDependencyKind::Ignore)
            .map(|dependency| ProjectKey::new(dependency.target_kind, dependency.target_id.clone()))
            .collect();
        Ok(Candidate {
            key: key.clone(),
            version: details.version.clone(),
            dependencies,
            source: CandidateSource::Remote(details),
        })
    }

    fn remote_details(&mut self, key: &ProjectKey) -> Result<&RegistryProjectDetails, String> {
        if !self.remote.contains_key(key) {
            let route = format!("/registry/projects/{}/{}", key.kind.route_segment(), key.id);
            let url = endpoint_url(&self.settings.backend, &route)?;
            let mut details: RegistryProjectDetails =
                parse_json_response(ureq::get(&url).call(), "registry project lookup failed")?;
            absolutize_details(&mut details, &self.settings.backend)?;
            self.remote.insert(key.clone(), details);
        }
        self.remote
            .get(key)
            .ok_or_else(|| "registry project lookup returned no data".to_owned())
    }

    fn install_candidate(&self, candidate: &Candidate) -> Result<(), String> {
        match (&candidate.key.kind, &candidate.source) {
            (_, CandidateSource::Cached) => Ok(()),
            (RegistryProjectKind::Mod, CandidateSource::Local(project)) => {
                install_local_mod(self.settings, project, &candidate.version)
            }
            (RegistryProjectKind::Modpack, CandidateSource::Local(project)) => {
                install_local_modpack(self.settings, project)
            }
            (RegistryProjectKind::Mod, CandidateSource::Remote(details)) => {
                install_remote_mod(self.settings, details)
            }
            (RegistryProjectKind::Modpack, CandidateSource::Remote(details)) => {
                install_remote_modpack(self.settings, details)
            }
        }
    }
}

fn cached_candidate(settings: &LauncherSettings, key: &ProjectKey) -> Result<Candidate, String> {
    match key.kind {
        RegistryProjectKind::Mod => {
            let manifest_path = Path::new(&settings.mod_cache)
                .join(&key.id)
                .join("Cargo.toml");
            let source = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
            let manifest = parse_registry_mod_manifest(
                &source,
                &manifest_path,
                &[RegistryWorkspaceManifest {
                    path: &manifest_path,
                    source: &source,
                }],
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "cached Cargo.toml is not a Patchwork mod".to_owned())?;
            Ok(Candidate {
                key: key.clone(),
                version: manifest.version,
                dependencies: mod_dependencies(&manifest.mod_info, &manifest_path)?,
                source: CandidateSource::Cached,
            })
        }
        RegistryProjectKind::Modpack => {
            let manifest_path =
                Path::new(&settings.modpacks_cache).join(format!("{}.toml", key.id));
            let source = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
            let manifest = parse_registry_modpack_manifest(&source, &manifest_path)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "cached TOML is not a Patchwork modpack".to_owned())?;
            Ok(Candidate {
                key: key.clone(),
                version: manifest.version,
                dependencies: manifest
                    .dependencies
                    .into_iter()
                    .filter(|dependency| !dependency.ignored)
                    .map(|dependency| {
                        ProjectKey::new(registry_kind(dependency.target_kind), dependency.target_id)
                    })
                    .collect(),
                source: CandidateSource::Cached,
            })
        }
    }
}

fn local_dependencies(
    project: &RegistryBrowseProject,
    root: &Path,
) -> Result<Vec<ProjectKey>, String> {
    let manifest_path = project
        .local_manifest_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "local project has no manifest path".to_owned())?;
    let source = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
    match project.project_kind {
        RegistryProjectKind::Mod => {
            let owned = workspace_sources(&manifest_path, root);
            let manifests = owned
                .iter()
                .map(|(path, source)| RegistryWorkspaceManifest {
                    path,
                    source: source.as_str(),
                })
                .collect::<Vec<_>>();
            let manifest = parse_registry_mod_manifest(&source, &manifest_path, &manifests)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "local Cargo.toml is not a Patchwork mod".to_owned())?;
            mod_dependencies(&manifest.mod_info, &manifest_path)
        }
        RegistryProjectKind::Modpack => {
            let manifest = parse_registry_modpack_manifest(&source, &manifest_path)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "local TOML is not a Patchwork modpack".to_owned())?;
            Ok(manifest
                .dependencies
                .into_iter()
                .filter(|dependency| !dependency.ignored)
                .map(|dependency| {
                    ProjectKey::new(registry_kind(dependency.target_kind), dependency.target_id)
                })
                .collect())
        }
    }
}

fn workspace_sources(manifest_path: &Path, root: &Path) -> Vec<(PathBuf, String)> {
    let mut manifests = Vec::new();
    for directory in manifest_path.parent().into_iter().flat_map(Path::ancestors) {
        let path = directory.join("Cargo.toml");
        if let Ok(source) = fs::read_to_string(&path) {
            manifests.push((path, source));
        }
        if directory == root {
            break;
        }
    }
    manifests
}

fn mod_dependencies(
    info: &patchwork::ModInfo,
    manifest_path: &Path,
) -> Result<Vec<ProjectKey>, String> {
    let mut dependencies = info
        .dependencies
        .init
        .iter()
        .chain(&info.dependencies.run)
        .chain(&info.dependencies.ownership)
        .map(|dependency| {
            let (kind, id) = parse_registry_dependency(dependency, manifest_path)
                .map_err(|error| error.to_string())?;
            Ok(ProjectKey::new(registry_kind(kind), id))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(provided_api) = &info.provides {
        dependencies.push(ProjectKey::new(
            RegistryProjectKind::Mod,
            provided_api.clone(),
        ));
    }
    Ok(dependencies)
}

fn registry_kind(kind: RegistryDependencyTargetKind) -> RegistryProjectKind {
    match kind {
        RegistryDependencyTargetKind::Mod => RegistryProjectKind::Mod,
        RegistryDependencyTargetKind::Modpack => RegistryProjectKind::Modpack,
    }
}

fn install_local_mod(
    settings: &LauncherSettings,
    project: &RegistryBrowseProject,
    version: &str,
) -> Result<(), String> {
    let manifest = project
        .local_manifest_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "local mod has no Cargo.toml path".to_owned())?;
    let source_dir = manifest
        .parent()
        .ok_or_else(|| "local mod Cargo.toml has no parent directory".to_owned())?;
    let destination = Path::new(&settings.mod_cache).join(&project.project_id);
    replace_directory(&destination, |staging| {
        copy_directory(source_dir, staging, 0)?;
        if let Some(expected) = project.manifest_sha256.as_deref() {
            verify_manifest_hash(&staging.join("Cargo.toml"), expected)?;
        }
        materialize_workspace_version(&staging.join("Cargo.toml"), version)?;
        validate_installed_mod(staging, project, version)
    })
}

fn install_remote_mod(
    settings: &LauncherSettings,
    details: &RegistryProjectDetails,
) -> Result<(), String> {
    let source_url = details
        .source_url
        .as_deref()
        .ok_or_else(|| "registry mod has no source URL".to_owned())?;
    let bytes = fetch_bytes(source_url, MAX_SOURCE_ARCHIVE_BYTES)?;
    let destination = Path::new(&settings.mod_cache).join(&details.project_id);
    replace_directory(&destination, |staging| {
        extract_source_archive(&bytes, staging)?;
        let manifest_path = staging.join("Cargo.toml");
        verify_manifest_hash(&manifest_path, &details.manifest_sha256)?;
        materialize_workspace_version(&manifest_path, &details.version)?;
        validate_installed_mod_details(staging, details)
    })
}

fn install_local_modpack(
    settings: &LauncherSettings,
    project: &RegistryBrowseProject,
) -> Result<(), String> {
    let manifest = project
        .local_manifest_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "local modpack has no manifest path".to_owned())?;
    let directory = manifest
        .parent()
        .ok_or_else(|| "local modpack manifest has no parent directory".to_owned())?;
    let readme = local_project_readme(RegistryProjectKind::Modpack, directory, &project.project_id);
    let image = local_project_image(directory, &project.project_id);
    install_modpack_files(
        settings,
        &project.project_id,
        &project.version,
        fs::read(&manifest).map_err(|error| error.to_string())?,
        readme
            .map(fs::read)
            .transpose()
            .map_err(|error| error.to_string())?,
        image
            .map(|path| {
                let extension = image_extension(path.to_string_lossy().as_ref())
                    .ok_or_else(|| "local modpack image type is unsupported".to_owned())?;
                Ok::<(String, Vec<u8>), String>((
                    extension.to_owned(),
                    fs::read(path).map_err(|error| error.to_string())?,
                ))
            })
            .transpose()?,
        project.manifest_sha256.as_deref(),
    )
}

fn install_remote_modpack(
    settings: &LauncherSettings,
    details: &RegistryProjectDetails,
) -> Result<(), String> {
    let manifest = fetch_artifact(&details.manifest_url)?;
    let readme = details
        .readme_url
        .as_deref()
        .map(fetch_artifact)
        .transpose()?
        .map(|artifact| artifact.bytes);
    let image = details
        .image_url
        .as_deref()
        .map(fetch_artifact)
        .transpose()?
        .map(|artifact| {
            let extension = image_extension(&artifact.filename)
                .ok_or_else(|| "registry modpack image type is unsupported".to_owned())?;
            Ok::<(String, Vec<u8>), String>((extension.to_owned(), artifact.bytes))
        })
        .transpose()?;
    install_modpack_files(
        settings,
        &details.project_id,
        &details.version,
        manifest.bytes,
        readme,
        image,
        Some(&details.manifest_sha256),
    )
}

fn install_modpack_files(
    settings: &LauncherSettings,
    id: &str,
    version: &str,
    manifest: Vec<u8>,
    readme: Option<Vec<u8>>,
    image: Option<(String, Vec<u8>)>,
    expected_hash: Option<&str>,
) -> Result<(), String> {
    if let Some(expected_hash) = expected_hash {
        if sha256_hex(&manifest) != expected_hash {
            return Err("modpack manifest checksum does not match the registry".to_owned());
        }
    }
    let source = std::str::from_utf8(&manifest)
        .map_err(|_| "modpack manifest is not valid UTF-8".to_owned())?;
    let virtual_path = PathBuf::from(format!("{id}.toml"));
    let parsed = parse_registry_modpack_manifest(source, &virtual_path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "downloaded TOML is not a Patchwork modpack".to_owned())?;
    if parsed.id != id || parsed.version != version {
        return Err("downloaded modpack identity/version does not match the registry".to_owned());
    }

    let cache = Path::new(&settings.modpacks_cache);
    fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let suffix = rand::random::<u64>();
    let manifest_temp = cache.join(format!(".{id}-{suffix}.toml.part"));
    fs::write(&manifest_temp, &manifest).map_err(|error| error.to_string())?;
    let readme_temp = readme
        .map(|bytes| {
            let path = cache.join(format!(".{id}-{suffix}.md.part"));
            fs::write(&path, bytes).map_err(|error| error.to_string())?;
            Ok::<_, String>(path)
        })
        .transpose()?;
    let image_temp = image
        .map(|(extension, bytes)| {
            let path = cache.join(format!(".{id}-{suffix}.{extension}.part"));
            fs::write(&path, bytes).map_err(|error| error.to_string())?;
            Ok::<_, String>((extension, path))
        })
        .transpose()?;

    remove_modpack_sidecars(cache, id)?;
    if let Some(path) = readme_temp {
        fs::rename(path, cache.join(format!("{id}.md"))).map_err(|error| error.to_string())?;
    }
    if let Some((extension, path)) = image_temp {
        fs::rename(path, cache.join(format!("{id}.{extension}")))
            .map_err(|error| error.to_string())?;
    }
    fs::rename(manifest_temp, cache.join(format!("{id}.toml"))).map_err(|error| error.to_string())
}

fn remove_modpack_sidecars(directory: &Path, id: &str) -> Result<(), String> {
    for extension in ["md", "png", "webp", "jpg", "jpeg"] {
        let path = directory.join(format!("{id}.{extension}"));
        if path.exists() {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn copy_cached_modpack_to_profile(
    settings: &LauncherSettings,
    id: &str,
) -> Result<(), String> {
    let id = sanitize_id(id)?;
    let cache = Path::new(&settings.modpacks_cache);
    let profiles = Path::new(&settings.profiles_dir);
    let destination = profiles.join(format!("{id}.toml"));
    if destination.exists() {
        return Err(format!("A profile with ID '{id}' already exists"));
    }
    fs::copy(cache.join(format!("{id}.toml")), &destination)
        .map_err(|error| format!("Failed to create profile '{id}': {error}"))?;
    for extension in ["md", "png", "webp", "jpg", "jpeg"] {
        let source = cache.join(format!("{id}.{extension}"));
        if source.is_file() {
            fs::copy(&source, profiles.join(format!("{id}.{extension}")))
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn replace_directory(
    destination: &Path,
    populate: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "cache destination has no parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let id = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let suffix = rand::random::<u64>();
    let staging = parent.join(format!(".{id}-{suffix}.part"));
    let backup = parent.join(format!(".{id}-{suffix}.backup"));
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    if let Err(error) = populate(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if destination.exists() {
        fs::rename(destination, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, destination);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(error.to_string());
    }
    if backup.exists() {
        fs::remove_dir_all(backup).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path, depth: usize) -> Result<(), String> {
    if depth > 64 {
        return Err("local mod directory nesting exceeds 64 levels".to_owned());
    }
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        let target = destination.join(&name);
        if file_type.is_symlink() {
            return Err(format!(
                "local mod contains unsupported symbolic link '{}'",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            fs::create_dir(&target).map_err(|error| error.to_string())?;
            copy_directory(&entry.path(), &target, depth + 1)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn fetch_bytes(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let response = ureq::get(url).call().map_err(http_error)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("downloaded source archive is too large".to_owned());
    }
    Ok(bytes)
}

fn extract_source_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let decoder = GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let mut files = 0_usize;
    let mut total = 0_u64;
    for entry in archive.entries().map_err(|error| error.to_string())? {
        let mut entry = entry.map_err(|error| error.to_string())?;
        let path = entry
            .path()
            .map_err(|error| error.to_string())?
            .into_owned();
        if !safe_relative_path(&path) {
            return Err("source archive contains an unsafe path".to_owned());
        }
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err("source archive contains an unsupported entry type".to_owned());
        }
        if entry_type.is_file() {
            files += 1;
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| "source archive is too large".to_owned())?;
            if files > MAX_SOURCE_FILES || total > MAX_EXTRACTED_SOURCE_BYTES {
                return Err("source archive exceeds extraction limits".to_owned());
            }
        }
        entry
            .unpack_in(destination)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn verify_manifest_hash(path: &Path, expected: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if sha256_hex(&bytes) != expected {
        return Err("downloaded Cargo.toml checksum does not match the registry".to_owned());
    }
    Ok(())
}

fn materialize_workspace_version(path: &Path, version: &str) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document = toml::from_str::<toml::Value>(&source).map_err(|error| error.to_string())?;
    let Some(package) = document
        .get_mut("package")
        .and_then(toml::Value::as_table_mut)
    else {
        return Err("downloaded Cargo.toml has no package table".to_owned());
    };
    if package
        .get("version")
        .and_then(toml::Value::as_table)
        .and_then(|version| version.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
    {
        package.insert(
            "version".to_owned(),
            toml::Value::String(version.to_owned()),
        );
        fs::write(
            path,
            toml::to_string_pretty(&document).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn validate_installed_mod(
    directory: &Path,
    project: &RegistryBrowseProject,
    version: &str,
) -> Result<(), String> {
    let details = RegistryProjectDetails {
        project_kind: project.project_kind,
        project_id: project.project_id.clone(),
        title: project.title.clone(),
        description: project.description.clone(),
        version: version.to_owned(),
        downloads: None,
        publisher_uuid: String::new(),
        publisher_name: String::new(),
        published_at: String::new(),
        repository_url: String::new(),
        repository_path: String::new(),
        source_commit: String::new(),
        source_tree_oid: String::new(),
        manifest_sha256: String::new(),
        manifest_url: String::new(),
        source_url: None,
        readme_url: None,
        image_url: None,
        dependencies: Vec::new(),
    };
    validate_installed_mod_details(directory, &details)
}

fn validate_installed_mod_details(
    directory: &Path,
    details: &RegistryProjectDetails,
) -> Result<(), String> {
    let manifest_path = directory.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path).map_err(|error| error.to_string())?;
    let manifest = parse_registry_mod_manifest(
        &source,
        &manifest_path,
        &[RegistryWorkspaceManifest {
            path: &manifest_path,
            source: &source,
        }],
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "downloaded Cargo.toml is not a Patchwork mod".to_owned())?;
    if manifest.id != details.project_id || manifest.version != details.version {
        return Err("downloaded mod identity/version does not match the registry".to_owned());
    }
    Ok(())
}

fn newer_than(candidate: &str, installed: &str) -> bool {
    match (Version::parse(candidate), Version::parse(installed)) {
        (Ok(candidate), Ok(installed)) => candidate > installed,
        _ => candidate != installed,
    }
}

fn absolutize_details(details: &mut RegistryProjectDetails, backend: &str) -> Result<(), String> {
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
    Ok(())
}

fn parse_json_response<T: DeserializeOwned>(
    response: Result<ureq::Response, ureq::Error>,
    fallback: &str,
) -> Result<T, String> {
    match response {
        Ok(response) => response.into_json().map_err(|error| error.to_string()),
        Err(error) => Err(http_error_with_fallback(error, fallback)),
    }
}

fn http_error(error: ureq::Error) -> String {
    http_error_with_fallback(error, "download failed")
}

fn http_error_with_fallback(error: ureq::Error, fallback: &str) -> String {
    match error {
        ureq::Error::Status(_, response) => response
            .into_string()
            .unwrap_or_else(|_| fallback.to_owned()),
        error => error.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    app: &AppHandle,
    running: bool,
    mode: InstallMode,
    completed: usize,
    total: usize,
    current: Option<String>,
    message: String,
    errors: Vec<String>,
) {
    let event = RegistryDownloadEvent {
        running,
        phase: match mode {
            InstallMode::Missing => "download",
            InstallMode::Updates => "updates",
            InstallMode::CheckUpdates => "refresh",
        }
        .to_owned(),
        completed,
        total,
        current,
        message,
        errors,
    };
    if let Ok(mut status) = app.state::<AppState>().download_status.lock() {
        status.update(event.clone());
    }
    let _ = app.emit(PATCHWORK_DOWNLOAD_EVENT, event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_uses_semver() {
        assert!(newer_than("1.10.0", "1.9.0"));
        assert!(!newer_than("1.0.0", "1.0.0"));
        assert!(!newer_than("1.0.0-beta.1", "1.0.0"));
    }

    #[test]
    fn archive_paths_are_relative_and_normal() {
        assert!(safe_relative_path(Path::new("src/lib.rs")));
        assert!(!safe_relative_path(Path::new("../secret")));
        assert!(!safe_relative_path(Path::new("/absolute")));
    }
}
