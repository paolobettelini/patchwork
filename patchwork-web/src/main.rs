use std::path::PathBuf;

use actix_files::{Files, NamedFile};
use actix_web::{App, HttpServer, Result, web};
use clap::Parser;
use patchwork_database::Database;

mod config;
mod email;
mod github;
mod server_auth;
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

    #[arg(long, env = "PATCHWORK_WEB_SITE_ROOT", default_value = "dist")]
    site_root: PathBuf,

    #[arg(long, env = "PATCHWORK_SECURE_COOKIES", default_value_t = false)]
    secure_cookies: bool,
}

#[derive(Clone)]
struct SiteFiles {
    root: PathBuf,
    index: PathBuf,
}

async fn stylesheet(files: web::Data<SiteFiles>) -> Result<NamedFile> {
    Ok(NamedFile::open(files.root.join("styles.css"))?)
}

async fn logo(files: web::Data<SiteFiles>) -> Result<NamedFile> {
    Ok(NamedFile::open(files.root.join("logo.png"))?)
}

async fn wasm(files: web::Data<SiteFiles>) -> Result<NamedFile> {
    Ok(NamedFile::open(files.root.join("pkg/patchwork_web.wasm"))?)
}

async fn fallback(files: web::Data<SiteFiles>) -> Result<NamedFile> {
    Ok(NamedFile::open(&files.index)?)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let config = config::ServerConfig::load(&args.config, args.address, args.port)
        .map_err(std::io::Error::other)?;
    let site_root = args.site_root;
    let dist_index = site_root.join("index.html");
    let source_index = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("index.html");
    let index = if dist_index.is_file() {
        dist_index
    } else {
        source_index
    };
    let bind_address = format!("{}:{}", config.address, config.port);
    let files = SiteFiles {
        root: site_root,
        index,
    };
    let database = Database::connect(&config.db_connection)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let email = email::EmailSender::new(config.email);
    let auth_state = server_auth::AuthState::new(database, email, args.secure_cookies);
    let github = github::GithubClient::new(config.github).map_err(std::io::Error::other)?;
    let github_state = server_github::GithubState::new(
        auth_state.database().clone(),
        github.clone(),
        config.frontend_url,
    );
    let registry_state = server_registry::RegistryState::new(auth_state.database().clone(), github);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(files.clone()))
            .app_data(web::Data::new(auth_state.clone()))
            .app_data(web::Data::new(github_state.clone()))
            .app_data(web::Data::new(registry_state.clone()))
            .configure(server_auth::configure)
            .configure(server_github::configure)
            .configure(server_registry::configure)
            .route("/pkg/patchwork_web_bg.wasm", web::route().to(wasm))
            .service(Files::new("/pkg", files.root.join("pkg")))
            .route("/styles.css", web::get().to(stylesheet))
            .route("/logo.png", web::get().to(logo))
            .route("/", web::get().to(fallback))
            .default_service(web::route().to(fallback))
    })
    .bind(bind_address)?
    .run()
    .await
}
