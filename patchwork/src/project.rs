use crate::codegen;
use crate::error::{PatchworkError, Result};
use crate::model::ModInfo;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const CARGO_TOML_TEMPLATE: &str = include_str!("../../template/Cargo.toml");
const MAIN_RS_TEMPLATE: &str = include_str!("../../template/src/main.rs");
const TEMPLATE_ASSETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../template/assets");

pub fn create_project(
    cache_folder: &Path,
    project_name: &str,
    mods_folder: &Path,
    modpacks_folder: &Path,
    modpack: &Path,
    mods: Vec<(String, ModInfo)>,
    provider_map: HashMap<String, String>,
    owned_objects: HashSet<String>,
) -> Result<()> {
    let project_dir = cache_folder.join(project_name);

    if project_dir.exists() {
        fs::remove_dir_all(&project_dir).map_err(|source| {
            PatchworkError::io("remove existing composed project", &project_dir, source)
        })?;
    }

    fs::create_dir_all(project_dir.join("src")).map_err(|source| {
        PatchworkError::io("create composed src directory", &project_dir, source)
    })?;
    fs::create_dir_all(project_dir.join("src").join("generated")).map_err(|source| {
        PatchworkError::io(
            "create composed generated directory",
            project_dir.join("src").join("generated"),
            source,
        )
    })?;
    fs::write(project_dir.join("src").join("generated").join("mod.rs"), "").map_err(|source| {
        PatchworkError::io(
            "write generated module marker",
            project_dir.join("src").join("generated").join("mod.rs"),
            source,
        )
    })?;
    fs::write(project_dir.join("src").join("main.rs"), MAIN_RS_TEMPLATE).map_err(|source| {
        PatchworkError::io(
            "write composed main.rs template",
            project_dir.join("src").join("main.rs"),
            source,
        )
    })?;
    copy_project_assets(&project_dir, mods_folder, &mods)?;

    let dependencies = mods
        .iter()
        .map(|(modname, _modinfo)| {
            let mod_path = mods_folder.join(modname);
            let mod_path = canonical_cargo_path(&mod_path, "canonicalize selected mod path")?;
            Ok(format!(
                "{} = {{ path = {} }}",
                modname,
                toml_string(&mod_path)
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("\n");

    let git_patches = generate_git_source_patches(mods_folder, &mods)?;
    let cargo_toml = CARGO_TOML_TEMPLATE
        .replace("#PLACEHOLDER", &dependencies)
        .replace(
            "name = \"template\"",
            &format!("name = \"{}\"", project_name),
        )
        + &git_patches;

    fs::write(project_dir.join("Cargo.toml"), cargo_toml).map_err(|source| {
        PatchworkError::io(
            "write composed Cargo.toml",
            project_dir.join("Cargo.toml"),
            source,
        )
    })?;

    let init_mods_glue = mods
        .iter()
        .filter(|(_, modinfo)| modinfo.has_lifecycle())
        .map(|(modname, modinfo)| {
            let name = modname.replace('-', "_");
            let entry = modinfo
                .entry_type()
                .expect("lifecycle mod should declare an entry type");
            let params = generate_mut_params(&modinfo.dependencies.init, &provider_map);

            format!(
                "
                let mut {} = {}::{}::init({});
                ",
                name, name, entry, params
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let arc_wrappers_glue = mods
        .iter()
        .filter(|(modname, modinfo)| modinfo.has_lifecycle() && !owned_objects.contains(modname))
        .map(|(modname, _modinfo)| {
            let name = modname.replace('-', "_");

            format!(
                "
                let {} = Arc::new({});
                ",
                name, name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let run_mods_glue = mods
        .iter()
        .filter(|(_, modinfo)| modinfo.has_lifecycle())
        .map(|(modname, modinfo)| {
            let name = modname.replace('-', "_");
            let params = generate_run_params(
                &modinfo.dependencies.run,
                &modinfo.dependencies.ownership,
                &provider_map,
            );

            format!(
                "
                let mod_handles = {}.run({});
                if let Some(vec) = mod_handles {{
                    handles.extend(vec);
                }}
            ",
                name, params
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let code = format!(
        "{}\n{}\n{}",
        init_mods_glue, arc_wrappers_glue, run_mods_glue
    );
    let main_rs = MAIN_RS_TEMPLATE.replace("//PLACEHOLDER", &code);
    fs::write(project_dir.join("src").join("main.rs"), main_rs).map_err(|source| {
        PatchworkError::io(
            "write composed main.rs",
            project_dir.join("src").join("main.rs"),
            source,
        )
    })?;

    let codegen_tasks = codegen::resolve_tasks(cache_folder, mods_folder, &mods)?;
    codegen::run_tasks(
        &project_dir,
        mods_folder,
        modpacks_folder,
        modpack,
        &codegen_tasks,
    )?;
    codegen::patch_generated_crates(&project_dir, &codegen_tasks)?;

    Ok(())
}

fn canonical_cargo_path(path: &Path, action: &'static str) -> Result<String> {
    let canonical = path
        .canonicalize()
        .map_err(|source| PatchworkError::io(action, path, source))?;
    Ok(cargo_path_string(&canonical))
}

fn cargo_path_string(path: &Path) -> String {
    let path = path.to_string_lossy();

    #[cfg(windows)]
    {
        if let Some(unc_path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc_path}");
        }
        if let Some(drive_path) = path.strip_prefix(r"\\?\") {
            return drive_path.to_owned();
        }
    }

    path.into_owned()
}

fn generate_git_source_patches(mods_folder: &Path, mods: &[(String, ModInfo)]) -> Result<String> {
    let mut git_sources = BTreeSet::new();
    for (mod_name, _) in mods {
        let manifest_path = mods_folder.join(mod_name).join("Cargo.toml");
        let source = fs::read_to_string(&manifest_path).map_err(|source| {
            PatchworkError::io(
                "read mod Cargo.toml for Git patches",
                &manifest_path,
                source,
            )
        })?;
        let manifest = toml::from_str::<toml::Value>(&source).map_err(|source| {
            PatchworkError::parse_toml("mod Cargo.toml", &manifest_path, source)
        })?;
        collect_git_sources(&manifest, &mut git_sources);
    }

    if git_sources.is_empty() {
        return Ok(String::new());
    }

    let patch_lines = mods
        .iter()
        .map(|(mod_name, _)| {
            let mod_path = mods_folder.join(mod_name);
            let mod_path = canonical_cargo_path(
                &mod_path,
                "canonicalize selected mod path for Git patches",
            )?;
            Ok(format!(
                "{} = {{ path = {} }}",
                toml_string(mod_name),
                toml_string(&mod_path)
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("\n");

    Ok(git_sources
        .into_iter()
        .map(|source| format!("\n[patch.{}]\n{patch_lines}\n", toml_string(&source)))
        .collect())
}

fn collect_git_sources(value: &toml::Value, sources: &mut BTreeSet<String>) {
    match value {
        toml::Value::Table(table) => {
            if let Some(source) = table.get("git").and_then(toml::Value::as_str) {
                sources.insert(source.to_owned());
            }
            for value in table.values() {
                collect_git_sources(value, sources);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                collect_git_sources(value, sources);
            }
        }
        _ => {}
    }
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn copy_project_assets(
    template_dir: &Path,
    mods_folder: &Path,
    mods: &[(String, ModInfo)],
) -> Result<()> {
    let output_assets = template_dir.join("assets");
    fs::create_dir_all(&output_assets).map_err(|source| {
        PatchworkError::io("create composed assets directory", &output_assets, source)
    })?;

    let template_assets = PathBuf::from(TEMPLATE_ASSETS_DIR);
    if template_assets.is_dir() {
        copy_directory_contents(&template_assets, &output_assets)?;
    }

    for (modname, _) in mods {
        let mod_assets = mods_folder.join(modname).join("assets");
        if !mod_assets.is_dir() {
            continue;
        }

        let mod_output_assets = output_assets.join(modname);
        fs::create_dir_all(&mod_output_assets).map_err(|source| {
            PatchworkError::io(
                "create composed mod assets directory",
                &mod_output_assets,
                source,
            )
        })?;
        copy_directory_contents(&mod_assets, &mod_output_assets)?;
    }

    Ok(())
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)
        .map_err(|source_error| PatchworkError::io("read assets directory", source, source_error))?
    {
        let entry = entry.map_err(|source_error| {
            PatchworkError::io("read asset directory entry", source, source_error)
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source_error| {
            PatchworkError::io("read asset file type", &source_path, source_error)
        })?;

        if file_type.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|source_error| {
                PatchworkError::io("create asset directory", &destination_path, source_error)
            })?;
            copy_directory_contents(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|source_error| {
                PatchworkError::io("copy asset file", &source_path, source_error)
            })?;
        } else {
            return Err(PatchworkError::UnsupportedAssetEntry { path: source_path });
        }
    }

    Ok(())
}

fn generate_mut_params(dependencies: &[String], provider_map: &HashMap<String, String>) -> String {
    dependencies
        .iter()
        .map(|dep| {
            let modname = provider_map.get(dep).unwrap_or(dep);
            let modname = modname.replace('-', "_");
            format!("&mut {}", modname)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn generate_run_params(
    run_deps: &[String],
    ownership_deps: &[String],
    provider_map: &HashMap<String, String>,
) -> String {
    let run_params = run_deps.iter().map(|dep| {
        let modname = provider_map.get(dep).unwrap_or(dep);
        let modname = modname.replace('-', "_");
        format!("{}.clone()", modname)
    });

    let ownership_params = ownership_deps.iter().map(|dep| {
        let modname = provider_map.get(dep).unwrap_or(dep);
        modname.replace('-', "_")
    });

    run_params
        .chain(ownership_params)
        .collect::<Vec<_>>()
        .join(", ")
}
