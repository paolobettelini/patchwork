use std::{fs, sync::Arc};

use base64::Engine;
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::GithubConfig;

const GITHUB_API_URL: &str = "https://api.github.com";
const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_API_VERSION: &str = "2026-03-10";
const USER_AGENT: &str = "Patchwork-Web";

#[derive(Clone)]
pub(crate) struct GithubClient {
    app_id: u64,
    client_id: String,
    client_secret: Arc<str>,
    callback_url: Url,
    private_key: Arc<EncodingKey>,
    http: Client,
}

#[derive(Clone, Debug)]
pub(crate) struct GithubUser {
    pub(crate) id: i64,
    pub(crate) login: String,
    pub(crate) avatar_url: String,
}

pub(crate) struct InstallationAccessToken {
    pub(crate) token: String,
    #[allow(dead_code)]
    pub(crate) expires_at: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthorizedGithubRepository {
    pub(crate) id: i64,
    pub(crate) owner: String,
    pub(crate) name: String,
    pub(crate) canonical_url: String,
    pub(crate) default_branch: String,
    #[allow(dead_code)]
    pub(crate) installation_id: i64,
    pub(crate) access_token: String,
    pub(crate) github_user: GithubUser,
}

#[derive(Clone, Debug)]
pub(crate) struct GithubCommit {
    pub(crate) sha: String,
    pub(crate) tree_sha: String,
}

#[derive(Clone, Debug)]
pub(crate) struct GithubTree {
    pub(crate) sha: String,
    pub(crate) entries: Vec<GithubTreeEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct GithubTreeEntry {
    pub(crate) path: String,
    pub(crate) kind: String,
    pub(crate) mode: String,
    pub(crate) sha: String,
    pub(crate) size: Option<u64>,
}

#[derive(Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Deserialize)]
struct UserTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct GithubUserResponse {
    id: u64,
    login: String,
    avatar_url: String,
}

#[derive(Deserialize)]
struct InstallationResponse {
    id: u64,
}

#[derive(Deserialize)]
struct RepositoryResponse {
    id: u64,
    name: String,
    owner: GithubRepositoryOwner,
    html_url: String,
    default_branch: String,
}

#[derive(Deserialize)]
struct GithubRepositoryOwner {
    login: String,
}

#[derive(Deserialize)]
struct CollaboratorPermissionResponse {
    permission: String,
    user: GithubUserResponse,
}

#[derive(Deserialize)]
struct CommitResponse {
    sha: String,
    commit: CommitDetails,
}

#[derive(Deserialize)]
struct CommitDetails {
    tree: CommitTree,
}

#[derive(Deserialize)]
struct CommitTree {
    sha: String,
}

#[derive(Deserialize)]
struct TreeResponse {
    sha: String,
    #[serde(default)]
    truncated: bool,
    tree: Vec<TreeEntryResponse>,
}

#[derive(Deserialize)]
struct TreeEntryResponse {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    mode: String,
    sha: String,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct BlobResponse {
    content: String,
    encoding: String,
    size: u64,
}

#[derive(Serialize)]
struct InstallationTokenRequest<'a> {
    #[serde(skip_serializing_if = "slice_is_empty")]
    repository_ids: &'a [i64],
}

#[derive(Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: String,
}

impl GithubClient {
    pub(crate) fn new(config: GithubConfig) -> Result<Self, String> {
        let private_key = fs::read(&config.private_key_path).map_err(|error| {
            format!(
                "failed to read GitHub App private key `{}`: {error}",
                config.private_key_path.display()
            )
        })?;
        let private_key = EncodingKey::from_rsa_pem(&private_key)
            .map_err(|error| format!("invalid GitHub App private key: {error}"))?;
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| format!("failed to build GitHub HTTP client: {error}"))?;

        Ok(Self {
            app_id: config.app_id,
            client_id: config.client_id,
            client_secret: Arc::from(config.client_secret),
            callback_url: config.callback_url,
            private_key: Arc::new(private_key),
            http,
        })
    }

    pub(crate) fn authorization_url(&self, state: &str) -> String {
        let mut url = Url::parse(GITHUB_AUTHORIZE_URL).expect("static GitHub URL is valid");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", self.callback_url.as_str())
            .append_pair("state", state);
        url.to_string()
    }

    pub(crate) async fn exchange_code_for_user(&self, code: &str) -> Result<GithubUser, String> {
        let response = self
            .http
            .post(GITHUB_TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_ref()),
                ("code", code),
                ("redirect_uri", self.callback_url.as_str()),
            ])
            .send()
            .await
            .map_err(|error| format!("GitHub token exchange failed: {error}"))?;
        let status = response.status();
        let token = response
            .json::<UserTokenResponse>()
            .await
            .map_err(|error| format!("invalid GitHub token response: {error}"))?;
        if !status.is_success() || token.access_token.is_none() {
            let reason = token
                .error_description
                .or(token.error)
                .unwrap_or_else(|| status.to_string());
            return Err(format!("GitHub rejected the authorization code: {reason}"));
        }

        self.current_user(&token.access_token.expect("checked above"))
            .await
    }

    pub(crate) fn app_jwt(&self) -> Result<String, String> {
        let now = Utc::now().timestamp();
        let claims = AppClaims {
            iat: now - 60,
            exp: now + 9 * 60,
            iss: self.app_id.to_string(),
        };
        encode(
            &Header::new(Algorithm::RS256),
            &claims,
            self.private_key.as_ref(),
        )
        .map_err(|error| format!("failed to sign GitHub App JWT: {error}"))
    }

    pub(crate) async fn installation_access_token(
        &self,
        installation_id: i64,
        repository_ids: &[i64],
    ) -> Result<InstallationAccessToken, String> {
        if installation_id <= 0 || repository_ids.iter().any(|id| *id <= 0) {
            return Err("GitHub installation and repository IDs must be positive".to_owned());
        }
        if repository_ids.len() > 500 {
            return Err("GitHub installation tokens support at most 500 repositories".to_owned());
        }

        let url = format!("{GITHUB_API_URL}/app/installations/{installation_id}/access_tokens");
        let response = self
            .http
            .post(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .bearer_auth(self.app_jwt()?)
            .json(&InstallationTokenRequest { repository_ids })
            .send()
            .await
            .map_err(|error| format!("GitHub installation token request failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("GitHub installation token request failed: {error}"))?
            .json::<InstallationTokenResponse>()
            .await
            .map_err(|error| format!("invalid GitHub installation token response: {error}"))?;

        Ok(InstallationAccessToken {
            token: response.token,
            expires_at: response.expires_at,
        })
    }

    pub(crate) async fn authorize_repository(
        &self,
        owner: &str,
        repository: &str,
        github_user_id: i64,
    ) -> Result<AuthorizedGithubRepository, String> {
        if github_user_id <= 0 {
            return Err("linked GitHub user ID must be positive".to_owned());
        }
        validate_repository_coordinate("owner", owner)?;
        validate_repository_coordinate("repository", repository)?;

        let installation_url = format!("{GITHUB_API_URL}/repos/{owner}/{repository}/installation");
        let installation = self
            .http
            .get(installation_url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .bearer_auth(self.app_jwt()?)
            .send()
            .await
            .map_err(|error| format!("GitHub installation lookup failed: {error}"))?;
        if installation.status() == StatusCode::NOT_FOUND {
            return Err("Patchwork's GitHub App is not installed for this repository.".to_owned());
        }
        let installation = installation
            .error_for_status()
            .map_err(|error| format!("GitHub installation lookup failed: {error}"))?
            .json::<InstallationResponse>()
            .await
            .map_err(|error| format!("invalid GitHub installation response: {error}"))?;
        let installation_id = i64::try_from(installation.id)
            .map_err(|_| "GitHub installation ID exceeds the supported range".to_owned())?;

        let broad_token = self.installation_access_token(installation_id, &[]).await?;
        let repository_data = self
            .repository(owner, repository, &broad_token.token)
            .await?;
        let github_user = self.user_by_id(github_user_id, &broad_token.token).await?;
        let permission = self
            .repository_permission(
                &repository_data.owner.login,
                &repository_data.name,
                &github_user.login,
                &broad_token.token,
            )
            .await?;
        if permission.user.id != u64::try_from(github_user_id).unwrap_or(u64::MAX) {
            return Err(
                "GitHub returned a collaborator with a different numeric user ID".to_owned(),
            );
        }
        if !matches!(
            permission.permission.as_str(),
            "write" | "maintain" | "admin"
        ) {
            return Err(format!(
                "GitHub account @{} has '{}' permission; write, maintain, or admin is required",
                github_user.login, permission.permission
            ));
        }

        let id = i64::try_from(repository_data.id)
            .map_err(|_| "GitHub repository ID exceeds the supported range".to_owned())?;
        let scoped_token = self
            .installation_access_token(installation_id, &[id])
            .await?;

        Ok(AuthorizedGithubRepository {
            id,
            owner: repository_data.owner.login,
            name: repository_data.name,
            canonical_url: repository_data.html_url,
            default_branch: repository_data.default_branch,
            installation_id,
            access_token: scoped_token.token,
            github_user,
        })
    }

    pub(crate) async fn resolve_commit(
        &self,
        repository: &AuthorizedGithubRepository,
        reference: &str,
    ) -> Result<GithubCommit, String> {
        let reference = reference.trim();
        if reference.is_empty() || reference.len() > 255 || reference.chars().any(char::is_control)
        {
            return Err("GitHub ref must contain between 1 and 255 characters".to_owned());
        }
        let url = api_url(&[
            "repos",
            &repository.owner,
            &repository.name,
            "commits",
            reference,
        ])?;
        let response = self
            .authorized_get(url, &repository.access_token)
            .await?
            .json::<CommitResponse>()
            .await
            .map_err(|error| format!("invalid GitHub commit response: {error}"))?;
        Ok(GithubCommit {
            sha: response.sha,
            tree_sha: response.commit.tree.sha,
        })
    }

    pub(crate) async fn tree(
        &self,
        repository: &AuthorizedGithubRepository,
        tree_sha: &str,
    ) -> Result<GithubTree, String> {
        validate_git_oid(tree_sha)?;
        let url = api_url(&[
            "repos",
            &repository.owner,
            &repository.name,
            "git",
            "trees",
            tree_sha,
        ])?;
        let response = self
            .authorized_get(url, &repository.access_token)
            .await?
            .json::<TreeResponse>()
            .await
            .map_err(|error| format!("invalid GitHub tree response: {error}"))?;
        if response.truncated {
            return Err(
                "GitHub truncated a tree response; the repository directory is too large to scan"
                    .to_owned(),
            );
        }
        Ok(GithubTree {
            sha: response.sha,
            entries: response
                .tree
                .into_iter()
                .map(|entry| GithubTreeEntry {
                    path: entry.path,
                    kind: entry.kind,
                    mode: entry.mode,
                    sha: entry.sha,
                    size: entry.size,
                })
                .collect(),
        })
    }

    pub(crate) async fn recursive_tree(
        &self,
        repository: &AuthorizedGithubRepository,
        tree_sha: &str,
    ) -> Result<GithubTree, String> {
        validate_git_oid(tree_sha)?;
        let mut url = api_url(&[
            "repos",
            &repository.owner,
            &repository.name,
            "git",
            "trees",
            tree_sha,
        ])?;
        url.query_pairs_mut().append_pair("recursive", "1");
        let response = self
            .authorized_get(url, &repository.access_token)
            .await?
            .json::<TreeResponse>()
            .await
            .map_err(|error| format!("invalid GitHub recursive tree response: {error}"))?;
        if response.truncated {
            return Err(
                "GitHub truncated the recursive tree response; narrow the scan to a subdirectory"
                    .to_owned(),
            );
        }
        Ok(GithubTree {
            sha: response.sha,
            entries: response
                .tree
                .into_iter()
                .map(|entry| GithubTreeEntry {
                    path: entry.path,
                    kind: entry.kind,
                    mode: entry.mode,
                    sha: entry.sha,
                    size: entry.size,
                })
                .collect(),
        })
    }

    pub(crate) async fn blob(
        &self,
        repository: &AuthorizedGithubRepository,
        blob_sha: &str,
        maximum_size: u64,
    ) -> Result<Vec<u8>, String> {
        validate_git_oid(blob_sha)?;
        let url = api_url(&[
            "repos",
            &repository.owner,
            &repository.name,
            "git",
            "blobs",
            blob_sha,
        ])?;
        let response = self
            .authorized_get(url, &repository.access_token)
            .await?
            .json::<BlobResponse>()
            .await
            .map_err(|error| format!("invalid GitHub blob response: {error}"))?;
        if response.size > maximum_size {
            return Err(format!(
                "GitHub blob is {} bytes, above the {maximum_size} byte scan limit",
                response.size
            ));
        }
        if response.encoding != "base64" {
            return Err(format!(
                "unsupported GitHub blob encoding '{}'",
                response.encoding
            ));
        }
        let compact = response
            .content
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|error| format!("invalid GitHub blob Base64: {error}"))
            .and_then(|bytes| {
                if bytes.len() as u64 > maximum_size {
                    Err(format!(
                        "decoded GitHub blob is {} bytes, above the {maximum_size} byte scan limit",
                        bytes.len()
                    ))
                } else {
                    Ok(bytes)
                }
            })
    }

    async fn current_user(&self, access_token: &str) -> Result<GithubUser, String> {
        let response = self
            .http
            .get(format!("{GITHUB_API_URL}/user"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| format!("GitHub user request failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("GitHub user request failed: {error}"))?
            .json::<GithubUserResponse>()
            .await
            .map_err(|error| format!("invalid GitHub user response: {error}"))?;
        let id = i64::try_from(response.id)
            .map_err(|_| "GitHub user ID exceeds the supported range".to_owned())?;

        Ok(GithubUser {
            id,
            login: response.login,
            avatar_url: response.avatar_url,
        })
    }

    async fn repository(
        &self,
        owner: &str,
        repository: &str,
        access_token: &str,
    ) -> Result<RepositoryResponse, String> {
        let url = api_url(&["repos", owner, repository])?;
        self.authorized_get(url, access_token)
            .await?
            .json::<RepositoryResponse>()
            .await
            .map_err(|error| format!("invalid GitHub repository response: {error}"))
    }

    async fn user_by_id(
        &self,
        github_user_id: i64,
        access_token: &str,
    ) -> Result<GithubUser, String> {
        let url = api_url(&["user", &github_user_id.to_string()])?;
        let response = self
            .authorized_get(url, access_token)
            .await?
            .json::<GithubUserResponse>()
            .await
            .map_err(|error| format!("invalid GitHub user response: {error}"))?;
        let id = i64::try_from(response.id)
            .map_err(|_| "GitHub user ID exceeds the supported range".to_owned())?;
        if id != github_user_id {
            return Err("GitHub user lookup returned a different numeric ID".to_owned());
        }
        Ok(GithubUser {
            id,
            login: response.login,
            avatar_url: response.avatar_url,
        })
    }

    async fn repository_permission(
        &self,
        owner: &str,
        repository: &str,
        login: &str,
        access_token: &str,
    ) -> Result<CollaboratorPermissionResponse, String> {
        let url = api_url(&[
            "repos",
            owner,
            repository,
            "collaborators",
            login,
            "permission",
        ])?;
        self.authorized_get(url, access_token)
            .await?
            .json::<CollaboratorPermissionResponse>()
            .await
            .map_err(|error| format!("invalid GitHub collaborator permission response: {error}"))
    }

    async fn authorized_get(
        &self,
        url: Url,
        access_token: &str,
    ) -> Result<reqwest::Response, String> {
        self.http
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| format!("GitHub API request failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("GitHub API request failed: {error}"))
    }
}

fn slice_is_empty<T>(values: &&[T]) -> bool {
    values.is_empty()
}

fn api_url(segments: &[&str]) -> Result<Url, String> {
    let mut url = Url::parse(GITHUB_API_URL).expect("static GitHub URL is valid");
    url.path_segments_mut()
        .map_err(|_| "GitHub API base URL cannot accept path segments".to_owned())?
        .extend(segments);
    Ok(url)
}

fn validate_repository_coordinate(field: &str, value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!("invalid GitHub repository {field}"))
    }
}

fn validate_git_oid(value: &str) -> Result<(), String> {
    if (40..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("invalid Git object ID returned by GitHub".to_owned())
    }
}
