use super::*;

pub(super) fn scan(
    root: &Path,
    config: &DevelopmentConfig,
    mods: &mut BTreeMap<String, PathBuf>,
    packs: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    if !root.is_dir() {
        return Err(invalid(format!(
            "checkout is not a directory: {}",
            root.display()
        )));
    }
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut count = 0;
    while let Some((directory, depth)) = pending.pop() {
        if depth > 128 {
            return Err(invalid("checkout nesting exceeds safety limit"));
        }
        // Never index a managed copy of the sources as another checkout.
        if directory.join(".patchwork-development").is_file() {
            continue;
        }
        for entry in entries(&directory)? {
            count += 1;
            if count > LIMIT {
                return Err(invalid("checkout scan exceeds safety limit"));
            }
            let name = entry.file_name();
            if skipped(&name.to_string_lossy()) || name == ".patchwork" {
                continue;
            }
            let kind = entry.file_type().map_err(|e| io_error(&entry.path(), e))?;
            if kind.is_dir() {
                pending.push((entry.path(), depth + 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            if name == "Cargo.toml" {
                let doc = manifest(&directory)?;
                if let Some(metadata) = doc
                    .get("package")
                    .and_then(|p| p.get("metadata"))
                    .and_then(|m| m.get("mod"))
                {
                    let id = package_name(&doc)?;
                    if crate::is_generated_mod_id(id)
                        || config.exclude.iter().any(|e| e == id)
                        || config.mods.contains_key(id)
                    {
                        continue;
                    }
                    let info: ModInfo = metadata
                        .clone()
                        .try_into()
                        .map_err(|e| PatchworkError::parse_toml("mod metadata", &path, e))?;
                    info.validate(id, &path)?;
                    insert(mods, id, &directory)?;
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                let source = read(&path)?;
                let Ok(doc) = toml::from_str::<Value>(&source) else {
                    continue;
                };
                // Match the core modpack schema, not arbitrary TOML configuration.
                if !["mods", "modpacks", "ignore"]
                    .iter()
                    .any(|key| doc.get(key).is_some_and(Value::is_array))
                {
                    continue;
                }
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| invalid("modpack filename must be UTF-8"))?;
                if config.exclude.contains(&format!("modpack/{id}"))
                    || config.modpacks.contains_key(id)
                {
                    continue;
                }
                validate_id(id)?;
                let _: Modpack = doc
                    .try_into()
                    .map_err(|e| PatchworkError::parse_toml("modpack", &path, e))?;
                insert(packs, id, &path)?;
            }
        }
    }
    Ok(())
}

fn insert(entries: &mut BTreeMap<String, PathBuf>, id: &str, path: &Path) -> Result<()> {
    let path = canonical(path)?;
    if let Some(previous) = entries.get(id) {
        if previous != &path {
            return Err(invalid(format!(
                "duplicate discovered ID '{id}': '{}' and '{}'; select an explicit override or exclude it",
                previous.display(),
                path.display()
            )));
        }
    }
    entries.insert(id.into(), path);
    Ok(())
}
