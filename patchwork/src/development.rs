//! Explicit, trusted local development inputs. Never read these from modpack metadata.
use crate::{ModInfo, Modpack, PatchworkError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

const MARKER: &str = "# Patchwork development overlay\n";
const LIMIT: usize = 100_000;
mod discovery;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DevelopmentConfig {
    pub enabled: bool,
    pub checkouts: Vec<DevelopmentCheckout>,
    pub exclude: Vec<String>,
    /// Canonical discovery roots retained only in a resolved snapshot for overlap checks.
    #[serde(skip)]
    pub checkout_roots: Vec<PathBuf>,
    pub mods: BTreeMap<String, PathBuf>,
    pub modpacks: BTreeMap<String, PathBuf>,
    pub libraries: Vec<DevelopmentLibrarySource>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DevelopmentCheckout {
    pub path: PathBuf,
    /// Empty means discover mods/modpacks only; no Git library patches.
    pub git: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentLibrarySource {
    pub git: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PreparedDevelopment {
    pub mods_folder: PathBuf,
    pub modpacks_folder: PathBuf,
    pub cargo_config: Option<PathBuf>,
}

impl DevelopmentConfig {
    /// Load an explicitly selected file, resolving paths relative to that file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = canonical(path.as_ref())?;
        let config: Self = toml::from_str(&read(&path)?)
            .map_err(|e| PatchworkError::parse_toml("development configuration", &path, e))?;
        config.resolved(path.parent().unwrap())
    }

    /// Structural validation, also suitable for configurations being edited in a UI.
    pub fn validate(&self) -> Result<()> {
        for checkout in &self.checkouts {
            if checkout.path.as_os_str().is_empty() {
                return Err(invalid("checkout path must not be empty"));
            }
        }
        for id in &self.exclude {
            validate_id(
                id.strip_prefix("modpack/")
                    .or_else(|| id.strip_prefix("library/"))
                    .unwrap_or(id),
            )?;
        }
        for (id, path) in self.mods.iter().chain(&self.modpacks) {
            validate_id(id)?;
            if path.as_os_str().is_empty() {
                return Err(invalid(format!("empty development path for '{id}'")));
            }
        }
        for library in &self.libraries {
            if library.git.trim().is_empty() || library.path.as_os_str().is_empty() {
                return Err(invalid(
                    "development libraries require a Git source and checkout path",
                ));
            }
        }
        Ok(())
    }

    /// Inspect local inputs without materializing files. Disabled configurations need no checkouts.
    pub fn resolved(&self, base: &Path) -> Result<Self> {
        self.validate()?;
        let mut config = self.clone();
        if !config.enabled {
            return Ok(config);
        }
        // Resolve a fresh snapshot each operation, never persist discovered IDs.
        let mut discovered_mods = BTreeMap::new();
        let mut discovered_packs = BTreeMap::new();
        for checkout in &config.checkouts {
            let root = canonical(&base.join(&checkout.path))?;
            config.checkout_roots.push(root.clone());
            discovery::scan(&root, &config, &mut discovered_mods, &mut discovered_packs)?;
            if !checkout.git.trim().is_empty() {
                config.libraries.push(DevelopmentLibrarySource {
                    git: checkout.git.trim().into(),
                    path: root,
                });
            }
        }
        discovered_mods.extend(config.mods);
        discovered_packs.extend(config.modpacks);
        config.mods = discovered_mods;
        config.modpacks = discovered_packs;
        config.checkouts.clear();
        for (id, path) in &mut config.mods {
            if crate::is_generated_mod_id(id) {
                return Err(invalid(format!(
                    "generated mod '{id}' is a codegen output, not a development override"
                )));
            }
            *path = canonical(&base.join(&*path))?;
            let manifest = manifest(path)?;
            if package_name(&manifest)? != *id {
                return Err(invalid(format!(
                    "mod '{id}' does not match package.name in {}",
                    path.display()
                )));
            }
            let info = manifest
                .get("package")
                .and_then(|p| p.get("metadata"))
                .and_then(|m| m.get("mod"))
                .ok_or_else(|| invalid(format!("mod '{id}' has no [package.metadata.mod]")))?;
            let info: ModInfo = info.clone().try_into().map_err(|e| {
                PatchworkError::parse_toml("mod metadata", path.join("Cargo.toml"), e)
            })?;
            info.validate(id, &path.join("Cargo.toml"))?;
        }
        for path in config.modpacks.values_mut() {
            *path = canonical(&base.join(&*path))?;
            let _: Modpack = toml::from_str(&read(path)?)
                .map_err(|e| PatchworkError::parse_toml("development modpack", &*path, e))?;
        }
        for library in &mut config.libraries {
            library.path = canonical(&base.join(&library.path))?;
            if !library.path.is_dir() {
                return Err(invalid(format!(
                    "library checkout is not a directory: {}",
                    library.path.display()
                )));
            }
        }
        library_packages(&config)?;
        Ok(config)
    }
}

/// Materialize a complete local/cache overlay into a dedicated per-profile directory.
pub fn prepare_development(
    config: &DevelopmentConfig,
    config_base: &Path,
    cache_mods: &Path,
    cache_modpacks: &Path,
    output_dir: &Path,
) -> Result<PreparedDevelopment> {
    prepare(
        config,
        config_base,
        cache_mods,
        cache_modpacks,
        output_dir,
        None,
    )
}

/// Materialize only the selected profile, its generators, and required Cargo packages.
pub fn prepare_development_for_modpack(
    config: &DevelopmentConfig,
    config_base: &Path,
    cache_mods: &Path,
    cache_modpacks: &Path,
    output_dir: &Path,
    modpack: &Path,
) -> Result<PreparedDevelopment> {
    prepare(
        config,
        config_base,
        cache_mods,
        cache_modpacks,
        output_dir,
        Some(modpack),
    )
}

fn prepare(
    config: &DevelopmentConfig,
    base: &Path,
    cache_mods: &Path,
    cache_modpacks: &Path,
    output: &Path,
    root: Option<&Path>,
) -> Result<PreparedDevelopment> {
    if !config.enabled {
        return Ok(PreparedDevelopment {
            mods_folder: cache_mods.into(),
            modpacks_folder: cache_modpacks.into(),
            cargo_config: None,
        });
    }
    let config = config.resolved(base)?;
    let mut mods = cached_entries(cache_mods, true)?;
    mods.extend(config.mods.clone());
    let mut modpacks = cached_entries(cache_modpacks, false)?;
    modpacks.extend(config.modpacks.clone());
    let libraries = library_packages(&config)?;
    let output = absolute_destination(output)?;
    for source in mods
        .values()
        .chain(modpacks.values())
        .chain(libraries.values())
        .chain(config.libraries.iter().map(|l| &l.path))
        .chain(config.checkout_roots.iter())
    {
        if source.starts_with(&output) || output.starts_with(source) {
            return Err(invalid(format!(
                "development output overlaps input: {}",
                source.display()
            )));
        }
    }
    for cache in [cache_mods, cache_modpacks] {
        if cache.exists() {
            let cache = canonical(cache)?;
            if output.starts_with(&cache) || cache.starts_with(&output) {
                return Err(invalid(
                    "development output must not overlap downloaded caches",
                ));
            }
        }
    }
    if output.exists() {
        check_tree(&output, &mut 0, 0)?;
        if fs::read_dir(&output)
            .map_err(|e| io_error(&output, e))?
            .next()
            .is_some()
            && fs::read_to_string(output.join(".patchwork-development"))
                .ok()
                .as_deref()
                != Some(MARKER)
        {
            return Err(invalid(
                "development output must be empty or a Patchwork-owned development directory",
            ));
        }
    }
    let selected = match root {
        Some(root) => selected_sources(root, &mods, &modpacks, cache_mods, cache_modpacks)?,
        None => mods.keys().cloned().collect(),
    };
    let mut materializer = Materializer {
        output: output.clone(),
        mods,
        libraries,
        destinations: BTreeMap::new(),
        patches: BTreeMap::new(),
        wanted: BTreeSet::new(),
        count: 0,
        library_sources: config.libraries.clone(),
        unattributed: BTreeMap::new(),
    };
    materializer.directory(&output)?;
    materializer.file(&output.join(".patchwork-development"), MARKER.as_bytes())?;
    materializer.directory(&output.join("mods"))?;
    materializer.directory(&output.join("modpacks"))?;
    for id in selected {
        if let Some(source) = materializer.mods.get(&id).cloned() {
            materializer.package(&source, Some(&id))?;
        } else {
            return Err(invalid(format!(
                "selected development mod or generator '{id}' is unavailable"
            )));
        }
    }
    // Explicit checkout links apply even to libraries used only by remote transitive dependencies.
    for ((git, name), source) in materializer.libraries.clone() {
        let destination = materializer.package(&source, None)?;
        materializer.patch(&git, &name, &destination)?;
    }
    for (id, source) in modpacks {
        materializer.file(
            &output.join("modpacks").join(format!("{id}.toml")),
            &fs::read(&source).map_err(|e| io_error(&source, e))?,
        )?;
    }
    let mut patch = Table::new();
    for ((git, name), path) in &materializer.patches {
        let table = patch
            .entry(git.clone())
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .unwrap();
        table.insert(
            name.clone(),
            Value::Table(Table::from_iter([("path".into(), path_value(path))])),
        );
    }
    let document = Value::Table(Table::from_iter([("patch".into(), Value::Table(patch))]));
    let cargo_config = output.join("mods/.cargo/config.toml");
    materializer.file(
        &cargo_config,
        format!("{MARKER}{}", serialize(&document)?).as_bytes(),
    )?;
    prune(&output, &materializer.wanted)?;
    Ok(PreparedDevelopment {
        mods_folder: output.join("mods"),
        modpacks_folder: output.join("modpacks"),
        cargo_config: Some(cargo_config),
    })
}

/// Only generated development configs are propagated into composed projects.
pub(crate) fn cargo_config(mods: &Path) -> Result<Option<String>> {
    let path = mods.join(".cargo/config.toml");
    if !path.exists() {
        return Ok(None);
    }
    let source = read(&path)?;
    Ok(source.starts_with(MARKER).then_some(source))
}

struct Materializer {
    output: PathBuf,
    mods: BTreeMap<String, PathBuf>,
    libraries: BTreeMap<(String, String), PathBuf>,
    destinations: BTreeMap<PathBuf, PathBuf>,
    patches: BTreeMap<(String, String), PathBuf>,
    wanted: BTreeSet<PathBuf>,
    count: usize,
    library_sources: Vec<DevelopmentLibrarySource>,
    unattributed: BTreeMap<String, String>,
}

impl Materializer {
    fn directory(&mut self, path: &Path) -> Result<()> {
        if !path.starts_with(&self.output) {
            return Err(invalid("materialized path escaped development output"));
        }
        if self.wanted.insert(path.into()) {
            self.count += 1;
            if self.count > LIMIT {
                return Err(invalid("development overlay exceeds 100000 entries"));
            }
        }
        if path != self.output {
            self.directory(path.parent().unwrap())?;
        }
        if path.is_file() {
            fs::remove_file(path).map_err(|e| io_error(path, e))?;
        }
        fs::create_dir_all(path).map_err(|e| io_error(path, e))
    }

    fn file(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.directory(path.parent().unwrap())?;
        self.wanted.insert(path.into());
        self.count += 1;
        if self.count > LIMIT {
            return Err(invalid("development overlay exceeds 100000 entries"));
        }
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|e| io_error(path, e))?;
        }
        if fs::read(path).ok().as_deref() != Some(bytes) {
            fs::write(path, bytes).map_err(|e| io_error(path, e))?;
        }
        Ok(())
    }

    fn copy(&mut self, source: &Path, destination: &Path, depth: usize) -> Result<()> {
        if depth > 128 {
            return Err(invalid(
                "development source nesting exceeds 128 directories",
            ));
        }
        self.directory(destination)?;
        for entry in entries(source)? {
            let path = entry.path();
            let name = entry.file_name();
            let kind = entry.file_type().map_err(|e| io_error(&path, e))?;
            if !kind.is_dir() && !kind.is_file() {
                return Err(invalid(format!(
                    "symlinks and special files are unsupported: {}",
                    path.display()
                )));
            }
            if skipped(&name.to_string_lossy()) || (depth == 0 && name == "Cargo.toml") {
                continue;
            }
            let target = destination.join(name);
            if kind.is_dir() {
                self.copy(&path, &target, depth + 1)?;
            } else if kind.is_file() {
                self.file(&target, &fs::read(&path).map_err(|e| io_error(&path, e))?)?;
                let permissions = entry
                    .metadata()
                    .map_err(|e| io_error(&path, e))?
                    .permissions();
                fs::set_permissions(&target, permissions).map_err(|e| io_error(&target, e))?;
            } else {
                return Err(invalid(format!(
                    "symlinks and special files are unsupported: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn package(&mut self, source: &Path, mod_id: Option<&str>) -> Result<PathBuf> {
        let source = canonical(source)?;
        if source.starts_with(&self.output) || self.output.starts_with(&source) {
            return Err(invalid("path dependency overlaps development output"));
        }
        if let Some(path) = self.destinations.get(&source) {
            return Ok(path.clone());
        }
        let mut doc = manifest(&source)?;
        let name = package_name(&doc)?.to_owned();
        let destination = if let Some(id) = mod_id {
            if name != id {
                return Err(invalid(format!(
                    "cached mod '{id}' has package.name '{name}'"
                )));
            }
            self.output.join("mods").join(id)
        } else {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            source.hash(&mut hash);
            self.output
                .join("packages")
                .join(format!("{name}-{:016x}", hash.finish()))
        };
        self.destinations
            .insert(source.clone(), destination.clone());
        self.copy(&source, &destination, 0)?;
        let workspace = workspace(&source, &doc)?;
        expand_inheritance(&mut doc, &source, workspace.as_ref())?;
        for key in ["readme", "license-file", "build"] {
            if let Some(path) = doc
                .get("package")
                .and_then(|p| p.get(key))
                .and_then(Value::as_str)
            {
                let original = source.join(path);
                let normalized = canonical(&original)?;
                if !normalized.starts_with(&source) {
                    let target = destination
                        .join(".patchwork")
                        .join(key)
                        .join(normalized.file_name().unwrap());
                    self.file(
                        &target,
                        &fs::read(&normalized).map_err(|e| io_error(&normalized, e))?,
                    )?;
                    doc["package"][key] = path_value(&target);
                }
            }
        }
        for table in cargo_dependency_tables_mut(&mut doc) {
            for (alias, dependency) in table {
                let Some(dependency) = dependency.as_table_mut() else {
                    continue;
                };
                let package = dependency
                    .get("package")
                    .and_then(Value::as_str)
                    .unwrap_or(alias)
                    .to_owned();
                let git = dependency
                    .get("git")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let path = dependency
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(git) = &git {
                    let library = self
                        .libraries
                        .iter()
                        .find(|((source, name), _)| {
                            git_identity(source) == git_identity(git) && name == &package
                        })
                        .map(|(_, path)| path.clone());
                    let local_mod = self.mod_for_git(&package, git)?;
                    if library.is_some() && local_mod.is_some() {
                        return Err(invalid(format!(
                            "ambiguous mod/library override for {git} package '{package}'"
                        )));
                    }
                    if let Some(local) = library.or(local_mod.clone()) {
                        if let Some(requirement) = dependency.get("version").and_then(Value::as_str)
                        {
                            let mut local_manifest = manifest(&local)?;
                            let local_workspace = self::workspace(&local, &local_manifest)?;
                            expand_inheritance(
                                &mut local_manifest,
                                &local,
                                local_workspace.as_ref(),
                            )?;
                            let version = local_manifest
                                .get("package")
                                .and_then(|p| p.get("version"))
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    invalid(format!(
                                        "local package '{package}' has no resolved version"
                                    ))
                                })?;
                            let requirement = semver::VersionReq::parse(requirement)
                                .map_err(|error| invalid(error.to_string()))?;
                            let version = semver::Version::parse(version)
                                .map_err(|error| invalid(error.to_string()))?;
                            if !requirement.matches(&version) {
                                return Err(invalid(format!(
                                    "local package '{package}' version {version} does not satisfy {requirement} required by '{name}'"
                                )));
                            }
                        }
                        let target =
                            self.package(&local, local_mod.as_ref().map(|_| package.as_str()))?;
                        self.patch(git, &package, &target)?;
                    }
                }
                if let Some(path) = path {
                    if crate::is_generated_mod_id(&package) {
                        // Generators receive this staged location as --dev-crate. It may not exist yet.
                        dependency.insert(
                            "path".into(),
                            path_value(&self.output.join("mods").join(&package)),
                        );
                        continue;
                    }
                    let local_mod = self.mods.get(&package).cloned();
                    let original = source.join(path);
                    let local = local_mod.as_deref().unwrap_or(&original);
                    let target =
                        self.package(local, local_mod.as_ref().map(|_| package.as_str()))?;
                    let actual = package_name(&manifest(local)?)?.to_owned();
                    if actual != package {
                        return Err(invalid(format!(
                            "path dependency '{alias}' expects '{package}', found '{actual}'"
                        )));
                    }
                    dependency.insert("path".into(), path_value(&target));
                }
            }
        }
        doc.as_table_mut().unwrap().remove("workspace");
        doc["package"].as_table_mut().unwrap().remove("workspace");
        // Each copied crate is independently buildable, including codegen executables.
        doc.as_table_mut()
            .unwrap()
            .insert("workspace".into(), Value::Table(Table::new()));
        self.file(&destination.join("Cargo.toml"), serialize(&doc)?.as_bytes())?;
        Ok(destination)
    }

    fn mod_for_git(&mut self, name: &str, git: &str) -> Result<Option<PathBuf>> {
        let Some(path) = self.mods.get(name).cloned() else {
            return Ok(None);
        };
        let mut identities = BTreeSet::new();
        for library in &self.library_sources {
            if path.starts_with(&library.path) {
                identities.insert(git_identity(&library.git));
            }
        }
        let mut doc = manifest(&path)?;
        let workspace = workspace(&path, &doc)?;
        expand_inheritance(&mut doc, &path, workspace.as_ref())?;
        if let Some(repository) = doc
            .get("package")
            .and_then(|p| p.get("repository"))
            .and_then(Value::as_str)
        {
            identities.insert(git_identity(repository));
        }
        if identities.is_empty() {
            // Downloaded crate archives may have no repository provenance. A unique observed
            // source is usable; two Git identities for the same ID are never guessed between.
            let identity = git_identity(git);
            if let Some(previous) = self.unattributed.insert(name.into(), identity.clone()) {
                if previous != identity {
                    return Err(invalid(format!(
                        "ambiguous Git sources for mod '{name}'; declare package.repository or link its checkout with a Git source"
                    )));
                }
            }
        } else if !identities.contains(&git_identity(git)) {
            return Ok(None);
        }
        Ok(Some(path))
    }

    fn patch(&mut self, git: &str, name: &str, path: &Path) -> Result<()> {
        let key = (git.to_owned(), name.to_owned());
        if let Some(previous) = self.patches.insert(key, path.into()) {
            if previous != path {
                return Err(invalid(format!(
                    "duplicate development override for {git} package '{name}'"
                )));
            }
        }
        Ok(())
    }
}

fn workspace(source: &Path, doc: &Value) -> Result<Option<(PathBuf, Value)>> {
    if doc.get("workspace").is_some() {
        return Ok(Some((source.into(), doc.clone())));
    }
    if let Some(root) = doc
        .get("package")
        .and_then(|p| p.get("workspace"))
        .and_then(Value::as_str)
    {
        let root = canonical(&source.join(root))?;
        let doc = manifest(&root)?;
        if doc.get("workspace").is_none() {
            return Err(invalid("package.workspace does not point to a workspace"));
        }
        return Ok(Some((root, doc)));
    }
    for parent in source.ancestors().skip(1) {
        if parent.join("Cargo.toml").is_file() {
            let doc = manifest(parent)?;
            if doc.get("workspace").is_some() {
                return Ok(Some((parent.into(), doc)));
            }
        }
    }
    Ok(None)
}

fn expand_inheritance(
    doc: &mut Value,
    source: &Path,
    workspace: Option<&(PathBuf, Value)>,
) -> Result<()> {
    let inherited = |section: &str, key: &str| -> Result<Value> {
        workspace
            .and_then(|(_, w)| w.get("workspace"))
            .and_then(|w| w.get(section))
            .and_then(|t| t.get(key))
            .cloned()
            .ok_or_else(|| {
                invalid(format!(
                    "{} requires missing workspace.{section}.{key}",
                    source.display()
                ))
            })
    };
    let package = doc
        .get_mut("package")
        .and_then(Value::as_table_mut)
        .ok_or_else(|| invalid("Cargo package table is required"))?;
    for (key, value) in package {
        if value.get("workspace").and_then(Value::as_bool) == Some(true) {
            *value = inherited("package", key)?;
            if matches!(key.as_str(), "readme" | "license-file") {
                if let Some(path) = value.as_str() {
                    *value = path_value(&workspace.unwrap().0.join(path));
                }
            }
        }
    }
    if doc
        .get("lints")
        .and_then(|l| l.get("workspace"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        let lints = workspace
            .and_then(|(_, w)| w.get("workspace"))
            .and_then(|w| w.get("lints"))
            .cloned()
            .ok_or_else(|| invalid("missing workspace.lints for inherited lints"))?;
        if doc["lints"].as_table().unwrap().len() != 1 {
            return Err(invalid("inherited lints cannot also define local lints"));
        }
        doc["lints"] = lints;
    }
    for table in dependency_tables_mut(doc) {
        for (name, dependency) in table {
            if dependency.get("workspace").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let local = dependency.as_table().unwrap().clone();
            let mut merged = match inherited("dependencies", name)? {
                Value::String(version) => {
                    Table::from_iter([("version".into(), Value::String(version))])
                }
                Value::Table(table) => table,
                _ => return Err(invalid("invalid inherited Cargo dependency")),
            };
            if let Some(path) = merged.get("path").and_then(Value::as_str) {
                merged.insert("path".into(), path_value(&workspace.unwrap().0.join(path)));
            }
            for (key, value) in local {
                match key.as_str() {
                    "workspace" => {}
                    "features" => {
                        let features = merged
                            .entry("features")
                            .or_insert_with(|| Value::Array(Vec::new()))
                            .as_array_mut()
                            .ok_or_else(|| {
                                invalid("inherited dependency features must be an array")
                            })?;
                        features.extend(
                            value
                                .as_array()
                                .ok_or_else(|| invalid("dependency features must be an array"))?
                                .iter()
                                .cloned(),
                        );
                    }
                    "optional" => {
                        merged.insert(key, value);
                    }
                    "default-features" if merged.get("default-features") == Some(&value) => {}
                    "default-features" => {
                        return Err(invalid(
                            "workspace default-features override is version-dependent; set it in workspace.dependencies",
                        ));
                    }
                    _ => {
                        return Err(invalid(format!(
                            "unsupported inherited dependency key '{key}' for '{name}'"
                        )));
                    }
                }
            }
            *dependency = Value::Table(merged);
        }
    }
    Ok(())
}

fn dependency_tables_mut(doc: &mut Value) -> Vec<&mut Table> {
    let mut tables = Vec::new();
    if let Some(root) = doc.as_table_mut() {
        for (key, value) in root {
            if matches!(
                key.as_str(),
                "dependencies" | "dev-dependencies" | "build-dependencies"
            ) {
                if let Some(table) = value.as_table_mut() {
                    tables.push(table);
                }
            } else if key == "target" {
                if let Some(targets) = value.as_table_mut() {
                    for (_, target) in targets.iter_mut() {
                        tables.extend(dependency_tables_mut(target));
                    }
                }
            }
        }
    }
    tables
}

fn cargo_dependency_tables_mut(doc: &mut Value) -> Vec<&mut Table> {
    let mut tables = Vec::new();
    if let Some(root) = doc.as_table_mut() {
        for (key, value) in root {
            match key.as_str() {
                "dependencies" | "dev-dependencies" | "build-dependencies" | "replace" => {
                    if let Some(table) = value.as_table_mut() {
                        tables.push(table);
                    }
                }
                "patch" => {
                    if let Some(sources) = value.as_table_mut() {
                        for (_, source) in sources {
                            if let Some(table) = source.as_table_mut() {
                                tables.push(table);
                            }
                        }
                    }
                }
                "target" => {
                    if let Some(targets) = value.as_table_mut() {
                        for (_, target) in targets {
                            tables.extend(dependency_tables_mut(target));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    tables
}

fn git_identity(git: &str) -> String {
    git.trim_end_matches('/')
        .trim_end_matches(".git")
        .to_owned()
}

fn library_packages(config: &DevelopmentConfig) -> Result<BTreeMap<(String, String), PathBuf>> {
    let mut packages: BTreeMap<(String, String), PathBuf> = BTreeMap::new();
    let mut count = 0;
    for library in &config.libraries {
        let mut pending = vec![(library.path.clone(), 0)];
        while let Some((directory, depth)) = pending.pop() {
            if directory.join(".patchwork-development").is_file() {
                continue;
            }
            count += 1;
            if count > LIMIT || depth > 128 {
                return Err(invalid("development library scan exceeds safety limit"));
            }
            if directory.join("Cargo.toml").is_file() {
                let doc = manifest(&directory)?;
                if doc.get("package").is_some()
                    && (doc.get("lib").is_some()
                        || (directory.join("src/lib.rs").is_file()
                            && doc
                                .get("package")
                                .and_then(|p| p.get("autolib"))
                                .and_then(Value::as_bool)
                                != Some(false)))
                    && doc
                        .get("package")
                        .and_then(|p| p.get("metadata"))
                        .and_then(|m| m.get("mod"))
                        .is_none()
                {
                    let name = package_name(&doc)?.to_owned();
                    if crate::is_generated_mod_id(&name)
                        || config.exclude.contains(&format!("library/{name}"))
                    {
                        continue;
                    }
                    let key = (library.git.clone(), name.clone());
                    let path = canonical(&directory)?;
                    for ((git, package), previous) in &packages {
                        if git_identity(git) == git_identity(&library.git)
                            && package == &name
                            && previous != &path
                        {
                            return Err(invalid(format!(
                                "duplicate library package '{name}' for Git source '{}': '{}' and '{}'",
                                library.git,
                                previous.display(),
                                path.display()
                            )));
                        }
                    }
                    if let Some(previous) = packages.insert(key, path.clone()) {
                        if previous != path {
                            return Err(invalid(format!(
                                "duplicate library package '{name}' for Git source '{}': '{}' and '{}'",
                                library.git,
                                previous.display(),
                                path.display()
                            )));
                        }
                    }
                }
            }
            for entry in entries(&directory)? {
                count += 1;
                if count > LIMIT {
                    return Err(invalid("development library scan exceeds safety limit"));
                }
                let kind = entry.file_type().map_err(|e| io_error(&entry.path(), e))?;
                // Discovery does not follow internal links; explicit roots may be links.
                if kind.is_symlink() {
                    continue;
                }
                if skipped(&entry.file_name().to_string_lossy()) {
                    continue;
                }
                if kind.is_dir() {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
    }
    Ok(packages)
}

fn selected_sources(
    root: &Path,
    mods: &BTreeMap<String, PathBuf>,
    packs: &BTreeMap<String, PathBuf>,
    cache_mods: &Path,
    cache_modpacks: &Path,
) -> Result<BTreeSet<String>> {
    let root = if root.is_file() {
        canonical(root)?
    } else {
        let id = root
            .to_str()
            .ok_or_else(|| invalid("modpack ID is not UTF-8"))?
            .trim_end_matches(".toml");
        packs
            .get(id)
            .cloned()
            .ok_or_else(|| invalid(format!("modpack '{id}' is unavailable")))?
    };
    let loaded = crate::modpacks::load_mods_with_sources(
        &root,
        &crate::modpacks::SourcePaths {
            mods_folder: cache_mods,
            modpacks_folder: cache_modpacks,
            mods,
            modpacks: packs,
        },
    )?;
    let mut selected: BTreeSet<_> = loaded.mods.keys().cloned().collect();
    for info in loaded.mods.values() {
        for codegen in &info.codegen {
            validate_id(&codegen.generator.crate_name)?;
            selected.insert(codegen.generator.crate_name.clone());
            if let Some(dev) = &codegen.dev_crate {
                validate_id(dev)?;
                if mods.contains_key(dev) {
                    selected.insert(dev.clone());
                }
            }
        }
    }
    Ok(selected)
}

fn cached_entries(folder: &Path, crates: bool) -> Result<BTreeMap<String, PathBuf>> {
    let mut result = BTreeMap::new();
    if !folder.exists() {
        return Ok(result);
    }
    for entry in entries(folder)? {
        let path = entry.path();
        if crates && !path.join("Cargo.toml").is_file() {
            continue;
        }
        if !crates && path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let id = if crates {
            path.file_name()
        } else {
            path.file_stem()
        }
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("cache entry is not UTF-8"))?
        .to_owned();
        validate_id(&id)?;
        result.insert(id, canonical(&path)?);
    }
    Ok(result)
}

fn prune(directory: &Path, wanted: &BTreeSet<PathBuf>) -> Result<()> {
    for entry in entries(directory)? {
        let path = entry.path();
        if !wanted.contains(&path) {
            if entry.file_type().map_err(|e| io_error(&path, e))?.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| io_error(&path, e))?;
            } else {
                fs::remove_file(&path).map_err(|e| io_error(&path, e))?;
            }
        } else if path.is_dir() {
            prune(&path, wanted)?;
        }
    }
    Ok(())
}

fn check_tree(path: &Path, count: &mut usize, depth: usize) -> Result<()> {
    *count += 1;
    if *count > LIMIT || depth > 128 {
        return Err(invalid("development output exceeds safety limit"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| io_error(path, e))?;
    if metadata.is_dir() {
        for entry in entries(path)? {
            check_tree(&entry.path(), count, depth + 1)?;
        }
    } else if !metadata.is_file() {
        return Err(invalid(format!(
            "symlinks and special files are unsupported: {}",
            path.display()
        )));
    }
    Ok(())
}

fn absolute_destination(path: &Path) -> Result<PathBuf> {
    let absolute = crate::paths::absolutize(path)?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            _ => normalized.push(component),
        }
        match fs::symlink_metadata(&normalized) {
            Ok(_) => normalized = canonical(&normalized)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&normalized, error)),
        }
    }
    Ok(normalized)
}

fn canonical(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|e| io_error(path, e))
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid(format!(
            "invalid development ID '{id}': use letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}
fn package_name(doc: &Value) -> Result<&str> {
    let name = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Cargo package.name must be a string"))?;
    validate_id(name)?;
    Ok(name)
}
fn manifest(directory: &Path) -> Result<Value> {
    let path = directory.join("Cargo.toml");
    toml::from_str(&read(&path)?).map_err(|e| PatchworkError::parse_toml("Cargo manifest", path, e))
}
fn entries(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|e| io_error(path, e))?
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| io_error(path, e))?;
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}
fn skipped(name: &str) -> bool {
    matches!(name, ".git" | "target" | ".cargo" | "Cargo.lock")
}
fn path_value(path: &Path) -> Value {
    Value::String(crate::paths::path_to_toml_string(path))
}
fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| io_error(path, e))
}
fn serialize(value: &Value) -> Result<String> {
    toml::to_string_pretty(value).map_err(|e| invalid(e.to_string()))
}
fn invalid(message: impl Into<String>) -> PatchworkError {
    PatchworkError::io_without_path(
        "development overlay",
        io::Error::new(io::ErrorKind::InvalidInput, message.into()),
    )
}
fn io_error(path: &Path, error: io::Error) -> PatchworkError {
    PatchworkError::io("prepare development overlay", path, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "patchwork-development-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn path(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
        fn write(&self, path: &str, source: &str) {
            let path = self.path(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        fn package(&self, directory: &str, name: &str, extra: &str, is_mod: bool) {
            self.write(&format!("{directory}/Cargo.toml"), &format!("[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2021\"\n{}\n{extra}", if is_mod { "[package.metadata.mod]\nsupport = true" } else { "" }));
            self.write(
                &format!("{directory}/src/lib.rs"),
                "pub fn value() -> u32 { 1 }\n",
            );
        }
        fn prepare(&self, config: &DevelopmentConfig) -> Result<PreparedDevelopment> {
            prepare_development(
                config,
                &self.0,
                &self.path("cache/mods"),
                &self.path("cache/packs"),
                &self.path("output"),
            )
        }
        fn scoped(&self, config: &DevelopmentConfig) -> Result<PreparedDevelopment> {
            prepare_development_for_modpack(
                config,
                &self.0,
                &self.path("cache/mods"),
                &self.path("cache/packs"),
                &self.path("output"),
                &self.path("profile.toml"),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn enabled() -> DevelopmentConfig {
        DevelopmentConfig {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn nested_browser_retains_excluded_entries_and_shared_active_dependencies() {
        let f = Fixture::new();
        for id in ["shared", "hidden", "active"] {
            f.package(&format!("cache/mods/{id}"), id, "", true);
        }
        f.write(
            "profile.toml",
            "version='1.0.0'\nmodpacks=['disabled','enabled']\nignore=['modpack/disabled']\n",
        );
        f.write(
            "cache/packs/disabled.toml",
            "version='1.0.0'\nmods=['shared','hidden']\nignore=['active']\n",
        );
        f.write(
            "cache/packs/enabled.toml",
            "version='1.0.0'\nmods=['shared','active']\n",
        );
        let entries = crate::inspect_profile_dependencies(
            &f.path("profile.toml"),
            &f.path("cache/mods"),
            &f.path("cache/packs"),
        )
        .unwrap();
        let shared = entries.iter().find(|entry| entry.id == "shared").unwrap();
        assert!(!shared.ignored);
        assert_eq!(shared.required_by.len(), 2);
        let hidden = entries.iter().find(|entry| entry.id == "hidden").unwrap();
        assert!(hidden.ignored && hidden.toggle_blocked_reason.is_some());
        assert!(
            entries
                .iter()
                .find(|entry| entry.id == "disabled")
                .unwrap()
                .toggle_blocked_reason
                .is_none()
        );
        assert!(
            !entries
                .iter()
                .find(|entry| entry.id == "active")
                .unwrap()
                .ignored
        );
        let selected = crate::modpacks::load_mods(
            &f.path("profile.toml"),
            &f.path("cache/mods"),
            &f.path("cache/packs"),
        )
        .unwrap();
        assert!(selected.mods.contains_key("active"));
        assert!(selected.mods.contains_key("shared"));
        assert!(!selected.mods.contains_key("hidden"));
    }

    #[test]
    fn exclusion_never_erases_a_required_dependency_edge() {
        let f = Fixture::new();
        f.package(
            "cache/mods/consumer",
            "consumer",
            "[package.metadata.mod.dependencies]\ninit=['required']",
            true,
        );
        f.write(
            "profile.toml",
            "version='1.0.0'\nmods=['consumer']\nignore=['required']\n",
        );
        fs::create_dir_all(f.path("cache/packs")).unwrap();
        let loaded = crate::modpacks::load_mods(
            &f.path("profile.toml"),
            &f.path("cache/mods"),
            &f.path("cache/packs"),
        )
        .unwrap();
        assert!(crate::graph::resolve(loaded.mods).is_err());
        f.package(
            "cache/mods/consumer",
            "consumer",
            "[package.metadata.mod.dependencies]\ninit=['modpack/required']",
            true,
        );
        f.write(
            "profile.toml",
            "version='1.0.0'\nmods=['consumer']\nignore=['modpack/required']\n",
        );
        let result = crate::modpacks::load_mods(
            &f.path("profile.toml"),
            &f.path("cache/mods"),
            &f.path("cache/packs"),
        );
        assert!(matches!(
            result,
            Err(PatchworkError::MissingDependency { .. })
        ));
    }

    #[test]
    fn late_exclusion_removes_already_loaded_mods_and_descendants() {
        let f = Fixture::new();
        f.package(
            "cache/mods/early",
            "early",
            "[package.metadata.mod.dependencies]\ninit=['leaf']",
            true,
        );
        f.package("cache/mods/leaf", "leaf", "", true);
        f.package(
            "cache/mods/late",
            "late",
            "[package.metadata.mod.dependencies]\ninit=['modpack/rules']",
            true,
        );
        f.write(
            "cache/packs/rules.toml",
            "version='1.0.0'\nmods=[]\nignore=['early']\n",
        );
        f.write("profile.toml", "version='1.0.0'\nmods=['early','late']\n");
        let loaded = crate::modpacks::load_mods(
            &f.path("profile.toml"),
            &f.path("cache/mods"),
            &f.path("cache/packs"),
        )
        .unwrap();
        assert!(!loaded.mods.contains_key("early"));
        assert!(!loaded.mods.contains_key("leaf"));
        assert!(loaded.mods.contains_key("late"));
    }

    #[test]
    fn browser_reads_local_overrides_and_handles_cycles() {
        let f = Fixture::new();
        f.package(
            "checkout/a",
            "a",
            "[package.metadata.mod.dependencies]\ninit=['b']",
            true,
        );
        f.package(
            "cache/mods/b",
            "b",
            "[package.metadata.mod.dependencies]\ninit=['a']",
            true,
        );
        f.write("profile.toml", "version='1.0.0'\nmods=['a']\n");
        fs::create_dir_all(f.path("cache/packs")).unwrap();
        let locals = BTreeMap::from([("a".into(), f.path("checkout/a"))]);
        let entries = crate::inspect_profile_dependencies_with_sources(
            &f.path("profile.toml"),
            &f.path("cache/mods"),
            &f.path("cache/packs"),
            &locals,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().find(|entry| entry.id == "a").unwrap().local);
        let page = crate::inspect_dependency_page_with_sources(
            crate::DependencyTarget::Profile { id: "profile" },
            &f.path("cache/mods"),
            &f.path("cache/packs"),
            &f.0,
            &locals,
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(page.mods.iter().any(|entry| entry.id == "a" && entry.found));
        assert!(!page.diagnostics.is_empty());
    }

    #[test]
    fn relative_configuration_resolves_without_copying() {
        let f = Fixture::new();
        f.package("checkout/local", "local", "", true);
        f.write("profiles/local.toml", "version='0.1.0'\nmods=['local']\n");
        f.write("dev.toml", "enabled=true\n[mods]\nlocal='checkout/local'\n[modpacks]\nlocal='profiles/local.toml'\n");
        let config = DevelopmentConfig::load(f.path("dev.toml")).unwrap();
        assert_eq!(config.mods["local"], f.path("checkout/local"));
        assert_eq!(config.modpacks["local"], f.path("profiles/local.toml"));
        assert!(!f.path("output").exists());
        let mut invalid = config;
        invalid
            .mods
            .insert("../escape".into(), f.path("checkout/local"));
        assert!(invalid.validate().is_err());
        assert!(!DevelopmentConfig::default().enabled);
        let prepared = f.prepare(&DevelopmentConfig::default()).unwrap();
        assert_eq!(prepared.mods_folder, f.path("cache/mods"));
        assert!(prepared.cargo_config.is_none());
    }

    #[test]
    fn mixed_sources_refresh_deletions_and_keep_unchanged_files_and_originals() {
        let f = Fixture::new();
        f.package(
            "cache/mods/a",
            "a",
            "[dependencies]\nb={path='../b'}\n",
            true,
        );
        f.package("cache/mods/b", "b", "", true);
        f.package(
            "local/b",
            "b",
            "[dependencies]\nhelper={path='../helper'}\n",
            true,
        );
        f.package("local/helper", "helper", "", false);
        f.write("local/b/obsolete.txt", "old");
        f.write("cache/packs/root.toml", "version='0.1.0'\nmods=['a','b']\n");
        let mut config = enabled();
        config.mods.insert("b".into(), PathBuf::from("local/b"));
        let cache_before = read(&f.path("cache/mods/a/Cargo.toml")).unwrap();
        let local_before = read(&f.path("local/b/Cargo.toml")).unwrap();
        let prepared = f.prepare(&config).unwrap();
        let a = manifest(&prepared.mods_folder.join("a")).unwrap();
        assert_eq!(
            a["dependencies"]["b"]["path"].as_str(),
            Some(f.path("output/mods/b").to_str().unwrap())
        );
        let helper =
            manifest(&prepared.mods_folder.join("b")).unwrap()["dependencies"]["helper"]["path"]
                .as_str()
                .unwrap()
                .to_owned();
        assert!(Path::new(&helper).starts_with(f.path("output/packages")));
        let unchanged = f.path("output/mods/a/src/lib.rs");
        let timestamp = fs::metadata(&unchanged).unwrap().modified().unwrap();
        fs::remove_file(f.path("local/b/obsolete.txt")).unwrap();
        f.write("local/b/src/lib.rs", "pub fn value() -> u32 { 2 }\n");
        f.prepare(&config).unwrap();
        assert!(!f.path("output/mods/b/obsolete.txt").exists());
        assert_eq!(
            read(&f.path("output/mods/b/src/lib.rs")).unwrap(),
            "pub fn value() -> u32 { 2 }\n"
        );
        assert_eq!(
            fs::metadata(unchanged).unwrap().modified().unwrap(),
            timestamp
        );
        assert_eq!(
            read(&f.path("cache/mods/a/Cargo.toml")).unwrap(),
            cache_before
        );
        assert_eq!(read(&f.path("local/b/Cargo.toml")).unwrap(), local_before);
        assert!(
            read(&f.path("cache/mods/b/src/lib.rs"))
                .unwrap()
                .contains("{ 1 }")
        );
        assert!(prepared.cargo_config.unwrap().is_file());
    }

    #[test]
    fn git_libraries_are_scoped_by_source_and_package_including_aliases_and_targets() {
        let f = Fixture::new();
        f.package("cache/mods/consumer", "consumer", "[dependencies]\nfirst={git='https://example.com/a',package='shared'}\n[target.'cfg(unix)'.build-dependencies]\nsecond={git='https://example.com/b',package='shared'}\n", true);
        f.package("a/shared", "shared", "", false);
        f.package("a/transitive", "transitive", "", false);
        f.package("a/generated-old", "generated-old", "", false);
        f.package("b/shared", "shared", "", false);
        let mut config = enabled();
        config.libraries = vec![
            DevelopmentLibrarySource {
                git: "https://example.com/a".into(),
                path: "a".into(),
            },
            DevelopmentLibrarySource {
                git: "https://example.com/b".into(),
                path: "b".into(),
            },
        ];
        f.write("profile.toml", "version='0.1.0'\nmods=['consumer']\n");
        let prepared = f.scoped(&config).unwrap();
        let patches: Value =
            toml::from_str(&read(&prepared.cargo_config.unwrap()).unwrap()).unwrap();
        assert_ne!(
            patches["patch"]["https://example.com/a"]["shared"],
            patches["patch"]["https://example.com/b"]["shared"]
        );
        assert!(
            patches["patch"]["https://example.com/a"]
                .get("transitive")
                .is_some()
        );
        assert!(
            patches["patch"]["https://example.com/b"]
                .get("transitive")
                .is_none()
        );
        assert!(
            patches["patch"]["https://example.com/a"]
                .get("consumer")
                .is_none()
        );
        assert!(
            patches["patch"]["https://example.com/a"]
                .get("generated-old")
                .is_none()
        );
    }

    #[test]
    fn workspace_inheritance_resolves_paths_features_package_fields_and_lints() {
        let f = Fixture::new();
        f.write("checkout/Cargo.toml", "[workspace]\nmembers=['member','helper']\n[workspace.package]\nversion='1.2.3'\nedition='2021'\nreadme='README.md'\n[workspace.dependencies]\nhelper={path='helper',features=['base']}\n[workspace.lints.rust]\nunsafe_code='forbid'\n");
        f.write("checkout/README.md", "workspace readme");
        f.write("checkout/member/Cargo.toml", "[package]\nname='member'\nversion.workspace=true\nedition.workspace=true\nreadme.workspace=true\n[package.metadata.mod]\nsupport=true\n[dependencies]\nhelper={workspace=true,features=['extra'],optional=true}\n[lints]\nworkspace=true\n");
        f.write("checkout/member/src/lib.rs", "");
        f.package(
            "checkout/helper",
            "helper",
            "[features]\nbase=[]\nextra=[]\n",
            false,
        );
        let mut config = enabled();
        config
            .mods
            .insert("member".into(), "checkout/member".into());
        let prepared = f.prepare(&config).unwrap();
        let doc = manifest(&prepared.mods_folder.join("member")).unwrap();
        assert_eq!(doc["package"]["version"].as_str(), Some("1.2.3"));
        assert_eq!(
            doc["dependencies"]["helper"]["features"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            doc["dependencies"]["helper"]["optional"].as_bool(),
            Some(true)
        );
        assert_eq!(doc["lints"]["rust"]["unsafe_code"].as_str(), Some("forbid"));
        let readme = Path::new(doc["package"]["readme"].as_str().unwrap());
        assert!(readme.starts_with(&prepared.mods_folder));
        assert_eq!(read(readme).unwrap(), "workspace readme");
        // Cargo itself checks that the materialized manifests remain valid outside their workspace.
        let status = std::process::Command::new("cargo")
            .args([
                "metadata",
                "--offline",
                "--no-deps",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(prepared.mods_folder.join("member/Cargo.toml"))
            .env("CARGO_TARGET_DIR", "/home/paolo/Desktop/sda2/rust_target")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert!(!f.path("checkout/Cargo.lock").exists());
    }

    #[test]
    fn checkouts_rediscover_new_mods_and_respect_manual_overrides_and_exclusions() {
        let f = Fixture::new();
        f.package("checkout/mods/a", "a", "", true);
        f.write(
            "checkout/modpacks/base.toml",
            "version='1.0.0'\nmods=['a']\n",
        );
        f.write("profile.toml", "version='1.0.0'\nmodpacks=['base']\n");
        let mut config = enabled();
        config.checkouts.push(DevelopmentCheckout {
            path: "checkout".into(),
            git: String::new(),
        });
        let first = f.scoped(&config).unwrap();
        assert!(first.mods_folder.join("a/Cargo.toml").is_file());
        f.package("checkout/mods/b", "b", "", true);
        f.write(
            "checkout/modpacks/base.toml",
            "version='1.0.0'\nmods=['a','b']\n",
        );
        let second = f.scoped(&config).unwrap();
        assert!(second.mods_folder.join("b/Cargo.toml").is_file());
        assert!(config.mods.is_empty());

        config.exclude.push("b".into());
        let resolved = config.resolved(&f.0).unwrap();
        assert!(!resolved.mods.contains_key("b"));
        f.package("manual/b", "b", "", true);
        config.mods.insert("b".into(), "manual/b".into());
        assert_eq!(config.resolved(&f.0).unwrap().mods["b"], f.path("manual/b"));
        config.exclude.push("modpack/base".into());
        assert!(!config.resolved(&f.0).unwrap().modpacks.contains_key("base"));
    }

    #[test]
    fn multiple_checkouts_and_external_libraries_have_explicit_conflict_resolution() {
        let f = Fixture::new();
        f.package("one/a", "a", "", true);
        f.package("two/b", "b", "", true);
        f.package("libraries/helper", "helper", "", false);
        let mut config = enabled();
        config.checkouts = vec![
            DevelopmentCheckout {
                path: "one".into(),
                git: String::new(),
            },
            DevelopmentCheckout {
                path: "two".into(),
                git: String::new(),
            },
            DevelopmentCheckout {
                path: "libraries".into(),
                git: "https://example.com/libs.git".into(),
            },
        ];
        let resolved = config.resolved(&f.0).unwrap();
        assert_eq!(resolved.mods.len(), 2);
        assert_eq!(library_packages(&resolved).unwrap().len(), 1);
        f.package("two/a", "a", "", true);
        assert!(
            config
                .resolved(&f.0)
                .unwrap_err()
                .to_string()
                .contains("duplicate discovered ID")
        );
        config.mods.insert("a".into(), "one/a".into());
        config.exclude.push("library/helper".into());
        let resolved = config.resolved(&f.0).unwrap();
        assert_eq!(resolved.mods["a"], f.path("one/a"));
        assert!(library_packages(&resolved).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn linked_profiles_and_configured_roots_are_supported_without_weakening_overlap_checks() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        f.package("checkout/a", "a", "", true);
        f.write("checkout/client.toml", "version='1.0.0'\nmods=['a']\n");
        symlink(f.path("checkout/client.toml"), f.path("profile.toml")).unwrap();
        symlink(f.path("checkout"), f.path("linked-checkout")).unwrap();
        let mut config = enabled();
        config.checkouts.push(DevelopmentCheckout {
            path: "linked-checkout".into(),
            git: String::new(),
        });
        f.scoped(&config).unwrap();
        assert!(
            fs::symlink_metadata(f.path("profile.toml"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::create_dir_all(f.path("real-build")).unwrap();
        symlink(f.path("real-build"), f.path("linked-build")).unwrap();
        prepare_development_for_modpack(
            &config,
            &f.0,
            &f.path("cache/mods"),
            &f.path("cache/packs"),
            &f.path("linked-build/overlay"),
            &f.path("profile.toml"),
        )
        .unwrap();
        assert!(f.path("real-build/overlay/mods/a/Cargo.toml").is_file());
        assert!(
            prepare_development(
                &config,
                &f.0,
                &f.path("cache/mods"),
                &f.path("cache/packs"),
                &f.path("linked-checkout/overlay")
            )
            .is_err()
        );
    }

    #[test]
    fn library_discovery_ignores_duplicate_composed_binaries() {
        let f = Fixture::new();
        for directory in [
            "checkout/build-server/server",
            "checkout/build-check/server",
        ] {
            f.write(
                &format!("{directory}/Cargo.toml"),
                "[package]\nname='server'\nversion='0.1.0'\n",
            );
            f.write(&format!("{directory}/src/main.rs"), "fn main() {}\n");
        }
        f.package("checkout/mods/helper", "helper", "", false);
        let mut config = enabled();
        config.libraries.push(DevelopmentLibrarySource {
            git: "https://example.com/game.git".into(),
            path: f.path("checkout"),
        });
        let packages = library_packages(&config).unwrap();
        assert_eq!(packages.len(), 1);
        assert!(packages.keys().all(|(_, name)| name == "helper"));
    }

    #[test]
    fn missing_workspace_values_and_duplicate_library_packages_have_actionable_errors() {
        let f = Fixture::new();
        f.package(
            "cache/mods/member",
            "member",
            "[dependencies]\nhelper.workspace=true\n",
            true,
        );
        assert!(
            f.prepare(&enabled())
                .unwrap_err()
                .to_string()
                .contains("workspace.dependencies.helper")
        );
        f.package("checkout/one", "duplicate", "", false);
        f.package("checkout/two", "duplicate", "", false);
        let mut config = enabled();
        config.libraries.push(DevelopmentLibrarySource {
            git: "https://example.com/repo".into(),
            path: "checkout".into(),
        });
        assert!(
            config
                .resolved(&f.0)
                .unwrap_err()
                .to_string()
                .contains("duplicate library package")
        );
    }

    #[test]
    fn selected_closure_honors_nested_pack_exclusions_and_ignores_unrelated_broken_crates() {
        let f = Fixture::new();
        f.package("cache/mods/good", "good", "", true);
        f.package(
            "cache/mods/unrelated",
            "unrelated",
            "[dependencies]\nmissing={path='../missing'}",
            true,
        );
        f.write(
            "cache/packs/nested.toml",
            "version='0.1.0'\nmods=['good']\n",
        );
        f.write(
            "profile.toml",
            "version='0.1.0'\nmodpacks=['nested','missing']\nignore=['modpack/missing']\n",
        );
        let prepared = f.scoped(&enabled()).unwrap();
        assert!(prepared.mods_folder.join("good").is_dir());
        assert!(!prepared.mods_folder.join("unrelated").exists());
        f.write("profile.toml", "version='0.1.0'\nmods=[]\n");
        f.scoped(&enabled()).unwrap();
        assert!(!prepared.mods_folder.join("good").exists());
    }

    #[test]
    fn missing_generated_outputs_and_dev_stubs_are_deferred() {
        let f = Fixture::new();
        f.package("cache/mods/owner", "owner", "[[package.metadata.mod.codegen]]\ncrate='generated-result'\ndev_crate='generated-result'\n[package.metadata.mod.codegen.generator]\ncrate='tool'\n[dependencies]\ngenerated-result={path='../generated-result'}\n[patch.crates-io]\ngenerated-other={path='../generated-other'}\n", true);
        f.package("cache/mods/tool", "tool", "", false);
        f.write("profile.toml", "version='0.1.0'\nmods=['owner']\n");
        let prepared = f.scoped(&enabled()).unwrap();
        let doc = manifest(&prepared.mods_folder.join("owner")).unwrap();
        assert_eq!(
            doc["dependencies"]["generated-result"]["path"].as_str(),
            Some(
                prepared
                    .mods_folder
                    .join("generated-result")
                    .to_str()
                    .unwrap()
            )
        );
        assert_eq!(
            doc["patch"]["crates-io"]["generated-other"]["path"].as_str(),
            Some(
                prepared
                    .mods_folder
                    .join("generated-other")
                    .to_str()
                    .unwrap()
            )
        );
        assert!(!prepared.mods_folder.join("generated-result").exists());
        assert!(prepared.mods_folder.join("tool").exists());
    }

    #[test]
    fn known_repository_does_not_override_unrelated_same_name_dependency() {
        let f = Fixture::new();
        f.write("cache/mods/shared/Cargo.toml", "[package]\nname='shared'\nversion='0.1.0'\nrepository='https://example.com/one.git'\n[package.metadata.mod]\nsupport=true\n");
        f.write("cache/mods/shared/src/lib.rs", "");
        f.package("cache/mods/user", "user", "[dependencies]\none={git='https://example.com/one',package='shared'}\ntwo={git='https://example.com/two',package='shared'}\n", true);
        let prepared = f.prepare(&enabled()).unwrap();
        let doc: Value = toml::from_str(&read(&prepared.cargo_config.unwrap()).unwrap()).unwrap();
        assert!(doc["patch"].get("https://example.com/one").is_some());
        assert!(doc["patch"].get("https://example.com/two").is_none());
    }

    #[test]
    fn ambiguous_unattributed_mod_git_sources_are_rejected() {
        let f = Fixture::new();
        f.package("cache/mods/shared", "shared", "", true);
        f.package("cache/mods/user", "user", "[dependencies]\none={git='https://example.com/one',package='shared'}\ntwo={git='https://example.com/two',package='shared'}\n", true);
        assert!(
            f.prepare(&enabled())
                .unwrap_err()
                .to_string()
                .contains("ambiguous Git sources")
        );
    }

    #[test]
    fn output_overlap_and_unowned_destination_are_rejected() {
        let f = Fixture::new();
        f.package("cache/mods/a", "a", "", true);
        assert!(
            prepare_development(
                &enabled(),
                &f.0,
                &f.path("cache/mods"),
                &f.path("cache/packs"),
                &f.path("cache/mods/out")
            )
            .is_err()
        );
        f.write("output/user-file", "keep me");
        assert!(f.prepare(&enabled()).is_err());
        assert_eq!(read(&f.path("output/user-file")).unwrap(), "keep me");
    }

    #[cfg(unix)]
    #[test]
    fn source_and_destination_symlinks_are_rejected_without_following_them() {
        let f = Fixture::new();
        f.package("cache/mods/a", "a", "", true);
        f.write("outside", "keep me");
        std::os::unix::fs::symlink(f.path("outside"), f.path("cache/mods/a/link")).unwrap();
        assert!(
            f.prepare(&enabled())
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        fs::remove_file(f.path("cache/mods/a/link")).unwrap();
        f.prepare(&enabled()).unwrap();
        std::os::unix::fs::symlink(f.path("outside"), f.path("output/mods/a/link")).unwrap();
        assert!(
            f.prepare(&enabled())
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        assert_eq!(read(&f.path("outside")).unwrap(), "keep me");
    }
}
