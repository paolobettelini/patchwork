use std::path::{Path, PathBuf};

use semver::Version;
use toml::Value;

use crate::error::{PatchworkError, Result};
use crate::model::ModInfo;

#[derive(Clone, Copy)]
pub struct RegistryWorkspaceManifest<'a> {
    pub path: &'a Path,
    pub source: &'a str,
}

#[derive(Debug, Clone)]
pub struct RegistryModManifest {
    pub id: String,
    pub title: String,
    pub version: String,
    pub mod_info: ModInfo,
}

pub fn parse_registry_mod_manifest(
    source: &str,
    manifest_path: &Path,
    workspace_manifests: &[RegistryWorkspaceManifest<'_>],
) -> Result<Option<RegistryModManifest>> {
    let document = toml::from_str::<Value>(source)
        .map_err(|source| PatchworkError::parse_toml("mod Cargo.toml", manifest_path, source))?;
    let Some(package) = document.get("package").and_then(Value::as_table) else {
        return Ok(None);
    };
    let Some(metadata) = package.get("metadata").and_then(Value::as_table) else {
        return Ok(None);
    };
    let Some(mod_metadata) = metadata.get("mod") else {
        return Ok(None);
    };

    let id = package
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_metadata(manifest_path, "<unknown>", "package.name is required"))?
        .trim()
        .to_owned();
    validate_registry_id(&id, manifest_path)?;

    let mod_info = mod_metadata
        .clone()
        .try_into::<ModInfo>()
        .map_err(|error| {
            invalid_metadata(
                manifest_path,
                &id,
                format!("cannot parse metadata: {error}"),
            )
        })?;
    mod_info.validate(&id, manifest_path)?;

    let version = resolve_package_version(package, manifest_path, workspace_manifests, &id)?;
    if version.is_empty() || version.len() > 64 || version.chars().any(char::is_control) {
        return Err(invalid_metadata(
            manifest_path,
            &id,
            "package.version must contain between 1 and 64 non-control characters",
        ));
    }
    Version::parse(&version).map_err(|error| {
        invalid_metadata(
            manifest_path,
            &id,
            format!("package.version is not valid semantic versioning: {error}"),
        )
    })?;

    let title = mod_info.title.as_deref().unwrap_or(&id).trim().to_owned();
    if title.is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control) {
        return Err(invalid_metadata(
            manifest_path,
            &id,
            "title must contain between 1 and 200 non-control characters",
        ));
    }

    for dependency in mod_info
        .dependencies
        .init
        .iter()
        .chain(&mod_info.dependencies.run)
        .chain(&mod_info.dependencies.ownership)
    {
        validate_registry_id(dependency, manifest_path)?;
        if dependency == &id {
            return Err(invalid_metadata(
                manifest_path,
                &id,
                "a mod cannot depend on itself",
            ));
        }
    }

    if let Some(provides) = &mod_info.provides {
        validate_registry_id(provides, manifest_path)?;
    }

    Ok(Some(RegistryModManifest {
        id,
        title,
        version,
        mod_info,
    }))
}

fn resolve_package_version(
    package: &toml::map::Map<String, Value>,
    manifest_path: &Path,
    workspace_manifests: &[RegistryWorkspaceManifest<'_>],
    mod_id: &str,
) -> Result<String> {
    let value = package
        .get("version")
        .ok_or_else(|| invalid_metadata(manifest_path, mod_id, "package.version is required"))?;
    if let Some(version) = value.as_str() {
        return Ok(version.trim().to_owned());
    }

    let inherits_workspace = value
        .as_table()
        .and_then(|table| table.get("workspace"))
        .and_then(Value::as_bool)
        == Some(true);
    if !inherits_workspace {
        return Err(invalid_metadata(
            manifest_path,
            mod_id,
            "package.version must be a string or `{ workspace = true }`",
        ));
    }

    let manifest_directory = manifest_path.parent().unwrap_or_else(|| Path::new(""));
    let mut candidates = workspace_manifests
        .iter()
        .filter_map(|candidate| {
            let workspace_directory = candidate.path.parent().unwrap_or_else(|| Path::new(""));
            manifest_directory
                .starts_with(workspace_directory)
                .then_some((*candidate, workspace_directory.components().count()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, depth)| std::cmp::Reverse(*depth));

    for (candidate, _) in candidates {
        let Ok(document) = toml::from_str::<Value>(candidate.source) else {
            continue;
        };
        let Some(workspace) = document.get("workspace").and_then(Value::as_table) else {
            continue;
        };
        let Some(version) = workspace
            .get("package")
            .and_then(Value::as_table)
            .and_then(|package| package.get("version"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        return Ok(version.trim().to_owned());
    }

    Err(invalid_metadata(
        manifest_path,
        mod_id,
        "package.version inherits from a workspace, but no ancestor `[workspace.package].version` was found",
    ))
}

fn validate_registry_id(value: &str, manifest_path: &Path) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        });
    if valid {
        Ok(())
    } else {
        Err(invalid_metadata(
            manifest_path,
            value,
            "mod IDs must be lowercase slugs containing only a-z, 0-9, '-', '_' or '.'",
        ))
    }
}

fn invalid_metadata(
    manifest_path: &Path,
    mod_name: &str,
    reason: impl Into<String>,
) -> PatchworkError {
    PatchworkError::InvalidModMetadata {
        mod_name: mod_name.to_owned(),
        manifest_path: PathBuf::from(manifest_path),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_support_mod_with_workspace_version() {
        let workspace_source = r#"
[workspace]
members = ["mods/api"]

[workspace.package]
version = "1.2.3"
"#;
        let manifest_source = r#"
[package]
name = "inventory-api"
version.workspace = true

[package.metadata.mod]
title = "Inventory API"
support = true
"#;
        let workspace_path = Path::new("Cargo.toml");
        let manifest_path = Path::new("mods/api/Cargo.toml");
        let parsed = parse_registry_mod_manifest(
            manifest_source,
            manifest_path,
            &[RegistryWorkspaceManifest {
                path: workspace_path,
                source: workspace_source,
            }],
        )
        .unwrap()
        .unwrap();

        assert_eq!(parsed.id, "inventory-api");
        assert_eq!(parsed.title, "Inventory API");
        assert_eq!(parsed.version, "1.2.3");
        assert!(parsed.mod_info.support);
    }

    #[test]
    fn ignores_non_patchwork_packages() {
        let parsed = parse_registry_mod_manifest(
            "[package]\nname = \"ordinary\"\nversion = \"1.0.0\"",
            Path::new("Cargo.toml"),
            &[],
        )
        .unwrap();
        assert!(parsed.is_none());
    }
}
