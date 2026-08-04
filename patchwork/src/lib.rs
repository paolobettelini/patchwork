use std::path::Path;

mod browser;
mod codegen;
mod error;
mod graph;
mod model;
mod modpacks;
mod paths;
mod project;
mod registry;

pub use browser::{
    DependencyDiagnostic, DependencyEntry, DependencyPage, DependencyPageKind, DependencyTarget,
    inspect_dependency_page,
};
pub use error::{PatchworkError, Result};
pub use model::{
    CodegenDeclaration, CodegenGenerator, Dependencies, ModInfo, Modpack, ProcessOptions,
    ProfileOptions, is_generated_mod_id,
};
pub use registry::{
    RegistryDependencyTargetKind, RegistryModManifest, RegistryModpackDependency,
    RegistryModpackManifest, RegistryWorkspaceManifest, parse_registry_dependency,
    parse_registry_mod_manifest, parse_registry_modpack_manifest,
};

pub fn compose_with_modpacks<P: AsRef<Path>, Q: AsRef<Path>, R: AsRef<Path>, S: AsRef<Path>>(
    modpack: P,
    project_name: Option<String>,
    mods_folder: Q,
    modpacks_folder: R,
    cache_folder: S,
) -> Result<()> {
    let mods_folder = mods_folder.as_ref().canonicalize().map_err(|source| {
        PatchworkError::io("canonicalize mods folder", mods_folder.as_ref(), source)
    })?;
    let modpacks_folder = modpacks_folder.as_ref().canonicalize().map_err(|source| {
        PatchworkError::io(
            "canonicalize modpacks folder",
            modpacks_folder.as_ref(),
            source,
        )
    })?;
    let cache_folder = paths::absolutize(cache_folder.as_ref())?;
    let modpack_path = modpacks::resolve_modpack_path(modpack.as_ref(), &modpacks_folder)?;

    let loaded = modpacks::load_mods(&modpack_path, &mods_folder, &modpacks_folder)?;
    let graph = graph::resolve(loaded.mods)?;

    let project_name = if let Some(name) = project_name {
        name
    } else {
        modpack_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| PatchworkError::MissingProjectName {
                modpack_path: modpack_path.clone(),
            })?
            .to_owned()
    };

    project::create_project(
        &cache_folder,
        &project_name,
        &mods_folder,
        &modpacks_folder,
        &modpack_path,
        graph.mods,
        graph.provider_map,
        graph.owned_objects,
    )?;

    Ok(())
}
