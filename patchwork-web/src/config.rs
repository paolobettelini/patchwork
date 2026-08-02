use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use url::Url;

const DEFAULT_ADDRESS: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;

#[derive(Clone)]
pub(crate) struct ServerConfig {
    pub(crate) address: String,
    pub(crate) port: u16,
    pub(crate) db_connection: String,
    pub(crate) frontend_url: Url,
    pub(crate) email: EmailConfig,
    pub(crate) github: GithubConfig,
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

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ConfigFile {
    server: ServerConfigFile,
    email: EmailConfigFile,
    github: GithubConfigFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ServerConfigFile {
    address: Option<String>,
    port: Option<u16>,
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

impl ServerConfig {
    pub(crate) fn load(
        path: &Path,
        address_override: Option<String>,
        port_override: Option<u16>,
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

        let frontend_url = parse_web_url("frontend-url", &file.server.frontend_url)?;
        let callback_url = parse_web_url("github.callback_url", &file.github.callback_url)?;
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

        Ok(Self {
            address,
            port: port_override.or(file.server.port).unwrap_or(DEFAULT_PORT),
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
        })
    }
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
    fn cli_address_and_port_override_file_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("patchwork.toml");
        fs::write(
            &path,
            r#"
[server]
address = "127.0.0.1"
port = 7000
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
"#,
        )
        .unwrap();

        let config = ServerConfig::load(&path, Some("0.0.0.0".to_owned()), Some(8080)).unwrap();

        assert_eq!(config.address, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.frontend_url.as_str(), "http://localhost:3000/");
        assert_eq!(config.email.resend_api_key, "resend-key");
        assert_eq!(
            config.github.private_key_path,
            directory.path().join("./github-app.pem")
        );
    }
}
