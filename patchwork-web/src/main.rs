use std::path::PathBuf;

use actix_web::{App, HttpServer, web};
use clap::Parser;
use patchwork_database::Database;

mod assets;
mod config;
mod email;
mod github;
mod server_auth;
mod server_game_auth;
mod server_github;
mod server_registry;

#[derive(Debug, Parser)]
#[command(about = "Serve the Patchwork web site.")]
struct Args {
    #[arg(long, value_name = "PATH")]
    config: PathBuf,

    #[arg(long)]
    address: Option<String>,

    #[arg(long)]
    port: Option<u16>,

    #[arg(long, value_name = "PATH")]
    base_path: Option<String>,

    #[arg(long, env = "PATCHWORK_WEB_SITE_ROOT", default_value = "dist")]
    site_root: PathBuf,

    #[arg(long, env = "PATCHWORK_SECURE_COOKIES", default_value_t = false)]
    secure_cookies: bool,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let config = config::ServerConfig::load(&args.config, args.address, args.port, args.base_path)
        .map_err(std::io::Error::other)?;
    let bind_address = format!("{}:{}", config.address, config.port);
    let frontend_assets = assets::FrontendAssets::new(args.site_root, config.base_path.clone());
    let database = Database::connect(&config.db_connection)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let email = email::EmailSender::new(config.email.clone());
    let auth_state = server_auth::AuthState::new(
        database,
        email,
        args.secure_cookies,
        config.base_path.clone(),
    );
    let game_auth_state =
        server_game_auth::GameAuthState::new(auth_state.database().clone(), &config.game_auth);
    server_game_auth::spawn_cleanup(game_auth_state.clone());
    let github = github::GithubClient::new(config.github.clone()).map_err(std::io::Error::other)?;
    let github_state = server_github::GithubState::new(
        auth_state.database().clone(),
        github.clone(),
        config.frontend_url,
    );
    let registry_state = server_registry::RegistryState::new(
        auth_state.database().clone(),
        github,
        config.base_path.clone(),
    );
    let route_scope = if config.base_path == "/" {
        String::new()
    } else {
        config.base_path
    };

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(frontend_assets.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(github_state.clone()))
            .app_data(web::Data::new(registry_state.clone()))
            .app_data(web::Data::new(game_auth_state.clone()))
            .service(
                web::scope(&route_scope)
                    .configure(server_auth::configure)
                    .configure(server_github::configure)
                    .configure(server_game_auth::configure)
                    .configure(server_registry::configure)
                    .configure(assets::configure)
                    .default_service(web::route().to(assets::fallback)),
            )
            .default_service(web::route().to(assets::outside_base_path))
    })
    .bind(bind_address)?
    .run()
    .await
}
