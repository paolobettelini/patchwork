use std::fmt;

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use patchwork_database::{
    Account, AuthorizeGameHandshake, CreateGameTransfer, Database, DatabaseError,
    GameServerInstance, RedeemGameHandshake,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::GameAuthConfig;

const PROTOCOL_VERSION: u16 = 1;
const LAUNCH_TICKET_SECONDS: i64 = 60;
const HANDSHAKE_SECONDS: i64 = 20;
const TRANSFER_SECONDS: i64 = 60;
const TRANSFER_RESERVATION_SECONDS: i64 = 20;
const SERVER_LEASE_SECONDS: i64 = 10 * 60;
const CLEANUP_INTERVAL_SECONDS: u64 = 60;
const CLEANUP_RETENTION_HOURS: i64 = 24;
const TRANSCRIPT_DOMAIN: &[u8] = b"patchwork-game-handshake-v1";

#[derive(Clone)]
pub(crate) struct GameAuthState {
    database: Database,
    process_session_hours: i64,
}

impl GameAuthState {
    pub(crate) fn new(database: Database, config: &GameAuthConfig) -> Self {
        Self {
            database,
            process_session_hours: config.process_session_hours,
        }
    }
}

pub(crate) fn spawn_cleanup(state: GameAuthState) {
    actix_web::rt::spawn(async move {
        loop {
            actix_web::rt::time::sleep(std::time::Duration::from_secs(CLEANUP_INTERVAL_SECONDS))
                .await;
            let now = Utc::now().naive_utc();
            let database = state.database.clone();
            match web::block(move || {
                database.cleanup_game_auth(now, now - Duration::hours(CLEANUP_RETENTION_HOURS))
            })
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("game authentication cleanup failed: {error}"),
                Err(error) => eprintln!("game authentication cleanup worker failed: {error}"),
            }
        }
    });
}

pub(crate) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/server/instances", web::post().to(create_server_instance))
        .route(
            "/server/instances/{server_id}/heartbeat",
            web::post().to(heartbeat_server_instance),
        )
        .route(
            "/server/instances/{server_id}",
            web::delete().to(close_server_instance),
        )
        .route("/game/launch-ticket", web::post().to(create_launch_ticket))
        .route(
            "/game/process-sessions",
            web::post().to(create_process_session),
        )
        .route(
            "/game/handshakes/authorize",
            web::post().to(authorize_handshake),
        )
        .route("/server/handshakes", web::post().to(register_handshake))
        .route(
            "/server/handshakes/{handshake_id}/redeem",
            web::post().to(redeem_handshake),
        )
        .route(
            "/server/player-sessions/{player_session_id}/transfers",
            web::post().to(create_transfer),
        )
        .route(
            "/server/transfers/{transfer_id}",
            web::get().to(get_transfer),
        );
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ServerInstanceResponse {
    server_id: String,
    server_secret: String,
    expires_in: i64,
}

async fn create_server_instance(state: web::Data<GameAuthState>) -> ApiResult {
    let server_id = Uuid::new_v4();
    let server_secret = random_token();
    let now = Utc::now().naive_utc();
    state
        .database
        .create_game_server_instance(
            server_id,
            &sha256_hex(&server_secret),
            now,
            now + Duration::seconds(SERVER_LEASE_SECONDS),
        )
        .map_err(GameApiError::from_database)?;

    Ok(HttpResponse::Created().json(ServerInstanceResponse {
        server_id: server_id.hyphenated().to_string(),
        server_secret,
        expires_in: SERVER_LEASE_SECONDS,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct HeartbeatResponse {
    alive: bool,
    server_id: String,
    expires_in: i64,
}

async fn heartbeat_server_instance(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    server_id: web::Path<String>,
) -> ApiResult {
    let server_id = parse_uuid("server_id", &server_id)?
        .hyphenated()
        .to_string();
    let secret_hash = server_secret_hash(&request)?;
    let now = Utc::now().naive_utc();
    state
        .database
        .heartbeat_game_server_instance(
            &server_id,
            &secret_hash,
            now,
            now + Duration::seconds(SERVER_LEASE_SECONDS),
        )
        .map_err(GameApiError::from_database)?
        .ok_or_else(|| {
            GameApiError::unauthorized(
                "invalid_server_instance",
                "server secret is invalid or the server lease has expired",
            )
        })?;

    Ok(HttpResponse::Ok().json(HeartbeatResponse {
        alive: true,
        server_id,
        expires_in: SERVER_LEASE_SECONDS,
    }))
}

async fn close_server_instance(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    server_id: web::Path<String>,
) -> ApiResult {
    let server_id = parse_uuid("server_id", &server_id)?
        .hyphenated()
        .to_string();
    let secret_hash = server_secret_hash(&request)?;
    if !state
        .database
        .close_game_server_instance(&server_id, &secret_hash, Utc::now().naive_utc())
        .map_err(GameApiError::from_database)?
    {
        return Err(GameApiError::unauthorized(
            "invalid_server_instance",
            "server secret is invalid or the server instance is not active",
        ));
    }
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct LaunchTicketResponse {
    launch_ticket: String,
    expires_in: i64,
}

async fn create_launch_ticket(state: web::Data<GameAuthState>, request: HttpRequest) -> ApiResult {
    let account = authenticated_app_account(&state.database, &request)?;
    let ticket = random_token();
    let now = Utc::now().naive_utc();
    state
        .database
        .create_game_launch_ticket(
            &sha256_hex(&ticket),
            &account.uuid,
            now + Duration::seconds(LAUNCH_TICKET_SECONDS),
        )
        .map_err(GameApiError::from_database)?;
    Ok(HttpResponse::Ok().json(LaunchTicketResponse {
        launch_ticket: ticket,
        expires_in: LAUNCH_TICKET_SECONDS,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct ProcessSessionRequest {
    launch_ticket: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ProcessSessionResponse {
    process_token: String,
    process_session_id: String,
    expires_in: i64,
    uuid: String,
    nickname: String,
}

async fn create_process_session(
    state: web::Data<GameAuthState>,
    body: web::Json<ProcessSessionRequest>,
) -> ApiResult {
    decode_32("launch_ticket", &body.launch_ticket)?;
    let process_token = random_token();
    let process_session_id = Uuid::new_v4();
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::hours(state.process_session_hours);
    let process = state
        .database
        .consume_game_launch_ticket(
            &sha256_hex(&body.launch_ticket),
            process_session_id,
            &sha256_hex(&process_token),
            now,
            expires_at,
        )
        .map_err(GameApiError::from_database)?
        .ok_or_else(|| {
            GameApiError::unauthorized(
                "invalid_launch_ticket",
                "launch ticket is invalid, expired, or already consumed",
            )
        })?;

    Ok(HttpResponse::Ok().json(ProcessSessionResponse {
        process_token,
        process_session_id: process.session.id,
        expires_in: (process.session.expires_at - now).num_seconds(),
        uuid: process.account.uuid,
        nickname: process.account.nickname,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct RegisterHandshakeRequest {
    handshake_id: String,
    protocol_version: u16,
    server_public_key: String,
    server_nonce: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct RegisterHandshakeResponse {
    registered: bool,
    server_id: String,
    expires_in: i64,
}

async fn register_handshake(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    body: web::Json<RegisterHandshakeRequest>,
) -> ApiResult {
    let server = authenticated_game_server(&state.database, &request)?;
    require_protocol_version(body.protocol_version)?;
    let handshake_id = parse_uuid("handshake_id", &body.handshake_id)?;
    reject_zero_key(
        "server_public_key",
        decode_32("server_public_key", &body.server_public_key)?,
    )?;
    decode_32("server_nonce", &body.server_nonce)?;
    let now = Utc::now().naive_utc();
    state
        .database
        .register_game_handshake(
            &server.server_id,
            handshake_id,
            i32::from(body.protocol_version),
            &body.server_public_key,
            &body.server_nonce,
            now,
            now + Duration::seconds(HANDSHAKE_SECONDS),
        )
        .map_err(GameApiError::from_database)?;

    Ok(HttpResponse::Ok().json(RegisterHandshakeResponse {
        registered: true,
        server_id: server.server_id,
        expires_in: HANDSHAKE_SECONDS,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct AuthorizeHandshakeRequest {
    protocol_version: u16,
    handshake_id: String,
    server_id: String,
    server_public_key: String,
    client_public_key: String,
    server_nonce: String,
    client_nonce: String,
    handshake_hash: String,
    transfer_ticket: Option<String>,
}

#[derive(Serialize)]
struct AuthorizedResponse {
    authorized: bool,
}

async fn authorize_handshake(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    body: web::Json<AuthorizeHandshakeRequest>,
) -> ApiResult {
    let process = authenticated_game_process(&state.database, &request)?;
    require_protocol_version(body.protocol_version)?;
    let handshake_id = parse_uuid("handshake_id", &body.handshake_id)?;
    let server_id = parse_uuid("server_id", &body.server_id)?
        .hyphenated()
        .to_string();
    if body.server_id != server_id {
        return Err(GameApiError::bad_request(
            "invalid_server_id",
            "server_id must use canonical lowercase hyphenated UUID text",
        ));
    }
    let server_public_key = decode_32("server_public_key", &body.server_public_key)?;
    let client_public_key = decode_32("client_public_key", &body.client_public_key)?;
    let server_nonce = decode_32("server_nonce", &body.server_nonce)?;
    let client_nonce = decode_32("client_nonce", &body.client_nonce)?;
    let supplied_hash = decode_32("handshake_hash", &body.handshake_hash)?;
    reject_zero_key("server_public_key", server_public_key)?;
    reject_zero_key("client_public_key", client_public_key)?;

    let expected_hash = transcript_hash(
        body.protocol_version,
        handshake_id,
        &server_id,
        &server_public_key,
        &client_public_key,
        &server_nonce,
        &client_nonce,
    )?;
    if !constant_time_eq(&expected_hash, &supplied_hash) {
        return Err(GameApiError::bad_request(
            "invalid_handshake_hash",
            "handshake_hash does not match the canonical transcript",
        ));
    }

    let transfer_ticket_hash = body
        .transfer_ticket
        .as_deref()
        .map(|ticket| {
            decode_32("transfer_ticket", ticket)?;
            Ok::<_, GameApiError>(sha256_hex(ticket))
        })
        .transpose()?;
    let now = Utc::now().naive_utc();
    state
        .database
        .authorize_game_handshake(
            &process,
            &AuthorizeGameHandshake {
                handshake_id,
                protocol_version: i32::from(body.protocol_version),
                server_id,
                server_public_key: body.server_public_key.clone(),
                server_nonce: body.server_nonce.clone(),
                client_public_key: body.client_public_key.clone(),
                client_nonce: body.client_nonce.clone(),
                handshake_hash: body.handshake_hash.clone(),
                transfer_ticket_hash,
                reservation_expires_at: now + Duration::seconds(TRANSFER_RESERVATION_SECONDS),
            },
            now,
        )
        .map_err(GameApiError::from_database)?;

    Ok(HttpResponse::Ok().json(AuthorizedResponse { authorized: true }))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct RedeemHandshakeRequest {
    client_public_key: String,
    client_nonce: String,
    handshake_hash: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct RedeemHandshakeResponse {
    accepted: bool,
    admission: String,
    player_session_id: String,
    account: RedeemedAccountResponse,
    source_server_id: Option<String>,
}

#[derive(Serialize)]
struct RedeemedAccountResponse {
    uuid: String,
    nickname: String,
}

async fn redeem_handshake(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    handshake_id: web::Path<String>,
    body: web::Json<RedeemHandshakeRequest>,
) -> ApiResult {
    let server = authenticated_game_server(&state.database, &request)?;
    let handshake_id = parse_uuid("handshake_id", &handshake_id)?;
    reject_zero_key(
        "client_public_key",
        decode_32("client_public_key", &body.client_public_key)?,
    )?;
    decode_32("client_nonce", &body.client_nonce)?;
    decode_32("handshake_hash", &body.handshake_hash)?;
    let admission = state
        .database
        .redeem_game_handshake(
            &server.server_id,
            &RedeemGameHandshake {
                handshake_id,
                client_public_key: body.client_public_key.clone(),
                client_nonce: body.client_nonce.clone(),
                handshake_hash: body.handshake_hash.clone(),
                direct_player_session_id: Uuid::new_v4(),
            },
            Utc::now().naive_utc(),
        )
        .map_err(GameApiError::from_database)?;

    Ok(HttpResponse::Ok().json(RedeemHandshakeResponse {
        accepted: true,
        admission: admission.admission,
        player_session_id: admission.player_session.id,
        account: RedeemedAccountResponse {
            uuid: admission.account.uuid,
            nickname: admission.account.nickname,
        },
        source_server_id: admission.source_server_id,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct CreateTransferResponse {
    transfer_id: String,
    transfer_ticket: String,
    expires_in: i64,
}

async fn create_transfer(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    player_session_id: web::Path<String>,
) -> ApiResult {
    let server = authenticated_game_server(&state.database, &request)?;
    let player_session_id = parse_uuid("player_session_id", &player_session_id)?;
    let transfer_ticket = random_token();
    let now = Utc::now().naive_utc();
    let transfer_id = Uuid::new_v4();
    state
        .database
        .create_game_transfer(
            &server.server_id,
            &CreateGameTransfer {
                transfer_id,
                ticket_hash: sha256_hex(&transfer_ticket),
                player_session_id,
                expires_at: now + Duration::seconds(TRANSFER_SECONDS),
            },
            now,
        )
        .map_err(GameApiError::from_database)?;

    Ok(HttpResponse::Ok().json(CreateTransferResponse {
        transfer_id: transfer_id.hyphenated().to_string(),
        transfer_ticket,
        expires_in: TRANSFER_SECONDS,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct TransferStatusResponse {
    transfer_id: String,
    status: String,
    target_server_id: Option<String>,
    target_handshake_id: Option<String>,
}

async fn get_transfer(
    state: web::Data<GameAuthState>,
    request: HttpRequest,
    transfer_id: web::Path<String>,
) -> ApiResult {
    let server = authenticated_game_server(&state.database, &request)?;
    let transfer_id = parse_uuid("transfer_id", &transfer_id)?
        .hyphenated()
        .to_string();
    let transfer = state
        .database
        .game_transfer_for_source(&transfer_id, &server.server_id, Utc::now().naive_utc())
        .map_err(GameApiError::from_database)?
        .ok_or_else(|| {
            GameApiError::new(
                StatusCode::NOT_FOUND,
                "transfer_not_found",
                "transfer does not exist for the authenticated source server",
            )
        })?;

    Ok(HttpResponse::Ok().json(TransferStatusResponse {
        transfer_id: transfer.id,
        status: transfer.status.to_ascii_uppercase(),
        target_server_id: transfer.target_server_id,
        target_handshake_id: transfer.target_handshake_id,
    }))
}

fn authenticated_app_account(
    database: &Database,
    request: &HttpRequest,
) -> Result<Account, GameApiError> {
    let token = bearer_token(request).ok_or_else(|| {
        GameApiError::unauthorized(
            "missing_app_token",
            "a Patchwork app Bearer token is required",
        )
    })?;
    database
        .account_for_app_token(&sha256_hex(token), Utc::now().naive_utc())
        .map_err(GameApiError::from_database)?
        .ok_or_else(|| {
            GameApiError::unauthorized("invalid_app_token", "app token is invalid or expired")
        })
}

fn authenticated_game_process(
    database: &Database,
    request: &HttpRequest,
) -> Result<patchwork_database::AuthorizedGameProcess, GameApiError> {
    let token = bearer_token(request).ok_or_else(|| {
        GameApiError::unauthorized(
            "missing_process_token",
            "a process Bearer token is required",
        )
    })?;
    database
        .game_process_for_token_hash(&sha256_hex(token), Utc::now().naive_utc())
        .map_err(GameApiError::from_database)?
        .ok_or_else(|| {
            GameApiError::unauthorized(
                "invalid_process_token",
                "process token is invalid, expired, or revoked",
            )
        })
}

fn authenticated_game_server(
    database: &Database,
    request: &HttpRequest,
) -> Result<GameServerInstance, GameApiError> {
    let secret_hash = server_secret_hash(request)?;
    database
        .game_server_for_secret_hash(&secret_hash, Utc::now().naive_utc())
        .map_err(GameApiError::from_database)?
        .ok_or_else(|| {
            GameApiError::unauthorized(
                "invalid_server_secret",
                "server secret is invalid or the server lease has expired",
            )
        })
}

fn server_secret_hash(request: &HttpRequest) -> Result<String, GameApiError> {
    let secret = bearer_token(request).ok_or_else(|| {
        GameApiError::unauthorized(
            "missing_server_secret",
            "a server Bearer secret is required",
        )
    })?;
    decode_32("server_secret", secret)?;
    Ok(sha256_hex(secret))
}

fn transcript_hash(
    protocol_version: u16,
    handshake_id: Uuid,
    server_id: &str,
    server_public_key: &[u8; 32],
    client_public_key: &[u8; 32],
    server_nonce: &[u8; 32],
    client_nonce: &[u8; 32],
) -> Result<[u8; 32], GameApiError> {
    let server_id = server_id.as_bytes();
    let server_id_len = u16::try_from(server_id.len())
        .map_err(|_| GameApiError::bad_request("invalid_server_id", "server_id is too long"))?;
    let mut transcript = Vec::with_capacity(28 + 2 + 16 + 2 + server_id.len() + 128);
    transcript.extend_from_slice(TRANSCRIPT_DOMAIN);
    transcript.extend_from_slice(&protocol_version.to_be_bytes());
    transcript.extend_from_slice(handshake_id.as_bytes());
    transcript.extend_from_slice(&server_id_len.to_be_bytes());
    transcript.extend_from_slice(server_id);
    transcript.extend_from_slice(server_public_key);
    transcript.extend_from_slice(client_public_key);
    transcript.extend_from_slice(server_nonce);
    transcript.extend_from_slice(client_nonce);
    Ok(Sha256::digest(transcript).into())
}

fn require_protocol_version(version: u16) -> Result<(), GameApiError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(GameApiError::bad_request(
            "unsupported_protocol_version",
            format!("protocol_version must be {PROTOCOL_VERSION}"),
        ))
    }
}

fn reject_zero_key(field: &'static str, key: [u8; 32]) -> Result<(), GameApiError> {
    if key.iter().all(|byte| *byte == 0) {
        Err(GameApiError::bad_request(
            "invalid_public_key",
            format!("{field} must not be the all-zero X25519 key"),
        ))
    } else {
        Ok(())
    }
}

fn decode_32(field: &'static str, value: &str) -> Result<[u8; 32], GameApiError> {
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| {
        GameApiError::bad_request(
            "invalid_encoding",
            format!("{field} must be unpadded Base64URL"),
        )
    })?;
    bytes.try_into().map_err(|_| {
        GameApiError::bad_request(
            "invalid_length",
            format!("{field} must decode to exactly 32 bytes"),
        )
    })
}

fn parse_uuid(field: &'static str, value: &str) -> Result<Uuid, GameApiError> {
    Uuid::parse_str(value)
        .map_err(|_| GameApiError::bad_request("invalid_uuid", format!("{field} must be a UUID")))
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

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

type ApiResult = Result<HttpResponse, GameApiError>;

#[derive(Debug)]
struct GameApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl GameApiError {
    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    fn unauthorized(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn from_database(error: DatabaseError) -> Self {
        match error {
            DatabaseError::Validation { message, .. } => {
                Self::bad_request("invalid_request", message)
            }
            DatabaseError::Conflict { key, .. } => Self::new(StatusCode::CONFLICT, "conflict", key),
            DatabaseError::NotFound { id, .. } => Self::new(StatusCode::NOT_FOUND, "not_found", id),
            DatabaseError::GameAuth { code, message } => {
                let status = if code.ends_with("_not_found") {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::CONFLICT
                };
                Self::new(status, code, message)
            }
            other => {
                eprintln!("game authentication database error: {other}");
                Self::internal("game authentication service failed")
            }
        }
    }
}

impl fmt::Display for GameApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl ResponseError for GameApiError {
    fn status_code(&self) -> StatusCode {
        self.status
    }

    fn error_response(&self) -> HttpResponse {
        #[derive(Serialize)]
        struct ErrorResponse<'a> {
            error: &'a str,
            message: &'a str,
        }

        HttpResponse::build(self.status).json(ErrorResponse {
            error: self.code,
            message: &self.message,
        })
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{App, test as actix_test};
    use patchwork_database::CreateAccount;
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn transcript_hash_is_stable() {
        let hash = transcript_hash(
            1,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            "7e63a4e8-9c65-4ec6-9bca-25ca5e303065",
            &[1; 32],
            &[2; 32],
            &[3; 32],
            &[4; 32],
        )
        .unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.encode(hash),
            "b_F6PXBXYQhFE8YDbMhcgq5ZJvdwnswjwWZTgknSt4s"
        );
    }

    #[actix_web::test]
    async fn dynamic_server_instance_renews_and_closes_with_its_secret() {
        let directory = tempdir().unwrap();
        let database = Database::connect(
            directory
                .path()
                .join("server-route.sqlite")
                .to_string_lossy(),
        )
        .unwrap();
        let state = GameAuthState {
            database,
            process_session_hours: 8,
        };
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure),
        )
        .await;

        let create = actix_test::TestRequest::post()
            .uri("/server/instances")
            .to_request();
        let create_response = actix_test::call_service(&app, create).await;
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let instance: Value = actix_test::read_body_json(create_response).await;
        let server_id = instance["server_id"].as_str().unwrap();
        let server_secret = instance["server_secret"].as_str().unwrap();

        let heartbeat = actix_test::TestRequest::post()
            .uri(&format!("/server/instances/{server_id}/heartbeat"))
            .insert_header(("Authorization", format!("Bearer {server_secret}")))
            .to_request();
        assert_eq!(
            actix_test::call_service(&app, heartbeat).await.status(),
            StatusCode::OK
        );

        let close = actix_test::TestRequest::delete()
            .uri(&format!("/server/instances/{server_id}"))
            .insert_header(("Authorization", format!("Bearer {server_secret}")))
            .to_request();
        assert_eq!(
            actix_test::call_service(&app, close).await.status(),
            StatusCode::NO_CONTENT
        );

        let heartbeat_after_close = actix_test::TestRequest::post()
            .uri(&format!("/server/instances/{server_id}/heartbeat"))
            .insert_header(("Authorization", format!("Bearer {server_secret}")))
            .to_request();
        assert_eq!(
            actix_test::call_service(&app, heartbeat_after_close)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[actix_web::test]
    async fn launch_ticket_creates_exactly_one_process_session() {
        let directory = tempdir().unwrap();
        let database =
            Database::connect(directory.path().join("game-route.sqlite").to_string_lossy())
                .unwrap();
        let account = database
            .create_account(CreateAccount {
                uuid: Uuid::new_v4(),
                nickname: "RoutePlayer".to_owned(),
                email: "route@example.com".to_owned(),
                password_hash: None,
            })
            .unwrap();
        let app_token = "desktop-app-token";
        database
            .create_app_token(
                &sha256_hex(app_token),
                &account.uuid,
                Some("test"),
                Utc::now().naive_utc() + Duration::hours(1),
            )
            .unwrap();
        let state = GameAuthState {
            database,
            process_session_hours: 8,
        };
        let app = actix_test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure),
        )
        .await;

        let ticket_request = actix_test::TestRequest::post()
            .uri("/game/launch-ticket")
            .insert_header(("Authorization", format!("Bearer {app_token}")))
            .to_request();
        let ticket_response: Value =
            actix_test::call_and_read_body_json(&app, ticket_request).await;
        let ticket = ticket_response["launch_ticket"].as_str().unwrap();

        let process_request = actix_test::TestRequest::post()
            .uri("/game/process-sessions")
            .set_json(json!({ "launch_ticket": ticket }))
            .to_request();
        let process_response = actix_test::call_service(&app, process_request).await;
        assert_eq!(process_response.status(), StatusCode::OK);

        let replay_request = actix_test::TestRequest::post()
            .uri("/game/process-sessions")
            .set_json(json!({ "launch_ticket": ticket }))
            .to_request();
        let replay_response = actix_test::call_service(&app, replay_request).await;
        assert_eq!(replay_response.status(), StatusCode::UNAUTHORIZED);
    }
}
