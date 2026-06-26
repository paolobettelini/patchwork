use crate::error::{PatchworkError, Result};
use crate::model::ModInfo;
use crate::paths::{crate_dir, path_to_toml_string, relative_path_for_manifest};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ResolvedCodegenTask {
    pub package: String,
    pub version: String,
    pub generator_manifest: PathBuf,
    pub command: String,
    pub output_crate_dir: PathBuf,
    pub dev_crate_dir: Option<PathBuf>,
}

const GENERATED_PATCH_BEGIN: &str = "# BEGIN COMPOSER CODEGEN PATCHES";
const GENERATED_PATCH_END: &str = "# END COMPOSER CODEGEN PATCHES";

pub fn resolve_tasks(
    cache_folder: &Path,
    mods_folder: &Path,
    mods: &[(String, ModInfo)],
) -> Result<Vec<ResolvedCodegenTask>> {
    let mut tasks = BTreeMap::new();

    for (_modname, modinfo) in mods {
        for declaration in &modinfo.codegen {
            if tasks.contains_key(&declaration.package) {
                return Err(PatchworkError::DuplicateGeneratedCrate {
                    package: declaration.package.clone(),
                });
            }

            let generator_manifest =
                crate_dir(mods_folder, &declaration.generator.crate_name)?.join("Cargo.toml");
            if !generator_manifest.exists() {
                return Err(PatchworkError::MissingCodegenGenerator {
                    generator_crate: declaration.generator.crate_name.clone(),
                    manifest_path: generator_manifest,
                });
            }

            let output_crate_dir = cache_folder.join(&declaration.package);
            let dev_crate_dir = declaration
                .dev_crate
                .as_deref()
                .map(|name| crate_dir(mods_folder, name))
                .transpose()?;

            tasks.insert(
                declaration.package.clone(),
                ResolvedCodegenTask {
                    package: declaration.package.clone(),
                    version: declaration.version.clone(),
                    generator_manifest: generator_manifest.canonicalize().map_err(|source| {
                        PatchworkError::io(
                            "canonicalize codegen generator manifest",
                            &generator_manifest,
                            source,
                        )
                    })?,
                    command: declaration.generator.command.clone(),
                    output_crate_dir,
                    dev_crate_dir,
                },
            );
        }
    }

    Ok(tasks.into_values().collect())
}

pub fn run_tasks(
    template_dir: &Path,
    mods_folder: &Path,
    modpacks_folder: &Path,
    modpack: &Path,
    tasks: &[ResolvedCodegenTask],
) -> Result<()> {
    for task in tasks {
        let mut command = Command::new("cargo");
        command
            .arg("run")
            .arg("--manifest-path")
            .arg(&task.generator_manifest)
            .arg("--")
            .arg(&task.command)
            .arg("--project")
            .arg(template_dir)
            .arg("--output-crate")
            .arg(&task.output_crate_dir)
            .arg("--package")
            .arg(&task.package)
            .arg("--version")
            .arg(&task.version)
            .arg("--mods-folder")
            .arg(mods_folder)
            .arg("--modpacks-folder")
            .arg(modpacks_folder)
            .arg("--modpack")
            .arg(modpack);

        if let Some(dev_crate_dir) = &task.dev_crate_dir {
            command.arg("--dev-crate").arg(dev_crate_dir);
        }

        let status = command.status().map_err(|source| {
            PatchworkError::io_without_path("run codegen generator process", source)
        })?;
        if !status.success() {
            return Err(PatchworkError::CodegenFailed {
                package: task.package.clone(),
                status: status.to_string(),
            });
        }
    }

    Ok(())
}

pub fn patch_generated_crates(template_dir: &Path, tasks: &[ResolvedCodegenTask]) -> Result<()> {
    if tasks.is_empty() {
        return Ok(());
    }

    let manifest_path = template_dir.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).map_err(|source| {
        PatchworkError::io("read generated Cargo.toml", &manifest_path, source)
    })?;
    let manifest = remove_generated_patch(&manifest);
    let patch_lines = tasks
        .iter()
        .map(|task| {
            format!(
                "{} = {{ path = \"{}\" }}",
                task.package,
                path_to_toml_string(&relative_path_for_manifest(
                    template_dir,
                    &task.output_crate_dir
                ))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let patch_block = format!("{GENERATED_PATCH_BEGIN}\n{patch_lines}\n{GENERATED_PATCH_END}");

    let patched_manifest = if manifest
        .lines()
        .any(|line| line.trim() == "[patch.crates-io]")
    {
        insert_into_existing_patch_table(&manifest, &patch_block)
    } else {
        append_generated_patch_table(&manifest, &patch_block)
    };

    fs::write(&manifest_path, patched_manifest).map_err(|source| {
        PatchworkError::io("write generated Cargo.toml", &manifest_path, source)
    })?;

    Ok(())
}

fn remove_generated_patch(manifest: &str) -> String {
    let mut output = Vec::new();
    let mut skipping = false;

    for line in manifest.lines() {
        match line.trim() {
            GENERATED_PATCH_BEGIN => skipping = true,
            GENERATED_PATCH_END => skipping = false,
            _ if !skipping => output.push(line),
            _ => {}
        }
    }

    let mut manifest = output.join("\n");
    if !manifest.ends_with('\n') {
        manifest.push('\n');
    }
    manifest
}

fn insert_into_existing_patch_table(manifest: &str, patch_block: &str) -> String {
    let mut output = Vec::new();
    let mut inserted = false;

    for line in manifest.lines() {
        output.push(line.to_string());
        if !inserted && line.trim() == "[patch.crates-io]" {
            output.push(patch_block.to_string());
            inserted = true;
        }
    }

    let mut manifest = output.join("\n");
    manifest.push('\n');
    manifest
}

fn append_generated_patch_table(manifest: &str, patch_block: &str) -> String {
    let mut manifest = manifest.trim_end().to_string();
    manifest.push_str("\n\n[patch.crates-io]\n");
    manifest.push_str(patch_block);
    manifest.push('\n');
    manifest
}
