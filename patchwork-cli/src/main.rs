use clap::Parser;
use log::error;
use std::{
    fs,
    path::{Path, PathBuf},
};

mod cli;
use cli::*;

fn main() {
    init_log();

    let cli = Cli::parse();

    match cli.command {
        Commands::Compose {
            modpack,
            mods_folder,
            modpacks_folder,
            cache,
            name,
            development,
            ignore,
        } => {
            if let Err(e) = compose(
                modpack,
                name,
                mods_folder,
                modpacks_folder,
                cache,
                development,
                ignore,
            ) {
                error!("Error composing modpack: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn compose(
    modpack: PathBuf,
    name: Option<String>,
    mods_folder: PathBuf,
    modpacks_folder: PathBuf,
    cache: PathBuf,
    development: Option<PathBuf>,
    ignore: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = development
        .as_ref()
        .map(patchwork::DevelopmentConfig::load)
        .transpose()?
        .unwrap_or_default();
    let name = name
        .or_else(|| {
            modpack
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .ok_or("cannot derive project name")?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("project name may contain only letters, digits, '-' and '_'".into());
    }
    let mut root = resolve_root(&modpack, &modpacks_folder, &config)?;
    if !ignore.is_empty() {
        let mut profile: patchwork::Modpack = toml::from_str(&fs::read_to_string(&root)?)?;
        profile.ignore.extend(ignore);
        let directory = cache.join(".profiles").join(&name);
        fs::create_dir_all(&directory)?;
        root = directory.join("profile.toml");
        fs::write(&root, toml::to_string_pretty(&profile)?)?;
    }
    let prepared = patchwork::prepare_development_for_modpack(
        &config,
        Path::new("."),
        &mods_folder,
        &modpacks_folder,
        &cache.join(".development").join(&name),
        &root,
    )?;
    patchwork::compose_with_modpacks(
        root,
        Some(name),
        prepared.mods_folder,
        prepared.modpacks_folder,
        cache,
    )?;
    Ok(())
}

fn resolve_root(
    modpack: &Path,
    folder: &Path,
    config: &patchwork::DevelopmentConfig,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if modpack.is_file() {
        return Ok(modpack.canonicalize()?);
    }
    let id = modpack
        .to_str()
        .ok_or("modpack ID must be UTF-8")?
        .trim_end_matches(".toml");
    if config.enabled {
        if let Some(path) = config.modpacks.get(id) {
            return Ok(path.clone());
        }
    }
    for path in [folder.join(modpack), folder.join(format!("{id}.toml"))] {
        if path.is_file() {
            return Ok(path.canonicalize()?);
        }
    }
    Err(format!(
        "modpack '{}' not found in {}",
        modpack.display(),
        folder.display()
    )
    .into())
}

const LOG_ENV: &str = "LOG";

fn init_log() {
    if std::env::var(LOG_ENV).is_err() {
        unsafe {
            std::env::set_var(LOG_ENV, "info");
        }
    }
    env_logger::init();
}
