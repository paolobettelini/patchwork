use clap::Parser;
use log::error;

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
        } => {
            if let Err(e) =
                patchwork::compose_with_modpacks(modpack, name, mods_folder, modpacks_folder, cache)
            {
                error!("Error composing modpack: {}", e);
                std::process::exit(1);
            }
        }
    }
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
