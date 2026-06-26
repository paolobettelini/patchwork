use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// A CLI tool for managing modpacks.
#[derive(Parser, Debug)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Compose a modpack
    Compose {
        /// Modpack id inside --modpacks-folder, or a direct TOML path
        #[arg(long)]
        modpack: PathBuf,

        /// Path to the folder containing mods
        #[arg(long)]
        mods_folder: PathBuf,

        /// Path to the folder containing modpacks
        #[arg(long)]
        modpacks_folder: PathBuf,

        /// Path to the cache / temporary folder
        #[arg(long)]
        cache: PathBuf,

        /// Project name
        #[arg(long)]
        name: Option<String>,
    },
}
