use crate::error::{PatchworkError, Result};
use crate::model::{ModInfo, Modpack, ProcessOptions};
use crate::paths::{crate_dir, path_to_toml_string, relative_path_for_manifest};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct ResolvedCodegenTask {
    pub package: String,
    pub version: String,
    pub generator_manifest: PathBuf,
    pub command: String,
    pub output_crate_dir: PathBuf,

    /// Temporary location used to regenerate the crate without touching the existing output.
    ///
    /// After generation, Patchwork compares this directory with the current generated crate.
    /// If their contents are identical, the existing crate is restored unchanged so Cargo can
    /// reuse its previous build artifacts and avoid unnecessary recompilation.
    pub temporary_output_crate_dir: PathBuf,
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
            let temporary_output_crate_dir = cache_folder.join("temp").join(&declaration.package);
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
                    temporary_output_crate_dir,
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
    output: &mut dyn FnMut(&[u8]),
) -> Result<()> {
    let build_options = profile_build_options(modpack)?;
    let cargo_arguments = expanded_build_arguments(&build_options, modpack)?;

    for task in tasks {
        prepare_generated_crate_backup(&task.output_crate_dir, &task.temporary_output_crate_dir)?;

        let had_existing_output = task.temporary_output_crate_dir.is_dir();
        let mut command = Command::new("cargo");
        command
            // Cargo discovers `.cargo/config.toml` from its working directory.
            // Running generators from the mods folder lets a source checkout
            // patch distributable Git-only helper libraries back to local
            // paths, while downloaded mod caches still use Git normally.
            .current_dir(mods_folder)
            .arg("run");
        command.args(&cargo_arguments);
        command.envs(&build_options.env);
        command.env("CARGO_TERM_COLOR", "always");
        command
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
        let status = match run_command_with_output(&mut command, output) {
            Ok(status) => status,
            Err(source) => {
                restore_generated_crate_backup(
                    &task.output_crate_dir,
                    &task.temporary_output_crate_dir,
                    had_existing_output,
                )?;
                return Err(PatchworkError::io_without_path(
                    "run codegen generator process",
                    source,
                ));
            }
        };

        if !status.success() {
            restore_generated_crate_backup(
                &task.output_crate_dir,
                &task.temporary_output_crate_dir,
                had_existing_output,
            )?;
            return Err(PatchworkError::CodegenFailed {
                package: task.package.clone(),
                status: status.to_string(),
            });
        }

        finish_generated_crate(
            &task.output_crate_dir,
            &task.temporary_output_crate_dir,
            had_existing_output,
        )?;
    }

    Ok(())
}

fn run_command_with_output(
    command: &mut Command,
    output: &mut dyn FnMut(&[u8]),
) -> io::Result<ExitStatus> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn()?;

    let stdout = child
        .stdout
        .take()
        .expect("piped codegen stdout should be available");

    let stderr = child
        .stderr
        .take()
        .expect("piped codegen stderr should be available");

    let (sender, receiver) = mpsc::channel::<Vec<u8>>();

    let stdout_reader = spawn_output_reader(stdout, sender.clone());
    let stderr_reader = spawn_output_reader(stderr, sender);

    for chunk in receiver {
        output(&chunk);
    }

    stdout_reader
        .join()
        .map_err(|_| io::Error::other("codegen stdout reader panicked"))??;

    stderr_reader
        .join()
        .map_err(|_| io::Error::other("codegen stderr reader panicked"))??;

    child.wait()
}

fn spawn_output_reader<R>(
    mut reader: R,
    sender: mpsc::Sender<Vec<u8>>,
) -> thread::JoinHandle<io::Result<()>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),

                Ok(length) => {
                    if sender.send(buffer[..length].to_vec()).is_err() {
                        return Ok(());
                    }
                }

                Err(error) if error.kind() == ErrorKind::Interrupted => continue,

                Err(error) => return Err(error),
            }
        }
    })
}

fn profile_build_options(modpack_path: &Path) -> Result<ProcessOptions> {
    let source = fs::read_to_string(modpack_path).map_err(|source| {
        PatchworkError::io("read modpack for codegen options", modpack_path, source)
    })?;
    let modpack = toml::from_str::<Modpack>(&source)
        .map_err(|source| PatchworkError::parse_toml("modpack", modpack_path, source))?;
    Ok(modpack.options.build)
}

fn expanded_build_arguments(
    options: &ProcessOptions,
    modpack_path: &Path,
) -> Result<Vec<String>> {
    options.expanded_args().map_err(|reason| {
        PatchworkError::InvalidModpackMetadata {
            id: modpack_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("<unknown>")
                .to_owned(),
            manifest_path: modpack_path.to_path_buf(),
            reason: format!("invalid profile build arguments: {reason}"),
        }
    })
}

fn prepare_generated_crate_backup(output_directory: &Path, backup_directory: &Path) -> Result<()> {
    if backup_directory.is_dir() {
        if output_directory.is_dir() {
            remove_directory_if_exists(backup_directory, "remove stale generated crate backup")?;
        } else {
            fs::rename(backup_directory, output_directory).map_err(|source| {
                PatchworkError::io("recover generated crate backup", backup_directory, source)
            })?;
        }
    }

    let temporary_root = backup_directory
        .parent()
        .expect("temporary generated crate must have a parent directory");
    fs::create_dir_all(temporary_root).map_err(|source| {
        PatchworkError::io("create codegen temporary directory", temporary_root, source)
    })?;

    if output_directory.is_dir() {
        fs::rename(output_directory, backup_directory).map_err(|source| {
            PatchworkError::io(
                "move generated crate to temporary backup",
                output_directory,
                source,
            )
        })?;
    }

    Ok(())
}

fn restore_generated_crate_backup(
    output_directory: &Path,
    backup_directory: &Path,
    had_existing_output: bool,
) -> Result<()> {
    remove_directory_if_exists(output_directory, "remove incomplete generated crate")?;

    if had_existing_output {
        fs::rename(backup_directory, output_directory).map_err(|source| {
            PatchworkError::io("restore generated crate backup", backup_directory, source)
        })?;
    }

    Ok(())
}

fn finish_generated_crate(
    output_directory: &Path,
    backup_directory: &Path,
    had_existing_output: bool,
) -> Result<()> {
    if !had_existing_output {
        return Ok(());
    }

    let unchanged = match directories_equal(output_directory, backup_directory) {
        Ok(unchanged) => unchanged,
        Err(error) => {
            restore_generated_crate_backup(
                output_directory,
                backup_directory,
                had_existing_output,
            )?;
            return Err(error);
        }
    };

    if unchanged {
        remove_directory_if_exists(output_directory, "remove unchanged regenerated crate")?;
        fs::rename(backup_directory, output_directory).map_err(|source| {
            PatchworkError::io(
                "restore unchanged generated crate",
                backup_directory,
                source,
            )
        })?;
    } else {
        remove_directory_if_exists(backup_directory, "remove outdated generated crate backup")?;
    }

    Ok(())
}

fn directories_equal(left: &Path, right: &Path) -> Result<bool> {
    let left_entries = sorted_directory_entries(left)?;
    let right_entries = sorted_directory_entries(right)?;

    if left_entries.len() != right_entries.len() {
        return Ok(false);
    }

    for (left_entry, right_entry) in left_entries.iter().zip(&right_entries) {
        if left_entry.file_name() != right_entry.file_name() {
            return Ok(false);
        }

        let left_path = left_entry.path();
        let right_path = right_entry.path();
        let left_type = left_entry.file_type().map_err(|source| {
            PatchworkError::io("read regenerated crate entry type", &left_path, source)
        })?;
        let right_type = right_entry.file_type().map_err(|source| {
            PatchworkError::io("read previous generated entry type", &right_path, source)
        })?;

        if left_type.is_dir() && right_type.is_dir() {
            if !directories_equal(&left_path, &right_path)? {
                return Ok(false);
            }
        } else if left_type.is_file() && right_type.is_file() {
            let left_contents = fs::read(&left_path).map_err(|source| {
                PatchworkError::io("read regenerated crate file", &left_path, source)
            })?;
            let right_contents = fs::read(&right_path).map_err(|source| {
                PatchworkError::io("read previous generated file", &right_path, source)
            })?;

            if left_contents != right_contents {
                return Ok(false);
            }
        } else if left_type.is_symlink() && right_type.is_symlink() {
            let left_target = fs::read_link(&left_path).map_err(|source| {
                PatchworkError::io("read regenerated crate symlink", &left_path, source)
            })?;
            let right_target = fs::read_link(&right_path).map_err(|source| {
                PatchworkError::io("read previous generated symlink", &right_path, source)
            })?;

            if left_target != right_target {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
    }

    Ok(true)
}

fn sorted_directory_entries(directory: &Path) -> Result<Vec<fs::DirEntry>> {
    let entries = fs::read_dir(directory).map_err(|source| {
        PatchworkError::io("read generated crate directory", directory, source)
    })?;

    let mut entries = entries
        .map(|entry| {
            entry.map_err(|source| {
                PatchworkError::io("read generated crate directory entry", directory, source)
            })
        })
        .collect::<Result<Vec<_>>>()?;

    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn remove_directory_if_exists(directory: &Path, action: &'static str) -> Result<()> {
    match fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PatchworkError::io(action, directory, source)),
    }
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
