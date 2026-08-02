use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose};
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;

use crate::model::{
    AppState, AuthProfile, GithubAccount, LauncherAuthStatus, PATCHWORK_AUTH_EVENT,
    PatchworkAuthEvent, StoredAuthState, default_auth_server_url,
};

const APP_CLIENT_ID: &str = "patchwork-app";
const OAUTH_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GITHUB_CALLBACK_PATH: &str = "/github-connected";

#[tauri::command]
pub(crate) fn auth_status(state: State<AppState>) -> Result<LauncherAuthStatus, String> {
    Ok(state
        .auth
        .lock()
        .map_err(|_| "auth lock is poisoned".to_string())?
        .status())
}

#[tauri::command]
pub(crate) fn start_oauth_login(
    app: AppHandle,
    state: State<AppState>,
    server_url: Option<String>,
) -> Result<LauncherAuthStatus, String> {
    let requested_server_url = match server_url {
        Some(server_url) => server_url,
        None => state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?
            .server_url
            .clone(),
    };
    let server_url = normalize_server_url(&requested_server_url)?;

    {
        let mut auth = state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        auth.server_url = server_url.clone();
        save_auth_state(&state.auth_path, &auth).map_err(|error| error.to_string())?;
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let redirect_uri = format!(
        "http://127.0.0.1:{}/callback",
        listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port()
    );
    let code_verifier = random_urlsafe(32);
    let code_challenge = pkce_s256(&code_verifier);
    let csrf_state = random_urlsafe(32);
    let authorize_url = authorize_url(&server_url, &redirect_uri, &code_challenge, &csrf_state)?;

    webbrowser::open(&authorize_url).map_err(|error| error.to_string())?;

    thread::spawn(move || {
        let result = complete_oauth_flow(
            &app,
            listener,
            server_url,
            redirect_uri,
            code_verifier,
            csrf_state,
        );
        if let Err(error) = result {
            let status = app
                .state::<AppState>()
                .auth
                .lock()
                .map(|auth| auth.status())
                .unwrap_or_else(|_| StoredAuthState::default().status());
            emit_auth_event(&app, status, Some(error));
        }
    });

    auth_status(state)
}

#[tauri::command]
pub(crate) fn refresh_auth_profile(state: State<AppState>) -> Result<LauncherAuthStatus, String> {
    let (server_url, token) = {
        let auth = state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        (auth.server_url.clone(), auth.access_token.clone())
    };

    let Some(token) = token else {
        return auth_status(state);
    };

    let profile = fetch_profile(&server_url, &token)?;
    let mut auth = state
        .auth
        .lock()
        .map_err(|_| "auth lock is poisoned".to_string())?;
    auth.profile = Some(profile);
    save_auth_state(&state.auth_path, &auth).map_err(|error| error.to_string())?;
    Ok(auth.status())
}

#[tauri::command]
pub(crate) fn logout_auth(state: State<AppState>) -> Result<LauncherAuthStatus, String> {
    let (server_url, token) = {
        let auth = state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        (auth.server_url.clone(), auth.access_token.clone())
    };

    if let Some(token) = token {
        let logout_url = endpoint_url(&server_url, "/api/auth/logout")?;
        let _ = ureq::post(&logout_url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
    }

    let mut auth = state
        .auth
        .lock()
        .map_err(|_| "auth lock is poisoned".to_string())?;
    auth.access_token = None;
    auth.profile = None;
    save_auth_state(&state.auth_path, &auth).map_err(|error| error.to_string())?;
    Ok(auth.status())
}

#[tauri::command]
pub(crate) fn update_auth_nickname(
    state: State<AppState>,
    nickname: String,
) -> Result<LauncherAuthStatus, String> {
    let (server_url, token) = {
        let auth = state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        (auth.server_url.clone(), auth.access_token.clone())
    };
    let token = token.ok_or_else(|| "not authenticated".to_string())?;
    let update_url = endpoint_url(&server_url, "/api/account/nickname")?;
    let profile = ureq::post(&update_url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_json(json!({ "nickname": nickname }))
        .map_err(|error| error.to_string())?
        .into_json::<AuthProfile>()
        .map_err(|error| error.to_string())?;

    let mut auth = state
        .auth
        .lock()
        .map_err(|_| "auth lock is poisoned".to_string())?;
    auth.profile = Some(profile);
    save_auth_state(&state.auth_path, &auth).map_err(|error| error.to_string())?;
    Ok(auth.status())
}

#[tauri::command]
pub(crate) fn start_github_connect(
    app: AppHandle,
    state: State<AppState>,
) -> Result<LauncherAuthStatus, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let completion_url = format!(
        "http://127.0.0.1:{}{GITHUB_CALLBACK_PATH}",
        listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port()
    );
    let authorization_url = request_github_authorization(&server_url, &token, &completion_url)?;
    webbrowser::open(&authorization_url).map_err(|error| error.to_string())?;

    thread::spawn(move || {
        if let Err(error) = complete_github_flow(&app, listener, server_url, token) {
            let status = app
                .state::<AppState>()
                .auth
                .lock()
                .map(|auth| auth.status())
                .unwrap_or_else(|_| StoredAuthState::default().status());
            emit_auth_event(&app, status, Some(error));
        }
    });

    auth_status(state)
}

#[tauri::command]
pub(crate) fn disconnect_github(
    app: AppHandle,
    state: State<AppState>,
) -> Result<LauncherAuthStatus, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    let disconnect_url = endpoint_url(&server_url, "/github/account")?;
    ureq::delete(&disconnect_url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|error| error.to_string())?;

    let profile = fetch_profile(&server_url, &token)?;
    let mut auth = state
        .auth
        .lock()
        .map_err(|_| "auth lock is poisoned".to_string())?;
    auth.profile = Some(profile);
    save_auth_state(&state.auth_path, &auth).map_err(|error| error.to_string())?;
    let status = auth.status();
    emit_auth_event(&app, status.clone(), None);
    Ok(status)
}

pub(crate) fn load_auth_state(path: &Path) -> Result<StoredAuthState, io::Error> {
    if !path.is_file() {
        return Ok(StoredAuthState::default());
    }

    let bytes = fs::read(path)?;
    let mut auth = serde_json::from_slice::<StoredAuthState>(&bytes).unwrap_or_default();
    if auth.server_url.trim().is_empty() {
        auth.server_url = default_auth_server_url();
    }
    Ok(auth)
}

pub(crate) fn save_auth_state(path: &Path, auth: &StoredAuthState) -> Result<(), io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    let json = serde_json::to_vec_pretty(auth).map_err(io::Error::other)?;
    file.write_all(&json)?;

    #[cfg(unix)]
    {
        let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }

    Ok(())
}

fn complete_oauth_flow(
    app: &AppHandle,
    listener: TcpListener,
    server_url: String,
    redirect_uri: String,
    code_verifier: String,
    csrf_state: String,
) -> Result<(), String> {
    let deadline = Instant::now() + OAUTH_TIMEOUT;
    let (mut stream, path) = loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let path = read_callback_path(&stream)?;
                break (stream, path);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("OAuth login timed out".to_string());
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.to_string()),
        }
    };

    let callback =
        Url::parse(&format!("http://127.0.0.1{path}")).map_err(|error| error.to_string())?;
    let code = callback
        .query_pairs()
        .find_map(|(key, value)| (key == "code").then(|| value.into_owned()))
        .ok_or_else(|| "OAuth callback did not include an authorization code".to_string());
    let state = callback
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()));

    if state.as_deref() != Some(&csrf_state) {
        let _ = write_loopback_response(&mut stream, false);
        return Err("OAuth callback state did not match".to_string());
    }

    let code = match code {
        Ok(code) => code,
        Err(error) => {
            let _ = write_loopback_response(&mut stream, false);
            return Err(error);
        }
    };

    let token_response = exchange_code(&server_url, &redirect_uri, &code_verifier, &code)?;
    {
        let app_state = app.state::<AppState>();
        let mut auth = app_state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        auth.server_url = server_url;
        auth.access_token = Some(token_response.access_token);
        auth.profile = Some(token_response.profile);
        save_auth_state(&app_state.auth_path, &auth).map_err(|error| error.to_string())?;
        emit_auth_event(app, auth.status(), None);
    }

    write_loopback_response(&mut stream, true).map_err(|error| error.to_string())
}

fn read_callback_path(stream: &TcpStream) -> Result<String, String> {
    read_loopback_path(stream, "/callback")
}

fn read_loopback_path(stream: &TcpStream, expected_path: &str) -> Result<String, String> {
    let mut stream = stream.try_clone().map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") || bytes.len() > 16 * 1024 {
            break;
        }
    }

    let request = String::from_utf8_lossy(&bytes);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "OAuth callback was empty".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let callback = Url::parse(&format!("http://127.0.0.1{path}"))
        .map_err(|_| "loopback callback request was invalid".to_string())?;
    if method != "GET" || callback.path() != expected_path {
        return Err("loopback callback request was invalid".to_string());
    }
    Ok(path.to_owned())
}

fn complete_github_flow(
    app: &AppHandle,
    listener: TcpListener,
    server_url: String,
    token: String,
) -> Result<(), String> {
    let deadline = Instant::now() + OAUTH_TIMEOUT;
    let (mut stream, path) = loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let path = read_loopback_path(&stream, GITHUB_CALLBACK_PATH)?;
                break (stream, path);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("GitHub connection timed out".to_string());
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.to_string()),
        }
    };

    let callback =
        Url::parse(&format!("http://127.0.0.1{path}")).map_err(|error| error.to_string())?;
    let result = callback
        .query_pairs()
        .find_map(|(key, value)| (key == "github").then(|| value.into_owned()))
        .ok_or_else(|| "GitHub callback did not include a result".to_string())?;
    if result != "connected" {
        let _ = write_github_loopback_response(&mut stream, false);
        return Err(match result.as_str() {
            "already-linked" => {
                "This GitHub account is already linked to another Patchwork account".to_string()
            }
            "cancelled" => "GitHub authorization was cancelled".to_string(),
            _ => "GitHub connection failed".to_string(),
        });
    }

    let github = fetch_github_account(&server_url, &token)?;
    let mut profile = fetch_profile(&server_url, &token)?;
    profile.github = Some(github);
    {
        let app_state = app.state::<AppState>();
        let mut auth = app_state
            .auth
            .lock()
            .map_err(|_| "auth lock is poisoned".to_string())?;
        auth.profile = Some(profile);
        save_auth_state(&app_state.auth_path, &auth).map_err(|error| error.to_string())?;
        emit_auth_event(app, auth.status(), None);
    }

    write_github_loopback_response(&mut stream, true).map_err(|error| error.to_string())
}

fn write_loopback_response(stream: &mut TcpStream, success: bool) -> Result<(), io::Error> {
    let (title, body) = if success {
        (
            "Patchwork login complete",
            "You can close this browser window and return to Patchwork.",
        )
    } else {
        (
            "Patchwork login failed",
            "The launcher could not complete authentication.",
        )
    };
    write_completion_response(stream, title, body)
}

fn write_github_loopback_response(stream: &mut TcpStream, success: bool) -> Result<(), io::Error> {
    let (title, body) = if success {
        (
            "GitHub account connected",
            "Patchwork has refreshed your profile. You can close this browser window.",
        )
    } else {
        (
            "GitHub connection failed",
            "Return to Patchwork for more information.",
        )
    };
    write_completion_response(stream, title, body)
}

fn write_completion_response(
    stream: &mut TcpStream,
    title: &str,
    body: &str,
) -> Result<(), io::Error> {
    let html = format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    :root {{
      --teal: #02a9a9;
      --gold: #fdb22c;
      --coral: #fd614e;
      --indigo: #6268c8;
      --ink: #f4f6fb;
      --muted: #a4aabc;
      --surface: #242833;
      --line: #4a5268;
      --bg: #171a23;
      --gradient: linear-gradient(90deg, var(--teal), var(--gold), var(--coral), var(--indigo));
    }}
    * {{ box-sizing: border-box; }}
    body {{
      min-height: 100vh;
      margin: 0;
      display: grid;
      place-items: center;
      padding: 18px;
      color: var(--ink);
      background: var(--bg);
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }}
    body::before {{
      position: fixed;
      inset: 0 0 auto;
      height: 5px;
      background: var(--gradient);
      content: "";
    }}
    main {{
      display: grid;
      width: min(480px, 100%);
      gap: 14px;
      padding: 24px;
      border: 1px dashed var(--line);
      border-radius: 8px;
      background: rgba(36, 40, 51, 0.96);
      box-shadow: 0 18px 44px rgba(0, 0, 0, 0.24);
    }}
    h1 {{ margin: 0; font-size: 38px; line-height: 1; }}
    p {{ margin: 0; color: var(--muted); line-height: 1.55; }}
    button {{
      min-height: 42px;
      border: 0;
      border-radius: 8px;
      color: #151821;
      background: var(--gradient);
      cursor: pointer;
      font: inherit;
      font-weight: 900;
    }}
  </style>
</head>
<body>
  <main>
    <h1>{title}</h1>
    <p>{body}</p>
    <button type="button" onclick="window.close()">Return to Patchwork</button>
  </main>
</body>
</html>"#
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream.write_all(response.as_bytes())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthTokenResponse {
    access_token: String,
    profile: AuthProfile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubConnectResponse {
    authorization_url: String,
}

pub(crate) fn authenticated_server_and_token(
    state: &State<AppState>,
) -> Result<(String, String), String> {
    let auth = state
        .auth
        .lock()
        .map_err(|_| "auth lock is poisoned".to_string())?;
    let token = auth
        .access_token
        .clone()
        .ok_or_else(|| "not authenticated".to_string())?;
    Ok((auth.server_url.clone(), token))
}

fn request_github_authorization(
    server_url: &str,
    token: &str,
    completion_url: &str,
) -> Result<String, String> {
    let connect_url = endpoint_url(server_url, "/github/connect")?;
    let response = ureq::post(&connect_url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_json(json!({ "completionUrl": completion_url }))
        .map_err(|error| error.to_string())?
        .into_json::<GithubConnectResponse>()
        .map_err(|error| error.to_string())?;
    Ok(response.authorization_url)
}

fn fetch_github_account(server_url: &str, token: &str) -> Result<GithubAccount, String> {
    let account_url = endpoint_url(server_url, "/github/account")?;
    ureq::get(&account_url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|error| error.to_string())?
        .into_json::<GithubAccount>()
        .map_err(|error| error.to_string())
}

fn exchange_code(
    server_url: &str,
    redirect_uri: &str,
    code_verifier: &str,
    code: &str,
) -> Result<OAuthTokenResponse, String> {
    let token_url = endpoint_url(server_url, "/oauth/token")?;
    let response = ureq::post(&token_url)
        .set("Content-Type", "application/json")
        .send_json(json!({
            "grant_type": "authorization_code",
            "client_id": APP_CLIENT_ID,
            "redirect_uri": redirect_uri,
            "code_verifier": code_verifier,
            "code": code,
        }))
        .map_err(|error| error.to_string())?;

    response.into_json().map_err(|error| error.to_string())
}

fn fetch_profile(server_url: &str, token: &str) -> Result<AuthProfile, String> {
    let profile_url = endpoint_url(server_url, "/api/profile")?;
    let response = ureq::get(&profile_url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|error| error.to_string())?;

    response.into_json().map_err(|error| error.to_string())
}

fn authorize_url(
    server_url: &str,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, String> {
    let mut url = Url::parse(server_url)
        .map_err(|error| error.to_string())?
        .join("/oauth/authorize")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", APP_CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    Ok(url.to_string())
}

pub(crate) fn endpoint_url(server_url: &str, path: &str) -> Result<String, String> {
    Ok(Url::parse(server_url)
        .map_err(|error| error.to_string())?
        .join(path)
        .map_err(|error| error.to_string())?
        .to_string())
}

fn normalize_server_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed).map_err(|error| error.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("auth server URL must use http or https".to_string());
    }
    Ok(trimmed.to_owned())
}

fn random_urlsafe(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    rand::rng().fill_bytes(&mut bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn emit_auth_event(app: &AppHandle, status: LauncherAuthStatus, error: Option<String>) {
    let _ = app.emit(PATCHWORK_AUTH_EVENT, PatchworkAuthEvent { status, error });
}
