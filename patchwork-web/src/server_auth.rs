use std::fmt::Write as _;

use actix_web::cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use actix_web::http::header::LOCATION;
use actix_web::{HttpRequest, HttpResponse, Result, error, web};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use base64::{Engine, engine::general_purpose};
use chrono::{Duration, Utc};
use patchwork_database::{
    Account, CreatePendingRegistration, Database, Pagination, PendingRegistrationVerification,
    PublishedMod, PublishedModpack,
};
use patchwork_web::auth_types::{
    AccountDto, LoginRequest, OAuthTokenRequest, OAuthTokenResponse, ProfileDto,
    PublishedProjectDto, RegisterRequest, RegistrationChallengeDto, UpdateNicknameRequest,
    VerifyRegistrationRequest,
};
use rand::{Rng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const WEB_SESSION_COOKIE: &str = "patchwork_session";
const APP_CLIENT_ID: &str = "patchwork-app";
const WEB_SESSION_DAYS: i64 = 14;
const APP_TOKEN_DAYS: i64 = 90;
const AUTH_CODE_MINUTES: i64 = 10;
const EMAIL_CODE_MINUTES: i64 = 10;

#[derive(Clone)]
pub(crate) struct AuthState {
    database: Database,
    email: crate::email::EmailSender,
    secure_cookies: bool,
}

impl AuthState {
    pub(crate) fn new(
        database: Database,
        email: crate::email::EmailSender,
        secure_cookies: bool,
    ) -> Self {
        Self {
            database,
            email,
            secure_cookies,
        }
    }

    pub(crate) fn database(&self) -> &Database {
        &self.database
    }
}

pub(crate) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/api/auth/register", web::post().to(register))
        .route(
            "/api/auth/register/verify",
            web::post().to(verify_registration),
        )
        .route("/api/auth/login", web::post().to(login))
        .route("/api/auth/me", web::get().to(auth_me))
        .route("/api/auth/logout", web::post().to(logout))
        .route("/api/account/nickname", web::post().to(update_nickname))
        .route("/api/profile", web::get().to(profile))
        .route("/oauth/authorize", web::get().to(oauth_authorize))
        .route("/oauth/login", web::post().to(oauth_login))
        .route("/oauth/register", web::post().to(oauth_register))
        .route(
            "/oauth/register/verify",
            web::post().to(oauth_verify_registration),
        )
        .route("/oauth/consent", web::post().to(oauth_consent))
        .route("/oauth/token", web::post().to(oauth_token));
}

async fn register(
    state: web::Data<AuthState>,
    body: web::Json<RegisterRequest>,
) -> Result<HttpResponse> {
    let challenge = start_registration(&state, &body).await?;

    Ok(HttpResponse::Accepted().json(challenge))
}

async fn verify_registration(
    state: web::Data<AuthState>,
    body: web::Json<VerifyRegistrationRequest>,
) -> Result<HttpResponse> {
    let account = complete_registration(&state.database, &body.verification_id, &body.code)
        .map_err(RegistrationCompletionError::into_actix)?;
    let session_token = create_web_session(&state, &account)?;
    let profile = profile_for_account(&state.database, &account)?;

    Ok(HttpResponse::Ok()
        .cookie(session_cookie(&session_token, state.secure_cookies))
        .json(profile))
}

async fn login(state: web::Data<AuthState>, body: web::Json<LoginRequest>) -> Result<HttpResponse> {
    let account =
        authenticate_password_login(&state.database, &body.identifier, &body.password_sha256)?;
    let session_token = create_web_session(&state, &account)?;
    let profile = profile_for_account(&state.database, &account)?;

    Ok(HttpResponse::Ok()
        .cookie(session_cookie(&session_token, state.secure_cookies))
        .json(profile))
}

async fn auth_me(state: web::Data<AuthState>, request: HttpRequest) -> Result<HttpResponse> {
    let account = authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    Ok(HttpResponse::Ok().json(profile_for_account(&state.database, &account)?))
}

async fn profile(state: web::Data<AuthState>, request: HttpRequest) -> Result<HttpResponse> {
    auth_me(state, request).await
}

async fn update_nickname(
    state: web::Data<AuthState>,
    request: HttpRequest,
    body: web::Json<UpdateNicknameRequest>,
) -> Result<HttpResponse> {
    let account = authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    let account_uuid = parse_account_uuid(&account)?;
    let account = state
        .database
        .update_account_nickname(account_uuid, &body.nickname)
        .map_err(to_bad_request_or_internal)?;
    Ok(HttpResponse::Ok().json(profile_for_account(&state.database, &account)?))
}

async fn logout(state: web::Data<AuthState>, request: HttpRequest) -> Result<HttpResponse> {
    if let Some(cookie) = request.cookie(WEB_SESSION_COOKIE) {
        let _ = state
            .database
            .delete_web_session(&sha256_hex(cookie.value()));
    }

    if let Some(token) = bearer_token(&request) {
        let _ = state.database.delete_app_token(&sha256_hex(token));
    }

    Ok(HttpResponse::Ok()
        .cookie(clear_session_cookie(state.secure_cookies))
        .finish())
}

#[derive(Clone, Debug, Deserialize)]
struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: Option<String>,
}

async fn oauth_authorize(
    state: web::Data<AuthState>,
    request: HttpRequest,
    query: web::Query<AuthorizeQuery>,
) -> Result<HttpResponse> {
    validate_authorize_request(&query)?;

    if let Some(account) = authenticated_account(&state.database, &request)? {
        return Ok(HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(authorize_consent_html(&query, &account)));
    }

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(authorize_login_html(&query)))
}

#[derive(Debug, Deserialize)]
struct OAuthLoginForm {
    identifier: String,
    password_sha256: String,
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthRegisterForm {
    email: String,
    nickname: String,
    password_sha256: String,
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthVerifyRegistrationForm {
    verification_id: String,
    email: String,
    code: String,
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthConsentForm {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    state: Option<String>,
}

impl From<&OAuthLoginForm> for AuthorizeQuery {
    fn from(form: &OAuthLoginForm) -> Self {
        Self {
            response_type: form.response_type.clone(),
            client_id: form.client_id.clone(),
            redirect_uri: form.redirect_uri.clone(),
            code_challenge: form.code_challenge.clone(),
            code_challenge_method: form.code_challenge_method.clone(),
            state: form.state.clone(),
        }
    }
}

impl From<&OAuthRegisterForm> for AuthorizeQuery {
    fn from(form: &OAuthRegisterForm) -> Self {
        Self {
            response_type: form.response_type.clone(),
            client_id: form.client_id.clone(),
            redirect_uri: form.redirect_uri.clone(),
            code_challenge: form.code_challenge.clone(),
            code_challenge_method: form.code_challenge_method.clone(),
            state: form.state.clone(),
        }
    }
}

impl From<&OAuthVerifyRegistrationForm> for AuthorizeQuery {
    fn from(form: &OAuthVerifyRegistrationForm) -> Self {
        Self {
            response_type: form.response_type.clone(),
            client_id: form.client_id.clone(),
            redirect_uri: form.redirect_uri.clone(),
            code_challenge: form.code_challenge.clone(),
            code_challenge_method: form.code_challenge_method.clone(),
            state: form.state.clone(),
        }
    }
}

impl From<&OAuthConsentForm> for AuthorizeQuery {
    fn from(form: &OAuthConsentForm) -> Self {
        Self {
            response_type: form.response_type.clone(),
            client_id: form.client_id.clone(),
            redirect_uri: form.redirect_uri.clone(),
            code_challenge: form.code_challenge.clone(),
            code_challenge_method: form.code_challenge_method.clone(),
            state: form.state.clone(),
        }
    }
}

async fn oauth_login(
    state: web::Data<AuthState>,
    form: web::Form<OAuthLoginForm>,
) -> Result<HttpResponse> {
    let form = form.into_inner();
    let query = AuthorizeQuery::from(&form);
    validate_authorize_request(&query)?;
    let account =
        authenticate_password_login(&state.database, &form.identifier, &form.password_sha256)?;
    let session_token = create_web_session(&state, &account)?;

    Ok(HttpResponse::Ok()
        .cookie(session_cookie(&session_token, state.secure_cookies))
        .content_type("text/html; charset=utf-8")
        .body(authorize_consent_html(&query, &account)))
}

async fn oauth_register(
    state: web::Data<AuthState>,
    form: web::Form<OAuthRegisterForm>,
) -> Result<HttpResponse> {
    let form = form.into_inner();
    let query = AuthorizeQuery::from(&form);
    validate_authorize_request(&query)?;
    let challenge = start_registration(
        &state,
        &RegisterRequest {
            email: form.email,
            nickname: form.nickname,
            password_sha256: form.password_sha256,
        },
    )
    .await?;

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(authorize_verification_html(
            &query,
            &challenge.verification_id,
            &challenge.email,
            None,
        )))
}

async fn oauth_verify_registration(
    state: web::Data<AuthState>,
    form: web::Form<OAuthVerifyRegistrationForm>,
) -> Result<HttpResponse> {
    let form = form.into_inner();
    let query = AuthorizeQuery::from(&form);
    validate_authorize_request(&query)?;

    let account = match complete_registration(&state.database, &form.verification_id, &form.code) {
        Ok(account) => account,
        Err(RegistrationCompletionError::Invalid(message)) => {
            return Ok(HttpResponse::BadRequest()
                .content_type("text/html; charset=utf-8")
                .body(authorize_verification_html(
                    &query,
                    &form.verification_id,
                    &form.email,
                    Some(&message),
                )));
        }
        Err(error) => return Err(error.into_actix()),
    };
    let session_token = create_web_session(&state, &account)?;

    Ok(HttpResponse::Ok()
        .cookie(session_cookie(&session_token, state.secure_cookies))
        .content_type("text/html; charset=utf-8")
        .body(authorize_consent_html(&query, &account)))
}

async fn oauth_consent(
    state: web::Data<AuthState>,
    request: HttpRequest,
    form: web::Form<OAuthConsentForm>,
) -> Result<HttpResponse> {
    let query = AuthorizeQuery::from(&form.into_inner());
    validate_authorize_request(&query)?;
    let account = authenticated_account(&state.database, &request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))?;
    redirect_with_code(&state, &account, &query)
}

async fn oauth_token(
    state: web::Data<AuthState>,
    body: web::Json<OAuthTokenRequest>,
) -> Result<HttpResponse> {
    if body.grant_type != "authorization_code" {
        return Err(error::ErrorBadRequest("unsupported grant_type"));
    }
    if body.client_id != APP_CLIENT_ID {
        return Err(error::ErrorBadRequest("unknown client_id"));
    }
    validate_loopback_redirect_uri(&body.redirect_uri)?;
    validate_pkce_verifier(&body.code_verifier)?;

    let now = Utc::now().naive_utc();
    let Some(code) = state
        .database
        .consume_oauth_authorization_code(&sha256_hex(&body.code), now)
        .map_err(to_bad_request_or_internal)?
    else {
        return Err(error::ErrorBadRequest(
            "invalid or expired authorization code",
        ));
    };

    if code.client_id != body.client_id || code.redirect_uri != body.redirect_uri {
        return Err(error::ErrorBadRequest(
            "authorization code does not match this client",
        ));
    }
    if code.code_challenge != pkce_s256(&body.code_verifier) {
        return Err(error::ErrorBadRequest("invalid PKCE verifier"));
    }

    let account_uuid = Uuid::parse_str(&code.account_uuid)
        .map_err(|_| error::ErrorInternalServerError("stored account UUID is invalid"))?;
    let account = state
        .database
        .get_account(account_uuid)
        .map_err(to_bad_request_or_internal)?
        .ok_or_else(|| error::ErrorInternalServerError("account disappeared"))?;
    let access_token = random_urlsafe(32);
    let expires_at = now + Duration::days(APP_TOKEN_DAYS);
    state
        .database
        .create_app_token(
            &sha256_hex(&access_token),
            &account.uuid,
            Some("Patchwork desktop app"),
            expires_at,
        )
        .map_err(to_bad_request_or_internal)?;

    Ok(HttpResponse::Ok().json(OAuthTokenResponse {
        access_token,
        token_type: "Bearer".to_owned(),
        expires_in: Duration::days(APP_TOKEN_DAYS).num_seconds(),
        profile: profile_for_account(&state.database, &account)?,
    }))
}

async fn start_registration(
    state: &AuthState,
    request: &RegisterRequest,
) -> Result<RegistrationChallengeDto> {
    if state
        .database
        .get_account_by_email(&request.email)
        .map_err(to_bad_request_or_internal)?
        .is_some()
    {
        return Err(error::ErrorBadRequest("email is already registered"));
    }
    if state
        .database
        .get_account_by_nickname(&request.nickname)
        .map_err(to_bad_request_or_internal)?
        .is_some()
    {
        return Err(error::ErrorBadRequest("username is already taken"));
    }

    let password_hash = hash_client_password(&request.password_sha256)?;
    let verification_id = random_urlsafe(32);
    let code = format!("{:06}", rand::rng().random_range(0..1_000_000_u32));
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::minutes(EMAIL_CODE_MINUTES);
    let verification_id_hash = sha256_hex(&verification_id);
    let pending = state
        .database
        .create_pending_registration(
            CreatePendingRegistration {
                verification_id_hash: verification_id_hash.clone(),
                code_hash: registration_code_hash(&verification_id, &code),
                email: request.email.clone(),
                nickname: request.nickname.clone(),
                password_hash,
                expires_at,
            },
            now,
        )
        .map_err(to_bad_request_or_internal)?;

    if let Err(send_error) = state
        .email
        .send_verification_code(&pending.email, &pending.nickname, &code, EMAIL_CODE_MINUTES)
        .await
    {
        let _ = state
            .database
            .delete_pending_registration(&verification_id_hash);
        eprintln!("failed to send registration email: {send_error}");
        return Err(error::ErrorInternalServerError(
            "failed to send verification email",
        ));
    }

    Ok(RegistrationChallengeDto {
        verification_id,
        email: pending.email,
        expires_in: Duration::minutes(EMAIL_CODE_MINUTES).num_seconds(),
    })
}

#[derive(Debug)]
enum RegistrationCompletionError {
    Invalid(String),
    Internal(String),
}

impl RegistrationCompletionError {
    fn into_actix(self) -> actix_web::Error {
        match self {
            Self::Invalid(message) => error::ErrorBadRequest(message),
            Self::Internal(message) => error::ErrorInternalServerError(message),
        }
    }
}

fn complete_registration(
    database: &Database,
    verification_id: &str,
    code: &str,
) -> std::result::Result<Account, RegistrationCompletionError> {
    validate_verification_id(verification_id)?;
    validate_verification_code(code)?;

    let result = database
        .verify_pending_registration(
            &sha256_hex(verification_id),
            &registration_code_hash(verification_id, code),
            Uuid::new_v4(),
            Utc::now().naive_utc(),
        )
        .map_err(|error| match error {
            patchwork_database::DatabaseError::Validation { .. }
            | patchwork_database::DatabaseError::Conflict { .. } => {
                RegistrationCompletionError::Invalid(error.to_string())
            }
            other => RegistrationCompletionError::Internal(other.to_string()),
        })?;

    match result {
        PendingRegistrationVerification::Verified(account) => Ok(account),
        PendingRegistrationVerification::InvalidCode { attempts_remaining } => {
            let message = if attempts_remaining == 0 {
                "invalid verification code; request a new code".to_owned()
            } else {
                format!("invalid verification code; {attempts_remaining} attempts remaining")
            };
            Err(RegistrationCompletionError::Invalid(message))
        }
        PendingRegistrationVerification::ExpiredOrMissing => {
            Err(RegistrationCompletionError::Invalid(
                "verification request is invalid or expired".to_owned(),
            ))
        }
    }
}

fn validate_verification_id(value: &str) -> std::result::Result<(), RegistrationCompletionError> {
    let is_valid = value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if is_valid {
        Ok(())
    } else {
        Err(RegistrationCompletionError::Invalid(
            "invalid verification request".to_owned(),
        ))
    }
}

fn validate_verification_code(value: &str) -> std::result::Result<(), RegistrationCompletionError> {
    if value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err(RegistrationCompletionError::Invalid(
            "verification code must contain exactly six digits".to_owned(),
        ))
    }
}

fn registration_code_hash(verification_id: &str, code: &str) -> String {
    sha256_hex(&format!("{verification_id}:{code}"))
}

fn authenticate_password_login(
    database: &Database,
    identifier: &str,
    password_sha256: &str,
) -> Result<Account> {
    let Some(account) = database
        .get_account_by_login_identifier(identifier)
        .map_err(to_bad_request_or_internal)?
    else {
        return Err(error::ErrorUnauthorized(
            "invalid username/email or password",
        ));
    };

    verify_client_password(password_sha256, account.password_hash.as_deref())?;
    Ok(account)
}

fn hash_client_password(password_sha256: &str) -> Result<String> {
    validate_client_password_sha256(password_sha256)?;
    let mut salt_bytes = [0_u8; 16];
    rand::rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|_| error::ErrorInternalServerError("failed to generate password salt"))?;
    Argon2::default()
        .hash_password(password_sha256.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| error::ErrorInternalServerError("failed to hash password"))
}

fn verify_client_password(password_sha256: &str, stored_hash: Option<&str>) -> Result<()> {
    validate_client_password_sha256(password_sha256)?;
    let Some(stored_hash) = stored_hash else {
        return Err(error::ErrorUnauthorized(
            "invalid username/email or password",
        ));
    };
    let parsed_hash = PasswordHash::new(stored_hash)
        .map_err(|_| error::ErrorUnauthorized("invalid username/email or password"))?;
    Argon2::default()
        .verify_password(password_sha256.as_bytes(), &parsed_hash)
        .map_err(|_| error::ErrorUnauthorized("invalid username/email or password"))
}

fn validate_client_password_sha256(value: &str) -> Result<()> {
    let is_valid = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_valid {
        Ok(())
    } else {
        Err(error::ErrorBadRequest(
            "passwordSha256 must be a SHA-256 hex digest",
        ))
    }
}

fn create_web_session(state: &AuthState, account: &Account) -> Result<String> {
    let token = random_urlsafe(32);
    let expires_at = Utc::now().naive_utc() + Duration::days(WEB_SESSION_DAYS);
    state
        .database
        .create_web_session(&sha256_hex(&token), &account.uuid, expires_at)
        .map_err(to_bad_request_or_internal)?;
    Ok(token)
}

fn redirect_with_code(
    state: &AuthState,
    account: &Account,
    query: &AuthorizeQuery,
) -> Result<HttpResponse> {
    let code = random_urlsafe(32);
    let expires_at = Utc::now().naive_utc() + Duration::minutes(AUTH_CODE_MINUTES);
    state
        .database
        .create_oauth_authorization_code(
            &sha256_hex(&code),
            &account.uuid,
            &query.client_id,
            &query.redirect_uri,
            &query.code_challenge,
            expires_at,
        )
        .map_err(to_bad_request_or_internal)?;

    let mut redirect_uri = Url::parse(&query.redirect_uri)
        .map_err(|error| error::ErrorBadRequest(error.to_string()))?;
    redirect_uri.query_pairs_mut().append_pair("code", &code);
    if let Some(state) = query.state.as_deref() {
        redirect_uri.query_pairs_mut().append_pair("state", state);
    }

    Ok(HttpResponse::Found()
        .insert_header((LOCATION, redirect_uri.to_string()))
        .finish())
}

pub(crate) fn authenticated_account(
    database: &Database,
    request: &HttpRequest,
) -> Result<Option<Account>> {
    let now = Utc::now().naive_utc();

    if let Some(token) = bearer_token(request) {
        return database
            .account_for_app_token(&sha256_hex(token), now)
            .map_err(to_bad_request_or_internal);
    }

    let Some(cookie) = request.cookie(WEB_SESSION_COOKIE) else {
        return Ok(None);
    };
    database
        .account_for_web_session(&sha256_hex(cookie.value()), now)
        .map_err(to_bad_request_or_internal)
}

fn profile_for_account(database: &Database, account: &Account) -> Result<ProfileDto> {
    let uuid = parse_account_uuid(account)?;
    let pagination = Pagination::new(100, 0).map_err(to_bad_request_or_internal)?;
    let mods = database
        .list_mods_by_publisher(uuid, pagination)
        .map_err(to_bad_request_or_internal)?
        .into_iter()
        .map(mod_to_dto)
        .collect();
    let modpacks = database
        .list_modpacks_by_publisher(uuid, pagination)
        .map_err(to_bad_request_or_internal)?
        .into_iter()
        .map(modpack_to_dto)
        .collect();
    let github = database
        .get_github_account(uuid)
        .map_err(to_bad_request_or_internal)?
        .map(crate::server_github::github_account_dto);

    Ok(ProfileDto {
        account: AccountDto {
            uuid: account.uuid.clone(),
            nickname: account.nickname.clone(),
            email: account.email.clone(),
        },
        github,
        mods,
        modpacks,
    })
}

fn parse_account_uuid(account: &Account) -> Result<Uuid> {
    Uuid::parse_str(&account.uuid)
        .map_err(|_| error::ErrorInternalServerError("stored account UUID is invalid"))
}

fn mod_to_dto(project: PublishedMod) -> PublishedProjectDto {
    PublishedProjectDto {
        id: project.id,
        title: project.title,
        kind: "Mod".to_owned(),
        downloads: project.downloads,
        latest_version: Some(project.latest_version),
        repository_url: Some(project.repository_url),
        repository_path: Some(project.repository_path),
        can_rescan: true,
    }
}

fn modpack_to_dto(project: PublishedModpack) -> PublishedProjectDto {
    PublishedProjectDto {
        id: project.id,
        title: project.title,
        kind: "Modpack".to_owned(),
        downloads: project.downloads,
        latest_version: Some(project.latest_version),
        repository_url: Some(project.repository_url),
        repository_path: Some(project.repository_path),
        can_rescan: true,
    }
}

fn validate_authorize_request(query: &AuthorizeQuery) -> Result<()> {
    if query.response_type != "code" {
        return Err(error::ErrorBadRequest("response_type must be code"));
    }
    if query.client_id != APP_CLIENT_ID {
        return Err(error::ErrorBadRequest("unknown client_id"));
    }
    if query.code_challenge_method != "S256" {
        return Err(error::ErrorBadRequest("code_challenge_method must be S256"));
    }
    if query.code_challenge.len() < 43 || query.code_challenge.len() > 128 {
        return Err(error::ErrorBadRequest("invalid code_challenge length"));
    }
    validate_loopback_redirect_uri(&query.redirect_uri)
}

fn validate_loopback_redirect_uri(value: &str) -> Result<()> {
    let parsed = Url::parse(value).map_err(|error| error::ErrorBadRequest(error.to_string()))?;
    if parsed.scheme() != "http" {
        return Err(error::ErrorBadRequest("redirect_uri must use http"));
    }
    match parsed.host_str() {
        Some("127.0.0.1") | Some("localhost") => {}
        _ => {
            return Err(error::ErrorBadRequest(
                "redirect_uri must be a loopback URL",
            ));
        }
    }
    if parsed.port().is_none() {
        return Err(error::ErrorBadRequest(
            "redirect_uri must include a loopback port",
        ));
    }
    Ok(())
}

fn validate_pkce_verifier(value: &str) -> Result<()> {
    if !(43..=128).contains(&value.len()) {
        return Err(error::ErrorBadRequest("invalid PKCE verifier length"));
    }
    let is_valid = value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'));
    if is_valid {
        Ok(())
    } else {
        Err(error::ErrorBadRequest("invalid PKCE verifier characters"))
    }
}

fn bearer_token(request: &HttpRequest) -> Option<&str> {
    request
        .headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

fn session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build(WEB_SESSION_COOKIE, token.to_owned())
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(CookieDuration::days(WEB_SESSION_DAYS))
        .finish()
}

fn clear_session_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build(WEB_SESSION_COOKIE, "")
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(CookieDuration::seconds(0))
        .finish()
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

fn pkce_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn authorize_login_html(query: &AuthorizeQuery) -> String {
    let oauth_fields = hidden_oauth_fields(query);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Sign in to Patchwork</title>
  <link rel="stylesheet" href="/styles.css">
</head>
<body>
  <main class="oauth-page">
    <section class="oauth-auth-stack">
      <form class="auth-card" method="post" action="/oauth/register" data-password-form data-register-form data-auth-panel="register">
        <img src="/logo.png" alt="Patchwork">
        <h1>Create account</h1>
        <p>Use a stable UUID-backed publisher account.</p>
        <label>
          <span>Email</span>
          <input name="email" type="email" autocomplete="email" required autofocus>
        </label>
        <label>
          <span>Username</span>
          <input name="nickname" autocomplete="username" required maxlength="16">
        </label>
        <label>
          <span>Password</span>
          <input name="password" type="password" autocomplete="new-password" required minlength="12">
        </label>
        <label>
          <span>Confirm password</span>
          <input name="password_confirmation" type="password" autocomplete="new-password" required minlength="12">
        </label>
        <div class="password-requirements" data-password-requirements hidden>
          <p class="password-requirement" data-requirement="length">At least 12 characters</p>
          <p class="password-requirement" data-requirement="lowercase">One lowercase letter</p>
          <p class="password-requirement" data-requirement="uppercase">One uppercase letter</p>
          <p class="password-requirement" data-requirement="number">One number</p>
          <p class="password-requirement" data-requirement="symbol">One symbol</p>
          <p class="password-requirement" data-requirement="match">Passwords match</p>
        </div>
        <input type="hidden" name="password_sha256">
        {oauth_fields}
        <button type="submit" class="sign-in-button">Create account</button>
        <button type="button" class="auth-switch-button" data-auth-switch data-target-auth="login">
          Already have an account? Sign in
        </button>
      </form>

      <form class="auth-card" method="post" action="/oauth/login" data-password-form data-auth-panel="login" hidden>
        <img src="/logo.png" alt="Patchwork">
        <h1>Sign in</h1>
        <p>Continue to Patchwork Desktop with an existing account.</p>
        <label>
          <span>Email or username</span>
          <input name="identifier" autocomplete="username" required>
        </label>
        <label>
          <span>Password</span>
          <input name="password" type="password" autocomplete="current-password" required>
        </label>
        <input type="hidden" name="password_sha256">
        {oauth_fields}
        <button type="submit" class="sign-in-button">Continue</button>
        <button type="button" class="auth-switch-button" data-auth-switch data-target-auth="register">
          Need an account? Create one
        </button>
      </form>
    </section>
  </main>
  {password_hash_script}
</body>
</html>"#,
        oauth_fields = oauth_fields,
        password_hash_script = password_hash_script(),
    )
}

fn authorize_consent_html(query: &AuthorizeQuery, account: &Account) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Authorize Patchwork Desktop</title>
  <link rel="stylesheet" href="/styles.css">
</head>
<body>
  <main class="oauth-page">
    <form class="auth-card oauth-consent-card" method="post" action="/oauth/consent">
      <img src="/logo.png" alt="Patchwork">
      <p class="catalog-kicker">Patchwork login complete</p>
      <h1>Authorize desktop app</h1>
      <p>You are signed in as <strong>{nickname}</strong>. Continue only if you started this login from Patchwork Desktop.</p>
      <div class="oauth-account-box">
        <span>{email}</span>
        <code>{uuid}</code>
      </div>
      {fields}
      <button type="submit" class="sign-in-button">Authorize access</button>
    </form>
  </main>
</body>
</html>"#,
        nickname = escape_text(&account.nickname),
        email = escape_text(&account.email),
        uuid = escape_text(&account.uuid),
        fields = hidden_oauth_fields(query),
    )
}

fn authorize_verification_html(
    query: &AuthorizeQuery,
    verification_id: &str,
    email: &str,
    message: Option<&str>,
) -> String {
    let error_message = message
        .map(|message| format!(r#"<p class="auth-error">{}</p>"#, escape_text(message)))
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Verify your Patchwork email</title>
  <link rel="stylesheet" href="/styles.css">
</head>
<body>
  <main class="oauth-page">
    <form class="auth-card oauth-verification-card" method="post" action="/oauth/register/verify">
      <img src="/logo.png" alt="Patchwork">
      <p class="catalog-kicker">Email verification</p>
      <h1>Check your inbox</h1>
      <p>We sent a six-digit code to <strong>{email}</strong>. It expires in {expires_in} minutes.</p>
      <label>
        <span>Verification code</span>
        <input class="verification-code-input" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{{6}}" minlength="6" maxlength="6" required autofocus>
      </label>
      {error_message}
      <input type="hidden" name="verification_id" value="{verification_id}">
      <input type="hidden" name="email" value="{email_attr}">
      {fields}
      <button type="submit" class="sign-in-button">Verify and continue</button>
    </form>
  </main>
</body>
</html>"#,
        email = escape_text(email),
        email_attr = escape_attr(email),
        verification_id = escape_attr(verification_id),
        expires_in = EMAIL_CODE_MINUTES,
        fields = hidden_oauth_fields(query),
    )
}

fn password_hash_script() -> &'static str {
    r#"<script>
const hex = bytes => Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("");
async function sha256(value) {
  const data = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return hex(new Uint8Array(digest));
}
const passwordRequirements = (password, confirmation) => ({
  length: password.length >= 12,
  lowercase: /[a-z]/.test(password),
  uppercase: /[A-Z]/.test(password),
  number: /[0-9]/.test(password),
  symbol: /[!-\/:-@\[-`{-~]/.test(password),
  match: password.length > 0 && password === confirmation,
});
function renderPasswordRequirements(form) {
  if (!form.matches("[data-register-form]")) return true;
  const password = form.querySelector("input[name='password']");
  const confirmation = form.querySelector("input[name='password_confirmation']");
  const requirements = form.querySelector("[data-password-requirements]");
  const checks = passwordRequirements(password.value, confirmation.value);
  const valid = Object.values(checks).every(Boolean);
  const started = password.value.length > 0 || confirmation.value.length > 0;
  requirements.hidden = !started || valid;
  form.querySelectorAll("[data-requirement]").forEach(row => {
    row.hidden = checks[row.dataset.requirement] === true;
  });
  password.classList.toggle("invalid", started && !valid);
  confirmation.classList.toggle("invalid", started && !valid);
  return valid;
}
document.querySelectorAll("[data-auth-switch]").forEach(button => {
  button.addEventListener("click", () => {
    const target = button.dataset.targetAuth;
    document.querySelectorAll("[data-auth-panel]").forEach(panel => {
      panel.hidden = panel.dataset.authPanel !== target;
      if (!panel.hidden) renderPasswordRequirements(panel);
    });
  });
});
document.querySelectorAll("[data-password-form]").forEach(form => {
  renderPasswordRequirements(form);
  form.querySelectorAll("input[name='password'], input[name='password_confirmation']").forEach(input => {
    input.addEventListener("input", () => renderPasswordRequirements(form));
  });
  form.addEventListener("submit", async event => {
    event.preventDefault();
    if (!renderPasswordRequirements(form)) return;
    const password = form.querySelector("input[name='password']");
    const target = form.querySelector("input[name='password_sha256']");
    target.value = await sha256(password.value);
    password.value = "";
    password.disabled = true;
    const confirmation = form.querySelector("input[name='password_confirmation']");
    if (confirmation) {
      confirmation.value = "";
      confirmation.disabled = true;
    }
    form.submit();
  });
});
</script>"#
}

fn hidden_oauth_fields(query: &AuthorizeQuery) -> String {
    let mut fields = String::new();
    hidden_field(&mut fields, "response_type", &query.response_type);
    hidden_field(&mut fields, "client_id", &query.client_id);
    hidden_field(&mut fields, "redirect_uri", &query.redirect_uri);
    hidden_field(&mut fields, "code_challenge", &query.code_challenge);
    hidden_field(
        &mut fields,
        "code_challenge_method",
        &query.code_challenge_method,
    );
    if let Some(state) = query.state.as_deref() {
        hidden_field(&mut fields, "state", state);
    }
    fields
}

fn hidden_field(output: &mut String, name: &str, value: &str) {
    let _ = write!(
        output,
        r#"<input type="hidden" name="{}" value="{}">"#,
        escape_attr(name),
        escape_attr(value)
    );
}

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn to_bad_request_or_internal(error: patchwork_database::DatabaseError) -> actix_web::Error {
    match error {
        patchwork_database::DatabaseError::Validation { .. }
        | patchwork_database::DatabaseError::Conflict { .. } => {
            error::ErrorBadRequest(error.to_string())
        }
        patchwork_database::DatabaseError::NotFound { .. } => {
            error::ErrorNotFound(error.to_string())
        }
        other => error::ErrorInternalServerError(other.to_string()),
    }
}
