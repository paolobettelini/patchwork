use crate::error::{PatchworkError, Result};
use crate::model::{CargoManifest, Dependencies, ModInfo, Modpack, is_generated_mod_id};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const MODPACK_PREFIX: &str = "modpack/";

pub struct LoadedMods {
    pub mods: HashMap<String, ModInfo>,
}

#[derive(Default)]
struct ModpackSelection {
    mods: Vec<String>,
}

struct Exclusions {
    effective: HashSet<String>,
    discovered: HashSet<String>,
}

impl Exclusions {
    fn contains(&self, id: &str) -> bool {
        self.effective.contains(id)
    }
    fn insert(&mut self, id: String) {
        self.discovered.insert(id.clone());
        self.effective.insert(id);
    }
}

pub(crate) struct SourcePaths<'a> {
    pub mods_folder: &'a Path,
    pub modpacks_folder: &'a Path,
    pub mods: &'a BTreeMap<String, PathBuf>,
    pub modpacks: &'a BTreeMap<String, PathBuf>,
}

impl SourcePaths<'_> {
    pub fn mod_manifest(&self, id: &str) -> PathBuf {
        self.mods
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.mods_folder.join(id))
            .join("Cargo.toml")
    }

    pub fn modpack_path(&self, id: &str) -> Result<PathBuf> {
        validate_modpack_id(id)?;
        match self.modpacks.get(id) {
            Some(path) => Ok(path.clone()),
            None => resolve_modpack_id(id, self.modpacks_folder),
        }
    }
}

pub fn resolve_modpack_path(modpack: &Path, modpacks_folder: &Path) -> Result<PathBuf> {
    if modpack.exists() {
        return modpack
            .canonicalize()
            .map_err(|source| PatchworkError::io("canonicalize modpack path", modpack, source));
    }

    let modpack_id = modpack
        .to_str()
        .ok_or_else(|| PatchworkError::InvalidUtf8Path {
            context: "modpack argument",
            path: modpack.to_path_buf(),
        })?;
    resolve_modpack_id(modpack_id, modpacks_folder)
}

pub fn load_mods(
    root_modpack: &Path,
    mods_folder: &Path,
    modpacks_folder: &Path,
) -> Result<LoadedMods> {
    load_mods_with_sources(
        root_modpack,
        &SourcePaths {
            mods_folder,
            modpacks_folder,
            mods: &BTreeMap::new(),
            modpacks: &BTreeMap::new(),
        },
    )
}

pub(crate) fn load_mods_with_sources(
    root_modpack: &Path,
    sources: &SourcePaths<'_>,
) -> Result<LoadedMods> {
    let mut previous = HashSet::new();
    let mut seen = HashSet::new();
    loop {
        let mut key = previous.iter().cloned().collect::<Vec<String>>();
        key.sort();
        if !seen.insert(key) {
            return Err(PatchworkError::InvalidModpackMetadata {
                id: root_modpack.display().to_string(),
                manifest_path: root_modpack.to_path_buf(),
                reason:
                    "cyclic exclusions: active modpacks disagree about which modpacks are enabled"
                        .into(),
            });
        }
        let mut ignored = Exclusions {
            effective: previous.clone(),
            discovered: HashSet::new(),
        };
        let result = load_selection(root_modpack, sources, &mut ignored);
        // Re-evaluate from active inputs so an excluded pack cannot leave stale exclusions.
        if ignored.discovered == previous {
            return result;
        }
        previous = ignored.discovered;
    }
}

fn load_selection(
    root_modpack: &Path,
    sources: &SourcePaths<'_>,
    ignored: &mut Exclusions,
) -> Result<LoadedMods> {
    let selection = collect_modpack_selection(root_modpack, sources, ignored)?;
    let mut selected = dedup(
        selection
            .mods
            .into_iter()
            .filter(|name| !ignored.contains(name) && !is_generated_mod_id(name))
            .collect(),
    );
    let mut selected_set = selected.iter().cloned().collect::<HashSet<_>>();

    let mut mods = HashMap::new();
    let mut index = 0;
    while index < selected.len() {
        let mod_name = selected[index].clone();
        index += 1;

        if ignored.contains(&mod_name)
            || is_generated_mod_id(&mod_name)
            || mods.contains_key(&mod_name)
        {
            continue;
        }

        let mut mod_info = read_mod_info(sources, &mod_name)?;
        if let Some(dependency) = mod_info
            .dependencies
            .init
            .iter()
            .chain(&mod_info.dependencies.run)
            .chain(&mod_info.dependencies.ownership)
            .find(|id| id.starts_with(MODPACK_PREFIX) && ignored.contains(id))
        {
            return Err(PatchworkError::MissingDependency {
                dependent_mod: mod_name,
                dependency: dependency.clone(),
            });
        }
        expand_dependencies(
            &mut mod_info.dependencies,
            sources,
            ignored,
            &mut selected,
            &mut selected_set,
        )?;
        if let Some(provided_api) = &mod_info.provides {
            select_mod(provided_api, &ignored, &mut selected, &mut selected_set);
        }
        mods.insert(mod_name, mod_info);
    }

    Ok(LoadedMods { mods })
}

fn collect_modpack_selection(
    root_modpack: &Path,
    sources: &SourcePaths<'_>,
    ignored: &mut Exclusions,
) -> Result<ModpackSelection> {
    let mut selection = ModpackSelection::default();
    let mut stack = Vec::new();
    let mut visited = HashSet::new();
    collect_modpack_tree(
        root_modpack,
        sources,
        &mut stack,
        &mut visited,
        &mut selection,
        ignored,
    )?;

    Ok(selection)
}

fn collect_modpack_tree(
    modpack_path: &Path,
    sources: &SourcePaths<'_>,
    stack: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
    selection: &mut ModpackSelection,
    ignored: &mut Exclusions,
) -> Result<()> {
    let modpack_path = modpack_path
        .canonicalize()
        .map_err(|source| PatchworkError::io("canonicalize modpack path", modpack_path, source))?;

    if let Some(position) = stack.iter().position(|path| path == &modpack_path) {
        let mut cycle = stack[position..].to_vec();
        cycle.push(modpack_path);
        return Err(PatchworkError::ModpackCycle { cycle });
    }

    if visited.contains(&modpack_path) {
        return Ok(());
    }

    stack.push(modpack_path.clone());
    let modpack = read_modpack(&modpack_path)?;

    for excluded in &modpack.ignore {
        if let Some(id) = excluded.strip_prefix(MODPACK_PREFIX) {
            validate_modpack_id(id)?;
        } else {
            validate_mod_name(excluded, "modpack ignore list")?;
        }
        ignored.insert(excluded.clone());
    }

    for child_id in &modpack.modpacks {
        if ignored.contains(&format!("{MODPACK_PREFIX}{child_id}")) {
            continue;
        }
        let child_path = sources.modpack_path(child_id)?;
        collect_modpack_tree(&child_path, sources, stack, visited, selection, ignored)?;
    }

    for mod_name in &modpack.mods {
        if let Some(modpack_id) = mod_name.strip_prefix(MODPACK_PREFIX) {
            if ignored.contains(mod_name) {
                continue;
            }
            let child_path = sources.modpack_path(modpack_id)?;
            collect_modpack_tree(&child_path, sources, stack, visited, selection, ignored)?;
            continue;
        }

        validate_mod_name(mod_name, "modpack mods list")?;
        if !is_generated_mod_id(mod_name) {
            selection.mods.push(mod_name.clone());
        }
    }

    stack.pop();
    visited.insert(modpack_path);
    Ok(())
}

fn expand_dependencies(
    dependencies: &mut Dependencies,
    sources: &SourcePaths<'_>,
    ignored: &mut Exclusions,
    selected: &mut Vec<String>,
    selected_set: &mut HashSet<String>,
) -> Result<()> {
    dependencies.init =
        expand_dependency_list(&dependencies.init, sources, ignored, selected, selected_set)?;
    dependencies.run =
        expand_dependency_list(&dependencies.run, sources, ignored, selected, selected_set)?;
    dependencies.ownership = expand_dependency_list(
        &dependencies.ownership,
        sources,
        ignored,
        selected,
        selected_set,
    )?;

    Ok(())
}

fn expand_dependency_list(
    dependencies: &[String],
    sources: &SourcePaths<'_>,
    ignored: &mut Exclusions,
    selected: &mut Vec<String>,
    selected_set: &mut HashSet<String>,
) -> Result<Vec<String>> {
    let mut expanded = Vec::new();

    for dependency in dependencies {
        if dependency.starts_with(MODPACK_PREFIX) {
            let mods =
                expand_modpack_reference(dependency, sources, ignored, selected, selected_set)?;
            expanded.extend(mods);
        } else if !is_generated_mod_id(dependency) {
            select_mod(dependency, ignored, selected, selected_set);
            expanded.push(dependency.clone());
        }
    }

    Ok(dedup(expanded))
}

fn select_mod(
    mod_name: &str,
    ignored: &Exclusions,
    selected: &mut Vec<String>,
    selected_set: &mut HashSet<String>,
) {
    if !ignored.contains(mod_name)
        && !is_generated_mod_id(mod_name)
        && selected_set.insert(mod_name.to_owned())
    {
        selected.push(mod_name.to_owned());
    }
}

fn expand_modpack_reference(
    reference: &str,
    sources: &SourcePaths<'_>,
    ignored: &mut Exclusions,
    selected: &mut Vec<String>,
    selected_set: &mut HashSet<String>,
) -> Result<Vec<String>> {
    let modpack_id =
        reference
            .strip_prefix(MODPACK_PREFIX)
            .ok_or_else(|| PatchworkError::InvalidModpackId {
                id: reference.to_string(),
                reason: "modpack references must use the 'modpack/' prefix",
            })?;
    validate_modpack_id(modpack_id)?;
    if ignored.contains(reference) {
        return Ok(Vec::new());
    }
    let modpack_path = sources.modpack_path(modpack_id)?;
    let dependency_selection = collect_modpack_selection(&modpack_path, sources, ignored)?;

    let mut expanded = Vec::new();
    for mod_name in dependency_selection.mods {
        if is_generated_mod_id(&mod_name) {
            continue;
        }

        if ignored.contains(&mod_name) {
            continue;
        }

        if selected_set.insert(mod_name.clone()) {
            selected.push(mod_name.clone());
        }
        expanded.push(mod_name);
    }

    Ok(dedup(expanded))
}

fn resolve_modpack_id(modpack_id: &str, modpacks_folder: &Path) -> Result<PathBuf> {
    validate_modpack_id(modpack_id)?;

    let candidate = modpacks_folder.join(modpack_id);
    if candidate.exists() {
        return candidate
            .canonicalize()
            .map_err(|source| PatchworkError::io("canonicalize modpack path", &candidate, source));
    }

    let candidate = modpacks_folder.join(format!("{modpack_id}.toml"));
    if candidate.exists() {
        return candidate
            .canonicalize()
            .map_err(|source| PatchworkError::io("canonicalize modpack path", &candidate, source));
    }

    Err(PatchworkError::ModpackNotFound {
        id: modpack_id.to_string(),
        folder: modpacks_folder.to_path_buf(),
    })
}

fn validate_modpack_id(modpack_id: &str) -> Result<()> {
    if modpack_id.contains('/') || modpack_id.contains('\\') {
        return Err(PatchworkError::InvalidModpackId {
            id: modpack_id.to_string(),
            reason: "path separators are not allowed",
        });
    }

    Ok(())
}

fn read_modpack(path: &Path) -> Result<Modpack> {
    let source = fs::read_to_string(path)
        .map_err(|source| PatchworkError::io("read modpack", path, source))?;
    toml::from_str(&source).map_err(|source| PatchworkError::parse_toml("modpack", path, source))
}

fn read_mod_info(sources: &SourcePaths<'_>, mod_name: &str) -> Result<ModInfo> {
    validate_mod_name(mod_name, "selected mod list")?;
    let manifest_path = sources.mod_manifest(mod_name);

    if !manifest_path.exists() {
        return Err(PatchworkError::MissingModManifest {
            mod_name: mod_name.to_string(),
            manifest_path,
        });
    }

    let manifest_source = fs::read_to_string(&manifest_path)
        .map_err(|source| PatchworkError::io("read mod Cargo.toml", &manifest_path, source))?;
    let manifest: CargoManifest = toml::from_str(&manifest_source)
        .map_err(|source| PatchworkError::parse_toml("mod Cargo.toml", &manifest_path, source))?;

    let mod_info =
        manifest
            .package
            .metadata
            .mod_info
            .ok_or_else(|| PatchworkError::MissingModMetadata {
                mod_name: mod_name.to_string(),
                manifest_path: manifest_path.clone(),
            })?;
    mod_info.validate(mod_name, &manifest_path)?;
    Ok(mod_info)
}

fn validate_mod_name(mod_name: &str, context: &'static str) -> Result<()> {
    if mod_name.contains('/') || mod_name.contains('\\') {
        return Err(PatchworkError::InvalidModName {
            name: mod_name.to_string(),
            context,
        });
    }

    Ok(())
}

fn dedup(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}
