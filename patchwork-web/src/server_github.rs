use std::fmt::Write as _;

use actix_web::http::header::{CACHE_CONTROL, LOCATION};
use actix_web::{HttpRequest, HttpResponse, Result, error, web};
use base64::{Engine, engine::general_purpose};
use chrono::{Duration, Utc};
use patchwork_database::{Database, DatabaseError, GithubAccount};
use patchwork_web::auth_types::{GithubAccountDto, GithubConnectRequest, GithubConnectResponse};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::github::GithubClient;
use crate::server_auth;

const GITHUB_STATE_MINUTES: i64 = 10;

#[derive(Clone)]
pub(crate) struct GithubState {
    database: Database,
    github: GithubClient,
    frontend_url: Url,
}

impl GithubState {
    pub(crate) fn new(database: Database, github: GithubClient, frontend_url: Url) -> Self {
        Self {
            database,
            github,
            frontend_url,
        }
    }
}

pub(crate) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/github/connect", web::get().to(connect))
        .route("/github/connect", web::post().to(connect_desktop))
        .route("/github/callback", web::get().to(callback))
        .route(
            "/github/installation-complete",
            web::get().to(installation_complete),
        )
        .service(
            web::resource("/github/account")
                .route(web::get().to(account))
                .route(web::delete().to(disconnect)),
        );
}

async fn connect(state: web::Data<GithubState>, request: HttpRequest) -> Result<HttpResponse> {
    let account = server_auth::authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    let completion_url = state
        .frontend_url
        .join("profile")
        .map_err(error::ErrorInternalServerError)?;
    let authorization_url = create_authorization(&state, &account.uuid, &completion_url)?;

    Ok(HttpResponse::Found()
        .insert_header((LOCATION, authorization_url))
        .insert_header((CACHE_CONTROL, "no-store"))
        .finish())
}

async fn connect_desktop(
    state: web::Data<GithubState>,
    request: HttpRequest,
    body: web::Json<GithubConnectRequest>,
) -> Result<HttpResponse> {
    let account = server_auth::authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    let completion_url = validate_desktop_completion_url(&body.completion_url)?;
    let authorization_url = create_authorization(&state, &account.uuid, &completion_url)?;

    Ok(HttpResponse::Ok()
        .insert_header((CACHE_CONTROL, "no-store"))
        .json(GithubConnectResponse { authorization_url }))
}

fn create_authorization(
    state: &GithubState,
    account_uuid: &str,
    completion_url: &Url,
) -> Result<String> {
    let oauth_state = random_urlsafe(32);
    let now = Utc::now().naive_utc();
    state
        .database
        .create_github_oauth_state(
            &sha256_hex(&oauth_state),
            account_uuid,
            completion_url.as_str(),
            now,
            now + Duration::minutes(GITHUB_STATE_MINUTES),
        )
        .map_err(to_http_error)?;
    Ok(state.github.authorization_url(&oauth_state))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    installation_id: Option<String>,
    setup_action: Option<String>,
}

async fn callback(
    state: web::Data<GithubState>,
    query: web::Query<CallbackQuery>,
) -> Result<HttpResponse> {
    let Some(oauth_state) = query.state.as_deref().filter(|value| !value.is_empty()) else {
        if query.code.is_some() || query.installation_id.is_some() || query.setup_action.is_some() {
            return Ok(installation_complete_response());
        }
        return Err(error::ErrorBadRequest("missing GitHub OAuth state"));
    };
    let stored_state = state
        .database
        .consume_github_oauth_state(&sha256_hex(oauth_state), Utc::now().naive_utc())
        .map_err(to_http_error)?
        .ok_or_else(|| error::ErrorBadRequest("invalid or expired GitHub OAuth state"))?;

    if query.error.is_some() {
        return redirect_to_completion(&stored_state.completion_url, "cancelled");
    }

    let code = query
        .code
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error::ErrorBadRequest("missing GitHub authorization code"))?;
    let github_user = state
        .github
        .exchange_code_for_user(code)
        .await
        .map_err(error::ErrorBadGateway)?;
    let account_uuid = Uuid::parse_str(&stored_state.account_uuid)
        .map_err(|_| error::ErrorInternalServerError("stored account UUID is invalid"))?;

    match state.database.link_github_account(
        account_uuid,
        github_user.id,
        &github_user.login,
        &github_user.avatar_url,
        Utc::now().naive_utc(),
    ) {
        Ok(_) => redirect_to_completion(&stored_state.completion_url, "connected"),
        Err(DatabaseError::Conflict { .. }) => {
            redirect_to_completion(&stored_state.completion_url, "already-linked")
        }
        Err(error) => Err(to_http_error(error)),
    }
}

async fn installation_complete() -> HttpResponse {
    installation_complete_response()
}

fn installation_complete_response() -> HttpResponse {
    const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>GitHub App installed | Patchwork</title>
  <style>
    :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; }
    * { box-sizing: border-box; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; padding: 24px; color: #f4f5f7; background: #111318; }
    main { width: min(560px, 100%); padding: 30px; border: 1px solid #353944; border-top: 4px solid #02a9a9; border-radius: 8px; background: #1a1d24; box-shadow: 0 18px 60px #0008; }
    .mark { display: grid; grid-template-columns: repeat(2, 17px); gap: 3px; width: max-content; margin-bottom: 22px; }
    .mark span { width: 17px; height: 17px; border-radius: 2px; }
    .mark span:nth-child(1) { background: #02a9a9; } .mark span:nth-child(2) { background: #fdb22c; }
    .mark span:nth-child(3) { background: #fd614e; } .mark span:nth-child(4) { background: #6268c8; }
    p { color: #b8bec9; line-height: 1.6; }
    h1 { margin: 0; font-size: clamp(25px, 5vw, 36px); letter-spacing: 0; }
    a { display: inline-flex; min-height: 42px; align-items: center; margin-top: 12px; padding: 0 16px; border-radius: 7px; color: #071413; background: linear-gradient(90deg, #02a9a9, #fdb22c); font-weight: 800; text-decoration: none; }
  </style>
</head>
<body>
  <main>
    <div class="mark" aria-hidden="true"><span></span><span></span><span></span><span></span></div>
    <h1>GitHub App installed</h1>
    <p>The installation is complete. You can now scan repositories on Patchwork. Patchwork will verify repository access and your write permission during the scan.</p>
    <a href="/profile">Return to Patchwork</a>
  </main>
</body>
</html>"#;

    HttpResponse::Ok()
        .insert_header((CACHE_CONTROL, "no-store"))
        .content_type("text/html; charset=utf-8")
        .body(PAGE)
}

async fn account(state: web::Data<GithubState>, request: HttpRequest) -> Result<HttpResponse> {
    let account = server_auth::authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    let account_uuid = parse_account_uuid(&account.uuid)?;
    let github = state
        .database
        .get_github_account(account_uuid)
        .map_err(to_http_error)?
        .ok_or_else(|| error::ErrorNotFound("GitHub account is not connected"))?;

    Ok(HttpResponse::Ok()
        .insert_header((CACHE_CONTROL, "no-store"))
        .json(github_account_dto(github)))
}

async fn disconnect(state: web::Data<GithubState>, request: HttpRequest) -> Result<HttpResponse> {
    let account = server_auth::authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    let account_uuid = parse_account_uuid(&account.uuid)?;
    state
        .database
        .unlink_github_account(account_uuid)
        .map_err(to_http_error)?;

    Ok(HttpResponse::NoContent()
        .insert_header((CACHE_CONTROL, "no-store"))
        .finish())
}

pub(crate) fn github_account_dto(account: GithubAccount) -> GithubAccountDto {
    GithubAccountDto {
        github_user_id: account.github_user_id,
        github_login: account.github_login,
        github_avatar_url: account.github_avatar_url,
    }
}

fn redirect_to_completion(completion_url: &str, result: &str) -> Result<HttpResponse> {
    let mut url = Url::parse(completion_url).map_err(error::ErrorInternalServerError)?;
    url.query_pairs_mut().append_pair("github", result);
    Ok(HttpResponse::Found()
        .insert_header((LOCATION, url.to_string()))
        .insert_header((CACHE_CONTROL, "no-store"))
        .finish())
}

fn validate_desktop_completion_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| error::ErrorBadRequest("invalid completionUrl"))?;
    let valid = url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some()
        && url.path() == "/github-connected"
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    if valid {
        Ok(url)
    } else {
        Err(error::ErrorBadRequest(
            "completionUrl must be http://127.0.0.1:<port>/github-connected",
        ))
    }
}

fn parse_account_uuid(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value)
        .map_err(|_| error::ErrorInternalServerError("stored account UUID is invalid"))
}

fn random_urlsafe(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    rand::rng().fill_bytes(&mut bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}

fn to_http_error(error: DatabaseError) -> actix_web::Error {
    match error {
        DatabaseError::Validation { .. } => error::ErrorBadRequest(error.to_string()),
        DatabaseError::Conflict { .. } => error::ErrorConflict(error.to_string()),
        DatabaseError::NotFound { .. } => error::ErrorNotFound(error.to_string()),
        other => error::ErrorInternalServerError(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_desktop_completion_url;

    #[test]
    fn accepts_only_the_desktop_loopback_completion_url() {
        let valid =
            validate_desktop_completion_url("http://127.0.0.1:51342/github-connected").unwrap();
        assert_eq!(valid.port(), Some(51342));

        for invalid in [
            "https://127.0.0.1:51342/github-connected",
            "http://localhost:51342/github-connected",
            "http://127.0.0.1/github-connected",
            "http://127.0.0.1:51342/callback",
            "http://127.0.0.1:51342/github-connected?next=https://example.com",
        ] {
            assert!(
                validate_desktop_completion_url(invalid).is_err(),
                "accepted invalid completion URL: {invalid}"
            );
        }
    }
}
