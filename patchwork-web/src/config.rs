use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use url::Url;

const DEFAULT_ADDRESS: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_BASE_PATH: &str = "/";

#[derive(Clone)]
pub(crate) struct ServerConfig {
    pub(crate) address: String,
    pub(crate) port: u16,
    pub(crate) base_path: String,
    pub(crate) db_connection: String,
    pub(crate) frontend_url: Url,
    pub(crate) email: EmailConfig,
    pub(crate) github: GithubConfig,
    pub(crate) game_auth: GameAuthConfig,
}

#[derive(Clone)]
pub(crate) struct EmailConfig {
    pub(crate) resend_api_key: String,
}

#[derive(Clone)]
pub(crate) struct GithubConfig {
    pub(crate) app_id: u64,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) private_key_path: PathBuf,
    pub(crate) callback_url: Url,
}

#[derive(Clone)]
pub(crate) struct GameAuthConfig {
    pub(crate) process_session_hours: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ConfigFile {
    server: ServerConfigFile,
    email: EmailConfigFile,
    github: GithubConfigFile,
    #[serde(default)]
    game_auth: GameAuthConfigFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ServerConfigFile {
    address: Option<String>,
    port: Option<u16>,
    base_path: Option<String>,
    db_connection: String,
    frontend_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailConfigFile {
    #[serde(rename = "RESEND_API_KEY")]
    resend_api_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GithubConfigFile {
    app_id: u64,
    client_id: String,
    client_secret: String,
    private_key_path: PathBuf,
    callback_url: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct GameAuthConfigFile {
    process_session_hours: Option<i64>,
}

impl ServerConfig {
    pub(crate) fn load(
        path: &Path,
        address_override: Option<String>,
        port_override: Option<u16>,
        base_path_override: Option<String>,
    ) -> Result<Self, String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("failed to read config `{}`: {error}", path.display()))?;
        let file = toml::from_str::<ConfigFile>(&contents)
            .map_err(|error| format!("invalid config `{}`: {error}", path.display()))?;

        let address = address_override
            .or(file.server.address)
            .unwrap_or_else(|| DEFAULT_ADDRESS.to_owned());
        if address.trim().is_empty() {
            return Err("address cannot be empty".to_owned());
        }
        if file.server.db_connection.trim().is_empty() {
            return Err("db-connection cannot be empty".to_owned());
        }
        if file.email.resend_api_key.trim().is_empty() {
            return Err("email.RESEND_API_KEY cannot be empty".to_owned());
        }

        let base_path = normalize_base_path(
            base_path_override
                .or(file.server.base_path)
                .as_deref()
                .unwrap_or(DEFAULT_BASE_PATH),
        )?;
        let mut frontend_url = parse_web_url("frontend-url", &file.server.frontend_url)?;
        frontend_url.set_path(&base_href(&base_path));
        let mut callback_url = parse_web_url("github.callback_url", &file.github.callback_url)?;
        if !callback_url.path().ends_with("/github/callback") {
            return Err("github.callback_url must end with /github/callback".to_owned());
        }
        callback_url.set_path(&prefixed_route(&base_path, "/github/callback"));
        let private_key_path = resolve_relative_path(path, &file.github.private_key_path);

        if file.github.app_id == 0 {
            return Err("github.app_id must be greater than zero".to_owned());
        }
        if file.github.client_id.trim().is_empty() {
            return Err("github.client_id cannot be empty".to_owned());
        }
        if file.github.client_secret.trim().is_empty() {
            return Err("github.client_secret cannot be empty".to_owned());
        }

        let process_session_hours = file.game_auth.process_session_hours.unwrap_or(8);
        if !(1..=168).contains(&process_session_hours) {
            return Err("game-auth.process-session-hours must be between 1 and 168".to_owned());
        }

        Ok(Self {
            address,
            port: port_override.or(file.server.port).unwrap_or(DEFAULT_PORT),
            base_path,
            db_connection: file.server.db_connection,
            frontend_url,
            email: EmailConfig {
                resend_api_key: file.email.resend_api_key,
            },
            github: GithubConfig {
                app_id: file.github.app_id,
                client_id: file.github.client_id,
                client_secret: file.github.client_secret,
                private_key_path,
                callback_url,
            },
            game_auth: GameAuthConfig {
                process_session_hours,
            },
        })
    }
}

pub(crate) fn prefixed_route(base_path: &str, route: &str) -> String {
    if base_path == "/" {
        format!("/{}", route.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, route.trim_start_matches('/'))
    }
}

pub(crate) fn base_href(base_path: &str) -> String {
    if base_path == "/" {
        "/".to_owned()
    } else {
        format!("{base_path}/")
    }
}

fn normalize_base_path(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("base-path cannot be empty".to_owned());
    }
    if !value.starts_with('/') {
        return Err("base-path must start with /".to_owned());
    }
    if value.contains(['?', '#', '\\']) || value.chars().any(char::is_control) {
        return Err("base-path must be a plain URL path without query or fragment".to_owned());
    }
    if value.contains(['{', '}']) {
        return Err("base-path cannot contain Actix route pattern characters".to_owned());
    }
    if value == "/" {
        return Ok(DEFAULT_BASE_PATH.to_owned());
    }

    let trimmed = value.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("base-path cannot contain only repeated slashes".to_owned());
    }
    if trimmed
        .split('/')
        .skip(1)
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err("base-path cannot contain empty, . or .. segments".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn parse_web_url(field: &str, value: &str) -> Result<Url, String> {
    let mut url = Url::parse(value.trim()).map_err(|error| format!("invalid {field}: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{field} must use http or https"));
    }
    if url.host_str().is_none() {
        return Err(format!("{field} must include a host"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{field} must not include a query or fragment"));
    }

    if field == "frontend-url" && !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn resolve_relative_path(config_path: &Path, value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_owned()
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_server_values_override_file_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("patchwork.toml");
        fs::write(
            &path,
            r#"
[server]
address = "127.0.0.1"
port = 7000
base-path = "/from-file"
db-connection = "patchwork.sqlite"
frontend-url = "http://localhost:3000"

[email]
RESEND_API_KEY = "resend-key"

[github]
app_id = 123456
client_id = "client"
client_secret = "secret"
private_key_path = "./github-app.pem"
callback_url = "http://localhost:8080/github/callback"

[game-auth]
process-session-hours = 24
"#,
        )
        .unwrap();

        let config = ServerConfig::load(
            &path,
            Some("0.0.0.0".to_owned()),
            Some(8080),
            Some("/registry/".to_owned()),
        )
        .unwrap();

        let file_config = ServerConfig::load(&path, None, None, None).unwrap();
        assert_eq!(file_config.base_path, "/from-file");
        assert_eq!(file_config.address, "127.0.0.1");
        assert_eq!(file_config.port, 7000);

        assert_eq!(config.address, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.base_path, "/registry");
        assert_eq!(
            config.frontend_url.as_str(),
            "http://localhost:3000/registry/"
        );
        assert_eq!(
            config.github.callback_url.as_str(),
            "http://localhost:8080/registry/github/callback"
        );
        assert_eq!(config.email.resend_api_key, "resend-key");
        assert_eq!(config.game_auth.process_session_hours, 24);
        assert_eq!(
            config.github.private_key_path,
            directory.path().join("./github-app.pem")
        );
    }

    #[test]
    fn normalizes_and_validates_base_paths() {
        assert_eq!(normalize_base_path("/").unwrap(), "/");
        assert_eq!(normalize_base_path("/patchwork/").unwrap(), "/patchwork");
        assert!(normalize_base_path("patchwork").is_err());
        assert!(normalize_base_path("/patchwork//web").is_err());
        assert!(normalize_base_path("/../patchwork").is_err());
        assert!(normalize_base_path("//").is_err());
        assert!(normalize_base_path("/{tenant}").is_err());
    }
}
