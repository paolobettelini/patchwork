use crate::error::{PatchworkError, Result};
use crate::model::{CargoManifest, Dependencies, ModInfo, Modpack};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const MODPACK_PREFIX: &str = "modpack/";

pub struct LoadedMods {
    pub mods: HashMap<String, ModInfo>,
}

#[derive(Default)]
struct ModpackSelection {
    mods: Vec<String>,
    ignore: HashSet<String>,
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
    let selection = collect_modpack_selection(root_modpack, modpacks_folder)?;
    let mut ignored = selection.ignore;
    let mut selected = dedup(
        selection
            .mods
            .into_iter()
            .filter(|name| !ignored.contains(name))
            .collect(),
    );
    let mut selected_set = selected.iter().cloned().collect::<HashSet<_>>();

    let mut mods = HashMap::new();
    let mut index = 0;
    while index < selected.len() {
        let mod_name = selected[index].clone();
        index += 1;

        if ignored.contains(&mod_name) || mods.contains_key(&mod_name) {
            continue;
        }

        let mut mod_info = read_mod_info(mods_folder, &mod_name)?;
        expand_dependencies(
            &mut mod_info.dependencies,
            modpacks_folder,
            &mut ignored,
            &mut selected,
            &mut selected_set,
        )?;
        mods.insert(mod_name, mod_info);
    }

    Ok(LoadedMods { mods })
}

fn collect_modpack_selection(
    root_modpack: &Path,
    modpacks_folder: &Path,
) -> Result<ModpackSelection> {
    let mut selection = ModpackSelection::default();
    let mut stack = Vec::new();
    let mut visited = HashSet::new();
    collect_modpack_tree(
        root_modpack,
        modpacks_folder,
        &mut stack,
        &mut visited,
        &mut selection,
    )?;

    Ok(selection)
}

fn collect_modpack_tree(
    modpack_path: &Path,
    modpacks_folder: &Path,
    stack: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
    selection: &mut ModpackSelection,
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

    for child_id in &modpack.modpacks {
        let child_path = resolve_modpack_id(child_id, modpacks_folder)?;
        collect_modpack_tree(&child_path, modpacks_folder, stack, visited, selection)?;
    }

    for mod_name in &modpack.mods {
        if let Some(modpack_id) = mod_name.strip_prefix(MODPACK_PREFIX) {
            let child_path = resolve_modpack_id(modpack_id, modpacks_folder)?;
            collect_modpack_tree(&child_path, modpacks_folder, stack, visited, selection)?;
            continue;
        }

        validate_mod_name(mod_name, "modpack mods list")?;
        selection.mods.push(mod_name.clone());
    }

    for ignored_mod in &modpack.ignore {
        validate_mod_name(ignored_mod, "modpack ignore list")?;
        selection.ignore.insert(ignored_mod.clone());
    }

    stack.pop();
    visited.insert(modpack_path);
    Ok(())
}

fn expand_dependencies(
    dependencies: &mut Dependencies,
    modpacks_folder: &Path,
    ignored: &mut HashSet<String>,
    selected: &mut Vec<String>,
    selected_set: &mut HashSet<String>,
) -> Result<()> {
    dependencies.init = expand_dependency_list(
        &dependencies.init,
        modpacks_folder,
        ignored,
        selected,
        selected_set,
    )?;
    dependencies.run = expand_dependency_list(
        &dependencies.run,
        modpacks_folder,
        ignored,
        selected,
        selected_set,
    )?;
    dependencies.ownership = expand_dependency_list(
        &dependencies.ownership,
        modpacks_folder,
        ignored,
        selected,
        selected_set,
    )?;

    Ok(())
}

fn expand_dependency_list(
    dependencies: &[String],
    modpacks_folder: &Path,
    ignored: &mut HashSet<String>,
    selected: &mut Vec<String>,
    selected_set: &mut HashSet<String>,
) -> Result<Vec<String>> {
    let mut expanded = Vec::new();

    for dependency in dependencies {
        if dependency.starts_with(MODPACK_PREFIX) {
            let mods = expand_modpack_reference(
                dependency,
                modpacks_folder,
                ignored,
                selected,
                selected_set,
            )?;
            expanded.extend(mods);
        } else {
            expanded.push(dependency.clone());
        }
    }

    Ok(dedup(expanded))
}

fn expand_modpack_reference(
    reference: &str,
    modpacks_folder: &Path,
    ignored: &mut HashSet<String>,
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
    let modpack_path = resolve_modpack_id(modpack_id, modpacks_folder)?;
    let dependency_selection = collect_modpack_selection(&modpack_path, modpacks_folder)?;

    ignored.extend(dependency_selection.ignore);

    let mut expanded = Vec::new();
    for mod_name in dependency_selection.mods {
        if ignored.contains(&mod_name) {
            selected_set.remove(&mod_name);
            selected.retain(|selected_mod| selected_mod != &mod_name);
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
    if modpack_id.contains('/') || modpack_id.contains('\\') {
        return Err(PatchworkError::InvalidModpackId {
            id: modpack_id.to_string(),
            reason: "path separators are not allowed",
        });
    }

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

fn read_modpack(path: &Path) -> Result<Modpack> {
    let source = fs::read_to_string(path)
        .map_err(|source| PatchworkError::io("read modpack", path, source))?;
    toml::from_str(&source).map_err(|source| PatchworkError::parse_toml("modpack", path, source))
}

fn read_mod_info(mods_folder: &Path, mod_name: &str) -> Result<ModInfo> {
    validate_mod_name(mod_name, "selected mod list")?;
    let manifest_path = mods_folder.join(mod_name).join("Cargo.toml");

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
