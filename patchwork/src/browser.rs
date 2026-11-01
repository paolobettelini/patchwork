use crate::error::{PatchworkError, Result};
use crate::graph;
use crate::model::{CargoManifest, Dependencies, ModInfo, Modpack};
use crate::modpacks;
use crate::modpacks::SourcePaths;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
    pub version: String,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDependency {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub found: bool,
    pub ignored: bool,
    pub local: bool,
    pub required_by: Vec<String>,
    pub toggle_blocked_reason: Option<String>,
    pub version: String,
    pub dependencies: Vec<String>,
    pub explicitly_ignored: bool,
    pub icon_data_url: Option<String>,
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

/// Inspect the full dependency graph, retaining excluded and missing entries.
pub fn inspect_profile_dependencies(
    root_modpack: &Path,
    mods_folder: &Path,
    modpacks_folder: &Path,
) -> Result<Vec<ProfileDependency>> {
    inspect_profile_dependencies_with_sources(
        root_modpack,
        mods_folder,
        modpacks_folder,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

/// Local mod values are crate directories; local modpack values are TOML files.
pub fn inspect_profile_dependencies_with_sources(
    root_modpack: &Path,
    mods_folder: &Path,
    modpacks_folder: &Path,
    mods: &BTreeMap<String, PathBuf>,
    modpacks: &BTreeMap<String, PathBuf>,
) -> Result<Vec<ProfileDependency>> {
    let sources = SourcePaths {
        mods_folder,
        modpacks_folder,
        mods,
        modpacks,
    };
    let root = read_modpack_document_from_path(root_modpack)?;
    let root_ref = format!("{MODPACK_PREFIX}{}", root.id);
    let roots = explicit_refs_for_modpack(&root.modpack);
    let mut ignored = root.modpack.ignore.iter().cloned().collect::<HashSet<_>>();
    let mut pending = roots
        .iter()
        .cloned()
        .map(|dep| (Some(root_ref.clone()), dep))
        .collect::<VecDeque<_>>();
    pending.extend(
        root.modpack
            .ignore
            .iter()
            .map(|id| (None, parse_dependency_ref(id))),
    );
    let mut indexes = HashMap::<DependencyRef, usize>::new();
    let mut entries = Vec::<ProfileDependency>::new();
    let mut children = HashMap::<DependencyRef, Vec<DependencyRef>>::new();
    let mut exclusions = HashMap::<DependencyRef, Vec<String>>::new();

    while let Some((parent, dep)) = pending.pop_front() {
        if let Some(&index) = indexes.get(&dep) {
            if let Some(parent) = parent {
                if !entries[index].required_by.contains(&parent) {
                    entries[index].required_by.push(parent);
                }
            }
            continue;
        }
        let (entry, refs, ignores, local) = match dep.kind {
            DependencyRefKind::Modpack => {
                let document = sources
                    .modpack_path(&dep.id)
                    .ok()
                    .and_then(|path| read_modpack_document_from_path(&path).ok());
                let (refs, ignores) = document
                    .as_ref()
                    .map(|document| {
                        (
                            explicit_refs_for_modpack(&document.modpack),
                            document.modpack.ignore.clone(),
                        )
                    })
                    .unwrap_or_default();
                (
                    modpack_entry(&dep.id, &sources, false),
                    refs,
                    ignores,
                    modpacks.contains_key(&dep.id),
                )
            }
            DependencyRefKind::Mod => {
                let refs = read_mod_summary(&dep.id, &sources)
                    .ok()
                    .map(|summary| refs_for_mod(&summary.info))
                    .unwrap_or_default();
                (
                    mod_entry(&dep.id, &sources, false),
                    refs,
                    Vec::new(),
                    mods.contains_key(&dep.id),
                )
            }
        };
        indexes.insert(dep.clone(), entries.len());
        entries.push(ProfileDependency {
            version: match dep.kind {
                DependencyRefKind::Mod => read_mod_summary(&dep.id, &sources)
                    .map(|s| s.version)
                    .unwrap_or_default(),
                DependencyRefKind::Modpack => sources
                    .modpack_path(&dep.id)
                    .ok()
                    .and_then(|p| read_modpack_document_from_path(&p).ok())
                    .map(|d| d.modpack.version)
                    .unwrap_or_default(),
            },
            dependencies: refs.iter().map(format_dependency_ref).collect(),
            explicitly_ignored: root.modpack.ignore.contains(&format_dependency_ref(&dep)),
            icon_data_url: None,
            id: dep.id.clone(),
            name: entry.name,
            kind: match dep.kind {
                DependencyRefKind::Mod => "mod",
                DependencyRefKind::Modpack => "modpack",
            }
            .to_owned(),
            found: entry.found,
            ignored: false,
            local,
            required_by: parent.into_iter().collect(),
            toggle_blocked_reason: None,
        });
        pending.extend(
            refs.iter()
                .cloned()
                .map(|child| (Some(format_dependency_ref(&dep)), child)),
        );
        pending.extend(ignores.iter().map(|id| (None, parse_dependency_ref(id))));
        children.insert(dep.clone(), refs);
        exclusions.insert(dep, ignores);
    }

    // Only active packs contribute exclusions. A second path can keep a shared mod active.
    let root_ignored = ignored.clone();
    let mut states = HashSet::new();
    let active = loop {
        let previous = ignored.clone();
        let mut key = previous.iter().cloned().collect::<Vec<_>>();
        key.sort();
        if !states.insert(key) {
            return Err(PatchworkError::InvalidModpackMetadata {
                id: root.id.clone(),
                manifest_path: root_modpack.to_path_buf(),
                reason: "cyclic modpack exclusions; remove one of the conflicting exclusions"
                    .into(),
            });
        }
        let mut discovered = root_ignored.clone();
        let mut active = HashSet::new();
        let mut pending = roots.clone();
        while let Some(dep) = pending.pop() {
            if ignored.contains(&format_dependency_ref(&dep)) || !active.insert(dep.clone()) {
                continue;
            }
            if let Some(excludes) = exclusions.get(&dep) {
                discovered.extend(excludes.iter().cloned());
                ignored.extend(excludes.iter().cloned());
            }
            if let Some(refs) = children.get(&dep) {
                pending.extend(refs.iter().cloned());
            }
        }
        if previous == discovered {
            break active;
        }
        ignored = discovered;
    };
    for (dep, index) in indexes {
        entries[index].ignored =
            ignored.contains(&format_dependency_ref(&dep)) || !active.contains(&dep);
        if entries[index].ignored && !root_ignored.contains(&format_dependency_ref(&dep)) {
            entries[index].toggle_blocked_reason = Some(
                if ignored.contains(&format_dependency_ref(&dep)) {
                    "Excluded by an imported modpack. Change that modpack's exclusion or disable the importing modpack.".into()
                } else {
                    "Inactive because its parent dependency is disabled. Enable the parent first."
                        .into()
                },
            );
        }
        entries[index].required_by.sort();
    }
    Ok(entries)
}

pub fn inspect_dependency_page(
    target: DependencyTarget<'_>,
    mods_folder: &Path,
    modpacks_folder: &Path,
    profiles_folder: &Path,
) -> Result<DependencyPage> {
    inspect_dependency_page_with_sources(
        target,
        mods_folder,
        modpacks_folder,
        profiles_folder,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

pub fn inspect_dependency_page_with_sources(
    target: DependencyTarget<'_>,
    mods_folder: &Path,
    modpacks_folder: &Path,
    profiles_folder: &Path,
    local_mods: &BTreeMap<String, PathBuf>,
    local_modpacks: &BTreeMap<String, PathBuf>,
) -> Result<DependencyPage> {
    let sources = SourcePaths {
        mods_folder,
        modpacks_folder,
        mods: local_mods,
        modpacks: local_modpacks,
    };
    match target {
        DependencyTarget::Profile { id } => inspect_modpack_page(
            DependencyPageKind::Profile,
            id,
            profiles_folder,
            &sources,
            true,
        ),
        DependencyTarget::Modpack { id } => inspect_modpack_page(
            DependencyPageKind::Modpack,
            id,
            modpacks_folder,
            &sources,
            false,
        ),
        DependencyTarget::Mod { id } => inspect_mod_page(id, &sources),
    }
}

fn inspect_modpack_page(
    kind: DependencyPageKind,
    id: &str,
    source_folder: &Path,
    sources: &SourcePaths<'_>,
    editable_profile: bool,
) -> Result<DependencyPage> {
    let document = if kind == DependencyPageKind::Modpack {
        read_modpack_document_from_path(&sources.modpack_path(id)?)?
    } else {
        read_modpack_document(source_folder, id)?
    };
    let ignored = document
        .modpack
        .ignore
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let explicit = explicit_refs_for_modpack(&document.modpack);

    let mut diagnostics = validate_modpack_graph(&document.path, sources);
    let distinct_dependency_count = collect_distinct_count(&explicit, sources, &mut diagnostics);

    let modpacks = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Modpack)
        .map(|dep| {
            modpack_entry(
                &dep.id,
                sources,
                ignored.contains(&format_dependency_ref(dep)),
            )
        })
        .collect();
    let mods = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Mod)
        .map(|dep| mod_entry(&dep.id, sources, ignored.contains(&dep.id)))
        .collect();

    Ok(DependencyPage {
        kind,
        id: id.to_owned(),
        name: non_empty_or(&document.modpack.name, id),
        description: non_empty_or(
            &document.modpack.description,
            "No description provided yet.",
        ),
        version: document.modpack.version.clone(),
        editable_profile,
        distinct_dependency_count,
        modpacks,
        mods,
        diagnostics: dedup_diagnostics(diagnostics),
    })
}

fn inspect_mod_page(id: &str, sources: &SourcePaths<'_>) -> Result<DependencyPage> {
    let summary = read_mod_summary(id, sources)?;
    let explicit = refs_for_mod(&summary.info);
    let mut diagnostics = validate_mod_dependencies(id, &summary.info, sources);
    let distinct_dependency_count = collect_distinct_count(&explicit, sources, &mut diagnostics);

    let modpacks = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Modpack)
        .map(|dep| modpack_entry(&dep.id, sources, false))
        .collect();
    let mods = explicit
        .iter()
        .filter(|dep| dep.kind == DependencyRefKind::Mod)
        .map(|dep| mod_entry(&dep.id, sources, false))
        .collect();

    Ok(DependencyPage {
        kind: DependencyPageKind::Mod,
        id: id.to_string(),
        name: summary.name,
        description: format!("Patchwork mod crate '{}'.", id),
        version: summary.version,
        editable_profile: false,
        distinct_dependency_count,
        modpacks,
        mods,
        diagnostics: dedup_diagnostics(diagnostics),
    })
}

fn validate_modpack_graph(
    root_path: &Path,
    sources: &SourcePaths<'_>,
) -> Vec<DependencyDiagnostic> {
    match modpacks::load_mods_with_sources(root_path, sources)
        .and_then(|loaded| graph::resolve(loaded.mods).map(|_| ()))
    {
        Ok(()) => Vec::new(),
        Err(error) => vec![diagnostic_from_error(error)],
    }
}

fn validate_mod_dependencies(
    mod_id: &str,
    info: &ModInfo,
    sources: &SourcePaths<'_>,
) -> Vec<DependencyDiagnostic> {
    let mut diagnostics = Vec::new();
    for dep in refs_for_mod(info) {
        match dep.kind {
            DependencyRefKind::Modpack => {
                if dep.id == mod_id {
                    diagnostics.push(DependencyDiagnostic {
                        kind: "self_dependency".to_string(),
                        message: format!("mod '{mod_id}' references itself as a modpack"),
                    });
                }
                if sources.modpack_path(&dep.id).is_err() {
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
                } else if !sources.mod_manifest(&dep.id).is_file()
                    && provider_for_dependency(sources, &dep.id).is_none()
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
    sources: &SourcePaths<'_>,
    diagnostics: &mut Vec<DependencyDiagnostic>,
) -> usize {
    let mut seen = HashSet::new();
    let mut visiting = Vec::new();
    for dep in roots {
        collect_dependency_ref(dep, sources, &mut seen, &mut visiting, diagnostics);
    }
    seen.len()
}

fn collect_dependency_ref(
    dep: &DependencyRef,
    sources: &SourcePaths<'_>,
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
            if let Ok(path) = sources.modpack_path(&dep.id) {
                match read_modpack_document_from_path(&path) {
                    Ok(document) => {
                        for child in explicit_refs_for_modpack(&document.modpack) {
                            collect_dependency_ref(&child, sources, seen, visiting, diagnostics);
                        }
                    }
                    Err(error) => diagnostics.push(diagnostic_from_error(error)),
                }
            }
        }
        DependencyRefKind::Mod => {
            if let Ok(summary) = read_mod_summary(&dep.id, sources) {
                for child in refs_for_mod(&summary.info) {
                    collect_dependency_ref(&child, sources, seen, visiting, diagnostics);
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

fn refs_for_mod(info: &ModInfo) -> Vec<DependencyRef> {
    let mut refs = explicit_refs_for_mod(&info.dependencies);
    if let Some(api) = &info.provides {
        refs.push(parse_dependency_ref(api));
    }
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

fn modpack_entry(id: &str, sources: &SourcePaths<'_>, ignored: bool) -> DependencyEntry {
    match sources
        .modpack_path(id)
        .ok()
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

fn mod_entry(id: &str, sources: &SourcePaths<'_>, ignored: bool) -> DependencyEntry {
    match read_mod_summary(id, sources) {
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
    version: String,
    info: ModInfo,
}

fn read_mod_summary(id: &str, sources: &SourcePaths<'_>) -> Result<ModSummary> {
    let manifest_path = sources.mod_manifest(id);
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
    let version = toml::from_str::<toml::Value>(&source)
        .ok()
        .and_then(|document| local_package_version(&document, &manifest_path))
        .unwrap_or_else(|| "Unknown".to_owned());
    let info =
        manifest
            .package
            .metadata
            .mod_info
            .ok_or_else(|| PatchworkError::MissingModMetadata {
                mod_name: id.to_string(),
                manifest_path: manifest_path.clone(),
            })?;
    info.validate(id, &manifest_path)?;
    Ok(ModSummary {
        name: manifest.package.name,
        version,
        info,
    })
}

fn local_package_version(document: &toml::Value, manifest_path: &Path) -> Option<String> {
    let version = document.get("package")?.get("version")?;
    if let Some(version) = version.as_str() {
        return Some(version.to_owned());
    }
    if !version
        .as_table()
        .and_then(|version| version.get("workspace"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }

    for directory in manifest_path.parent()?.ancestors() {
        let workspace_path = directory.join("Cargo.toml");
        let workspace = if workspace_path == manifest_path {
            document.clone()
        } else {
            let Ok(source) = fs::read_to_string(&workspace_path) else {
                continue;
            };
            let Ok(workspace) = toml::from_str::<toml::Value>(&source) else {
                continue;
            };
            workspace
        };
        if let Some(version) = workspace
            .get("workspace")
            .and_then(|workspace| workspace.get("package"))
            .and_then(|package| package.get("version"))
            .and_then(toml::Value::as_str)
        {
            return Some(version.to_owned());
        }
    }
    None
}

fn provider_for_dependency(sources: &SourcePaths<'_>, dependency: &str) -> Option<String> {
    let mut ids = sources.mods.keys().cloned().collect::<HashSet<_>>();
    if let Ok(entries) = fs::read_dir(sources.mods_folder) {
        ids.extend(
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned()),
        );
    }
    for id in ids {
        if let Ok(summary) = read_mod_summary(&id, sources) {
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
