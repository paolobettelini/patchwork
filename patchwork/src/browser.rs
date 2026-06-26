use crate::error::{PatchworkError, Result};
use crate::graph;
use crate::model::{CargoManifest, Dependencies, ModInfo, Modpack};
use crate::modpacks;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MODPACK_PREFIX: &str = "modpack/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyTarget<'a> {
    Profile { id: &'a str },
    Modpack { id: &'a str },
    Mod { id: &'a str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencyPageKind {
    Profile,
    Modpack,
    Mod,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyPage {
    pub kind: DependencyPageKind,
    pub id: String,
    pub name: String,
    pub description: String,
    pub editable_profile: bool,
    pub distinct_dependency_count: usize,
    pub modpacks: Vec<DependencyEntry>,
    pub mods: Vec<DependencyEntry>,
    pub diagnostics: Vec<DependencyDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEntry {
    pub id: String,
    pub name: String,
    pub found: bool,
    pub ignored: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyDiagnostic {
    pub kind: String,
    pub message: String,
}

#[derive(Debug)]
struct ModpackDocument {
    id: String,
    path: PathBuf,
    modpack: Modpack,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DependencyRefKind {
    Modpack,
    Mod,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DependencyRef {
    kind: DependencyRefKind,
    id: String,
}

pub fn inspect_dependency_page(
    target: DependencyTarget<'_>,
    mods_folder: &Path,
    modpacks_folder: &Path,
    profiles_folder: &Path,
) -> Result<DependencyPage> {
    match target {
        DependencyTarget::Profile { id } => inspect_modpack_page(
            DependencyPageKind::Profile,
            id,
            profiles_folder,
            mods_folder,
            modpacks_folder,
            true,
        ),
        DependencyTarget::Modpack { id } => inspect_modpack_page(
            DependencyPageKind::Modpack,
            id,
            modpacks_folder,
            mods_folder,
            modpacks_folder,
            false,
        ),
        DependencyTarget::Mod { id } => inspect_mod_page(id, mods_folder, modpacks_folder),
    }
}

fn inspect_modpack_page(
    kind: DependencyPageKind,
    id: &str,
    source_folder: &Path,
    mods_folder: &Path,
    modpacks_folder: &Path,
    editable_profile: bool,
) -> Result<DependencyPage> {
    let document = read_modpack_document(source_folder, id)?;
    let ignored = document
        .modpack
        .ignore
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let explicit = explicit_refs_for_modpack(&document.modpack);

    let mut diagnostics = validate_modpack_graph(&document.path, mods_folder, modpacks_folder);
    let distinct_dependency_count =
        collect_distinct_count(&explicit, mods_folder, modpacks_folder, &mut diagnostics);

    let modpacks = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Modpack)
        .map(|dep| modpack_entry(&dep.id, modpacks_folder, false))
        .collect();
    let mods = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Mod)
        .map(|dep| mod_entry(&dep.id, mods_folder, ignored.contains(&dep.id)))
        .collect();

    Ok(DependencyPage {
        kind,
        id: document.id,
        name: non_empty_or(&document.modpack.name, id),
        description: non_empty_or(
            &document.modpack.description,
            "No description provided yet.",
        ),
        editable_profile,
        distinct_dependency_count,
        modpacks,
        mods,
        diagnostics: dedup_diagnostics(diagnostics),
    })
}

fn inspect_mod_page(
    id: &str,
    mods_folder: &Path,
    modpacks_folder: &Path,
) -> Result<DependencyPage> {
    let summary = read_mod_summary(id, mods_folder)?;
    let explicit = explicit_refs_for_mod(&summary.info.dependencies);
    let mut diagnostics =
        validate_mod_dependencies(id, &summary.info, mods_folder, modpacks_folder);
    let distinct_dependency_count =
        collect_distinct_count(&explicit, mods_folder, modpacks_folder, &mut diagnostics);

    let modpacks = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Modpack)
        .map(|dep| modpack_entry(&dep.id, modpacks_folder, false))
        .collect();
    let mods = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Mod)
        .map(|dep| mod_entry(&dep.id, mods_folder, false))
        .collect();

    Ok(DependencyPage {
        kind: DependencyPageKind::Mod,
        id: id.to_string(),
        name: summary.name,
        description: format!("Patchwork mod crate '{}'.", id),
        editable_profile: false,
        distinct_dependency_count,
        modpacks,
        mods,
        diagnostics: dedup_diagnostics(diagnostics),
    })
}

fn validate_modpack_graph(
    root_path: &Path,
    mods_folder: &Path,
    modpacks_folder: &Path,
) -> Vec<DependencyDiagnostic> {
    match modpacks::load_mods(root_path, mods_folder, modpacks_folder)
        .and_then(|loaded| graph::resolve(loaded.mods).map(|_| ()))
    {
        Ok(()) => Vec::new(),
        Err(error) => vec![diagnostic_from_error(error)],
    }
}

fn validate_mod_dependencies(
    mod_id: &str,
    info: &ModInfo,
    mods_folder: &Path,
    modpacks_folder: &Path,
) -> Vec<DependencyDiagnostic> {
    let mut diagnostics = Vec::new();
    for dep in explicit_refs_for_mod(&info.dependencies) {
        match dep.kind {
            DependencyRefKind::Modpack => {
                if dep.id == mod_id {
                    diagnostics.push(DependencyDiagnostic {
                        kind: "self_dependency".to_string(),
                        message: format!("mod '{mod_id}' references itself as a modpack"),
                    });
                }
                if find_modpack_path(&dep.id, modpacks_folder).is_none() {
                    diagnostics.push(DependencyDiagnostic {
                        kind: "modpack_not_found".to_string(),
                        message: format!(
                            "modpack '{}' required by mod '{}' was not found",
                            dep.id, mod_id
                        ),
                    });
                }
            }
            DependencyRefKind::Mod => {
                if dep.id == mod_id {
                    diagnostics.push(diagnostic_from_error(PatchworkError::SelfDependency {
                        mod_name: mod_id.to_string(),
                    }));
                } else if !mod_manifest_path(mods_folder, &dep.id).is_file()
                    && provider_for_dependency(mods_folder, &dep.id).is_none()
                {
                    diagnostics.push(diagnostic_from_error(PatchworkError::MissingDependency {
                        dependent_mod: mod_id.to_string(),
                        dependency: dep.id,
                    }));
                }
            }
        }
    }
    diagnostics
}

fn collect_distinct_count(
    roots: &[DependencyRef],
    mods_folder: &Path,
    modpacks_folder: &Path,
    diagnostics: &mut Vec<DependencyDiagnostic>,
) -> usize {
    let mut seen = HashSet::new();
    let mut visiting = Vec::new();
    for dep in roots {
        collect_dependency_ref(
            dep,
            mods_folder,
            modpacks_folder,
            &mut seen,
            &mut visiting,
            diagnostics,
        );
    }
    seen.len()
}

fn collect_dependency_ref(
    dep: &DependencyRef,
    mods_folder: &Path,
    modpacks_folder: &Path,
    seen: &mut HashSet<DependencyRef>,
    visiting: &mut Vec<DependencyRef>,
    diagnostics: &mut Vec<DependencyDiagnostic>,
) {
    if let Some(position) = visiting.iter().position(|visited| visited == dep) {
        let mut cycle = visiting[position..]
            .iter()
            .map(|dep| format_dependency_ref(dep))
            .collect::<Vec<_>>();
        cycle.push(format_dependency_ref(dep));
        diagnostics.push(DependencyDiagnostic {
            kind: "dependency_cycle".to_string(),
            message: format!(
                "dependency cycle detected while browsing: {}",
                cycle.join(" -> ")
            ),
        });
        return;
    }

    if !seen.insert(dep.clone()) {
        return;
    }

    visiting.push(dep.clone());
    match dep.kind {
        DependencyRefKind::Modpack => {
            if let Some(path) = find_modpack_path(&dep.id, modpacks_folder) {
                match read_modpack_document_from_path(&path) {
                    Ok(document) => {
                        for child in explicit_refs_for_modpack(&document.modpack) {
                            collect_dependency_ref(
                                &child,
                                mods_folder,
                                modpacks_folder,
                                seen,
                                visiting,
                                diagnostics,
                            );
                        }
                    }
                    Err(error) => diagnostics.push(diagnostic_from_error(error)),
                }
            }
        }
        DependencyRefKind::Mod => {
            if let Ok(summary) = read_mod_summary(&dep.id, mods_folder) {
                for child in explicit_refs_for_mod(&summary.info.dependencies) {
                    collect_dependency_ref(
                        &child,
                        mods_folder,
                        modpacks_folder,
                        seen,
                        visiting,
                        diagnostics,
                    );
                }
            }
        }
    }
    visiting.pop();
}

fn explicit_refs_for_modpack(modpack: &Modpack) -> Vec<DependencyRef> {
    let mut refs = Vec::new();
    refs.extend(modpack.modpacks.iter().map(|id| DependencyRef {
        kind: DependencyRefKind::Modpack,
        id: id.clone(),
    }));
    for dependency in &modpack.mods {
        refs.push(parse_dependency_ref(dependency));
    }
    dedup_refs(refs)
}

fn explicit_refs_for_mod(dependencies: &Dependencies) -> Vec<DependencyRef> {
    let refs = dependencies
        .init
        .iter()
        .chain(dependencies.run.iter())
        .chain(dependencies.ownership.iter())
        .map(|dependency| parse_dependency_ref(dependency))
        .collect::<Vec<_>>();
    dedup_refs(refs)
}

fn parse_dependency_ref(dependency: &str) -> DependencyRef {
    if let Some(id) = dependency.strip_prefix(MODPACK_PREFIX) {
        DependencyRef {
            kind: DependencyRefKind::Modpack,
            id: id.to_string(),
        }
    } else {
        DependencyRef {
            kind: DependencyRefKind::Mod,
            id: dependency.to_string(),
        }
    }
}

fn modpack_entry(id: &str, modpacks_folder: &Path, ignored: bool) -> DependencyEntry {
    match find_modpack_path(id, modpacks_folder)
        .and_then(|path| read_modpack_document_from_path(&path).ok())
    {
        Some(document) => DependencyEntry {
            id: id.to_string(),
            name: non_empty_or(&document.modpack.name, id),
            found: true,
            ignored,
            reason: None,
        },
        None => DependencyEntry {
            id: id.to_string(),
            name: id.to_string(),
            found: false,
            ignored,
            reason: Some("Not Found".to_string()),
        },
    }
}

fn mod_entry(id: &str, mods_folder: &Path, ignored: bool) -> DependencyEntry {
    match read_mod_summary(id, mods_folder) {
        Ok(summary) => DependencyEntry {
            id: id.to_string(),
            name: summary.name,
            found: true,
            ignored,
            reason: None,
        },
        Err(_) => DependencyEntry {
            id: id.to_string(),
            name: id.to_string(),
            found: false,
            ignored,
            reason: Some("Not Found".to_string()),
        },
    }
}

struct ModSummary {
    name: String,
    info: ModInfo,
}

fn read_mod_summary(id: &str, mods_folder: &Path) -> Result<ModSummary> {
    let manifest_path = mod_manifest_path(mods_folder, id);
    if !manifest_path.exists() {
        return Err(PatchworkError::MissingModManifest {
            mod_name: id.to_string(),
            manifest_path,
        });
    }

    let source = fs::read_to_string(&manifest_path)
        .map_err(|source| PatchworkError::io("read mod Cargo.toml", &manifest_path, source))?;
    let manifest: CargoManifest = toml::from_str(&source)
        .map_err(|source| PatchworkError::parse_toml("mod Cargo.toml", &manifest_path, source))?;
    let info =
        manifest
            .package
            .metadata
            .mod_info
            .ok_or_else(|| PatchworkError::MissingModMetadata {
                mod_name: id.to_string(),
                manifest_path,
            })?;
    Ok(ModSummary {
        name: manifest.package.name,
        info,
    })
}

fn provider_for_dependency(mods_folder: &Path, dependency: &str) -> Option<String> {
    let entries = fs::read_dir(mods_folder).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if let Ok(summary) = read_mod_summary(&id, mods_folder) {
            if summary.info.provides.as_deref() == Some(dependency) {
                return Some(id);
            }
        }
    }
    None
}

fn read_modpack_document(source_folder: &Path, id: &str) -> Result<ModpackDocument> {
    let path = source_folder.join(format!("{id}.toml"));
    if !path.is_file() {
        return Err(PatchworkError::ModpackNotFound {
            id: id.to_string(),
            folder: source_folder.to_path_buf(),
        });
    }
    read_modpack_document_from_path(&path)
}

fn read_modpack_document_from_path(path: &Path) -> Result<ModpackDocument> {
    let source = fs::read_to_string(path)
        .map_err(|source| PatchworkError::io("read modpack", path, source))?;
    let modpack: Modpack = toml::from_str(&source)
        .map_err(|source| PatchworkError::parse_toml("modpack", path, source))?;
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("modpack")
        .to_string();
    Ok(ModpackDocument {
        id,
        path: path.to_path_buf(),
        modpack,
    })
}

fn find_modpack_path(id: &str, modpacks_folder: &Path) -> Option<PathBuf> {
    let candidate = modpacks_folder.join(id);
    if candidate.is_file() {
        return Some(candidate);
    }

    let candidate = modpacks_folder.join(format!("{id}.toml"));
    candidate.is_file().then_some(candidate)
}

fn mod_manifest_path(mods_folder: &Path, id: &str) -> PathBuf {
    mods_folder.join(id).join("Cargo.toml")
}

fn format_dependency_ref(dep: &DependencyRef) -> String {
    match dep.kind {
        DependencyRefKind::Modpack => format!("modpack/{}", dep.id),
        DependencyRefKind::Mod => dep.id.clone(),
    }
}

fn diagnostic_from_error(error: PatchworkError) -> DependencyDiagnostic {
    DependencyDiagnostic {
        kind: error.kind().to_string(),
        message: error.to_string(),
    }
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn dedup_refs(values: Vec<DependencyRef>) -> Vec<DependencyRef> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn dedup_diagnostics(values: Vec<DependencyDiagnostic>) -> Vec<DependencyDiagnostic> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert((value.kind.clone(), value.message.clone())))
        .collect()
}
