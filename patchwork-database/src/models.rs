use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{DatabaseError, Result};
use crate::schema::{
    accounts, app_tokens, game_handshakes, game_launch_tickets, game_player_sessions,
    game_process_sessions, game_server_instances, game_transfer_tickets, github_accounts,
    github_oauth_states, mod_version_dependencies, mod_versions, modpack_version_dependencies,
    modpack_versions, modpacks, mods, oauth_authorization_codes, pending_registrations,
    registry_scan_entries, registry_scans, repositories, web_sessions,
};

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = accounts)]
#[diesel(primary_key(uuid))]
pub struct Account {
    pub uuid: String,
    pub nickname: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAccount {
    pub uuid: Uuid,
    pub nickname: String,
    pub email: String,
    pub password_hash: Option<String>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = pending_registrations)]
#[diesel(primary_key(verification_id_hash))]
pub struct PendingRegistration {
    pub verification_id_hash: String,
    pub code_hash: String,
    pub email: String,
    pub nickname: String,
    pub password_hash: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub attempts: i32,
}

#[derive(Debug, Clone)]
pub struct CreatePendingRegistration {
    pub verification_id_hash: String,
    pub code_hash: String,
    pub email: String,
    pub nickname: String,
    pub password_hash: String,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub enum PendingRegistrationVerification {
    Verified(Account),
    InvalidCode { attempts_remaining: i32 },
    ExpiredOrMissing,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = repositories)]
#[diesel(primary_key(id))]
pub struct Repository {
    pub id: String,
    pub provider: String,
    pub provider_repository_id: i64,
    pub owner: String,
    pub name: String,
    pub canonical_url: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = mods)]
#[diesel(primary_key(id))]
#[diesel(belongs_to(Account, foreign_key = publisher_uuid))]
#[diesel(belongs_to(Repository, foreign_key = repository_id))]
pub struct Mod {
    pub id: String,
    pub publisher_uuid: String,
    pub repository_id: String,
    pub source_base_path: String,
    pub latest_version_id: Option<String>,
    pub downloads: i64,
    pub created_at: NaiveDateTime,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = mod_versions)]
#[diesel(primary_key(id))]
#[diesel(belongs_to(Mod, foreign_key = mod_id))]
pub struct ModVersion {
    pub id: String,
    pub mod_id: String,
    pub version: String,
    pub title: String,
    pub repository_path: String,
    pub source_commit: String,
    pub source_tree_oid: String,
    pub manifest_path: String,
    pub manifest_blob_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub readme_blob_oid: Option<String>,
    pub image_path: Option<String>,
    pub image_blob_oid: Option<String>,
    pub metadata_json: String,
    pub published_by: String,
    pub published_github_user_id: i64,
    pub published_at: NaiveDateTime,
    pub yanked_at: Option<NaiveDateTime>,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = mod_version_dependencies)]
#[diesel(primary_key(version_id, relation_kind, target_kind, target_id))]
#[diesel(belongs_to(ModVersion, foreign_key = version_id))]
pub struct ModVersionDependency {
    pub version_id: String,
    pub relation_kind: String,
    pub target_kind: String,
    pub target_id: String,
    pub position: i32,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations)]
#[diesel(table_name = registry_scans)]
#[diesel(primary_key(id))]
#[diesel(belongs_to(Account, foreign_key = publisher_uuid))]
pub struct RegistryScan {
    pub id: String,
    pub publisher_uuid: String,
    pub github_user_id: i64,
    pub github_repository_id: i64,
    pub repository_owner: String,
    pub repository_name: String,
    pub repository_url: String,
    pub base_path: String,
    pub requested_ref: String,
    pub resolved_commit: String,
    pub root_tree_oid: String,
    pub warnings_json: String,
    pub errors_json: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub published_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations)]
#[diesel(table_name = registry_scan_entries)]
#[diesel(primary_key(id))]
#[diesel(belongs_to(RegistryScan, foreign_key = scan_id))]
pub struct RegistryScanEntry {
    pub id: String,
    pub scan_id: String,
    pub project_kind: String,
    pub project_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub repository_path: String,
    pub source_tree_oid: String,
    pub manifest_path: String,
    pub manifest_blob_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub readme_blob_oid: Option<String>,
    pub image_path: Option<String>,
    pub image_blob_oid: Option<String>,
    pub status: String,
    pub metadata_json: String,
    pub dependencies_json: String,
    pub warnings_json: String,
    pub errors_json: String,
}

#[derive(Debug, Clone)]
pub struct CreateRegistryScan {
    pub id: Uuid,
    pub publisher_uuid: Uuid,
    pub github_user_id: i64,
    pub github_repository_id: i64,
    pub repository_owner: String,
    pub repository_name: String,
    pub repository_url: String,
    pub base_path: String,
    pub requested_ref: String,
    pub resolved_commit: String,
    pub root_tree_oid: String,
    pub warnings_json: String,
    pub errors_json: String,
    pub expires_at: NaiveDateTime,
    pub entries: Vec<CreateRegistryScanEntry>,
}

#[derive(Debug, Clone)]
pub struct CreateRegistryScanEntry {
    pub id: Uuid,
    pub project_kind: String,
    pub project_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub repository_path: String,
    pub source_tree_oid: String,
    pub manifest_path: String,
    pub manifest_blob_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub readme_blob_oid: Option<String>,
    pub image_path: Option<String>,
    pub image_blob_oid: Option<String>,
    pub status: String,
    pub metadata_json: String,
    pub dependencies_json: String,
    pub warnings_json: String,
    pub errors_json: String,
}

#[derive(Debug, Clone)]
pub struct RegistryScanWithEntries {
    pub scan: RegistryScan,
    pub entries: Vec<RegistryScanEntry>,
}

#[derive(Debug, Clone)]
pub struct RegistryModState {
    pub mod_record: Mod,
    pub repository: Repository,
    pub versions: Vec<ModVersion>,
}

#[derive(Debug, Clone)]
pub struct RegistryModpackState {
    pub modpack_record: Modpack,
    pub repository: Repository,
    pub versions: Vec<ModpackVersion>,
}

#[derive(Debug, Clone, Queryable)]
pub struct PublishedMod {
    pub id: String,
    pub title: String,
    pub latest_version: String,
    pub downloads: i64,
    pub publisher_uuid: String,
    pub repository_url: String,
    pub repository_path: String,
    pub source_commit: String,
    pub source_tree_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub image_path: Option<String>,
}

#[derive(Debug, Clone, Queryable)]
pub struct PublishedModpack {
    pub id: String,
    pub title: String,
    pub description: String,
    pub latest_version: String,
    pub downloads: i64,
    pub publisher_uuid: String,
    pub repository_url: String,
    pub repository_path: String,
    pub source_commit: String,
    pub source_tree_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub image_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PublishedRegistryVersion {
    pub project_kind: String,
    pub project_id: String,
    pub version: String,
    pub version_id: String,
}

#[derive(Debug, Clone)]
pub struct RegistryPublishResult {
    pub scan_id: String,
    pub published: Vec<PublishedRegistryVersion>,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = modpacks)]
#[diesel(primary_key(id))]
#[diesel(belongs_to(Account, foreign_key = publisher_uuid))]
pub struct Modpack {
    pub id: String,
    pub publisher_uuid: String,
    pub repository_id: String,
    pub source_base_path: String,
    pub latest_version_id: Option<String>,
    pub downloads: i64,
    pub created_at: NaiveDateTime,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = modpack_versions)]
#[diesel(primary_key(id))]
#[diesel(belongs_to(Modpack, foreign_key = modpack_id))]
pub struct ModpackVersion {
    pub id: String,
    pub modpack_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub repository_path: String,
    pub source_commit: String,
    pub source_tree_oid: String,
    pub manifest_path: String,
    pub manifest_blob_oid: String,
    pub manifest_sha256: String,
    pub readme_path: Option<String>,
    pub readme_blob_oid: Option<String>,
    pub image_path: Option<String>,
    pub image_blob_oid: Option<String>,
    pub metadata_json: String,
    pub published_by: String,
    pub published_github_user_id: i64,
    pub published_at: NaiveDateTime,
    pub yanked_at: Option<NaiveDateTime>,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = modpack_version_dependencies)]
#[diesel(primary_key(version_id, relation_kind, target_kind, target_id))]
#[diesel(belongs_to(ModpackVersion, foreign_key = version_id))]
pub struct ModpackVersionDependency {
    pub version_id: String,
    pub relation_kind: String,
    pub target_kind: String,
    pub target_id: String,
    pub position: i32,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = web_sessions)]
#[diesel(primary_key(token_hash))]
#[diesel(belongs_to(Account, foreign_key = account_uuid))]
pub struct WebSession {
    pub token_hash: String,
    pub account_uuid: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = oauth_authorization_codes)]
#[diesel(primary_key(code_hash))]
#[diesel(belongs_to(Account, foreign_key = account_uuid))]
pub struct OAuthAuthorizationCode {
    pub code_hash: String,
    pub account_uuid: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub used_at: Option<NaiveDateTime>,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = app_tokens)]
#[diesel(primary_key(token_hash))]
#[diesel(belongs_to(Account, foreign_key = account_uuid))]
pub struct AppToken {
    pub token_hash: String,
    pub account_uuid: String,
    pub label: Option<String>,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub last_used_at: Option<NaiveDateTime>,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = github_accounts)]
#[diesel(primary_key(account_uuid))]
#[diesel(belongs_to(Account, foreign_key = account_uuid))]
pub struct GithubAccount {
    pub account_uuid: String,
    pub github_user_id: i64,
    pub github_login: String,
    pub github_avatar_url: String,
    pub linked_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(
    Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, Deserialize,
)]
#[diesel(table_name = github_oauth_states)]
#[diesel(primary_key(state_hash))]
#[diesel(belongs_to(Account, foreign_key = account_uuid))]
pub struct GithubOAuthState {
    pub state_hash: String,
    pub account_uuid: String,
    pub completion_url: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub used_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = game_server_instances)]
#[diesel(primary_key(server_id))]
pub struct GameServerInstance {
    pub server_id: String,
    pub secret_hash: String,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub last_seen_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub closed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = game_launch_tickets)]
#[diesel(primary_key(ticket_hash))]
pub struct GameLaunchTicket {
    pub ticket_hash: String,
    pub account_uuid: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub consumed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = game_process_sessions)]
pub struct GameProcessSession {
    pub id: String,
    pub token_hash: String,
    pub account_uuid: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub last_used_at: Option<NaiveDateTime>,
    pub revoked_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = game_player_sessions)]
pub struct GamePlayerSession {
    pub id: String,
    pub account_uuid: String,
    pub process_session_id: String,
    pub current_server_id: String,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub disconnected_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = game_transfer_tickets)]
pub struct GameTransferTicket {
    pub id: String,
    pub ticket_hash: String,
    pub player_session_id: String,
    pub process_session_id: String,
    pub account_uuid: String,
    pub source_server_id: String,
    pub target_server_id: Option<String>,
    pub target_handshake_id: Option<String>,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub reserved_at: Option<NaiveDateTime>,
    pub reservation_expires_at: Option<NaiveDateTime>,
    pub consumed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, Deserialize)]
#[diesel(table_name = game_handshakes)]
pub struct GameHandshake {
    pub id: String,
    pub protocol_version: i32,
    pub server_id: String,
    pub server_public_key: String,
    pub server_nonce: String,
    pub process_session_id: Option<String>,
    pub account_uuid: Option<String>,
    pub client_public_key: Option<String>,
    pub client_nonce: Option<String>,
    pub handshake_hash: Option<String>,
    pub kind: Option<String>,
    pub transfer_id: Option<String>,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
    pub authorized_at: Option<NaiveDateTime>,
    pub consumed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct AuthorizedGameProcess {
    pub session: GameProcessSession,
    pub account: Account,
}

#[derive(Debug, Clone)]
pub struct GameAdmission {
    pub player_session: GamePlayerSession,
    pub account: Account,
    pub admission: String,
    pub source_server_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreatedGameTransfer {
    pub transfer: GameTransferTicket,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pagination {
    pub limit: i64,
    pub offset: i64,
}

impl Pagination {
    pub fn new(limit: i64, offset: i64) -> Result<Self> {
        if !(1..=100).contains(&limit) {
            return Err(DatabaseError::Validation {
                field: "limit",
                message: "must be between 1 and 100".to_owned(),
            });
        }
        if offset < 0 {
            return Err(DatabaseError::Validation {
                field: "offset",
                message: "must be non-negative".to_owned(),
            });
        }
        Ok(Self { limit, offset })
    }
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            limit: 25,
            offset: 0,
        }
    }
}

#[derive(Debug, Insertable)]
#[diesel(table_name = accounts)]
pub(crate) struct NewAccountRow<'a> {
    pub uuid: &'a str,
    pub nickname: &'a str,
    pub email: &'a str,
    pub password_hash: Option<&'a str>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = pending_registrations)]
pub(crate) struct NewPendingRegistrationRow<'a> {
    pub verification_id_hash: &'a str,
    pub code_hash: &'a str,
    pub email: &'a str,
    pub nickname: &'a str,
    pub password_hash: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = repositories)]
pub(crate) struct NewRepositoryRow<'a> {
    pub id: &'a str,
    pub provider: &'a str,
    pub provider_repository_id: i64,
    pub owner: &'a str,
    pub name: &'a str,
    pub canonical_url: &'a str,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = mods)]
pub(crate) struct NewModRow<'a> {
    pub id: &'a str,
    pub publisher_uuid: &'a str,
    pub repository_id: &'a str,
    pub source_base_path: &'a str,
    pub latest_version_id: Option<&'a str>,
    pub downloads: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = mod_versions)]
pub(crate) struct NewModVersionRow<'a> {
    pub id: &'a str,
    pub mod_id: &'a str,
    pub version: &'a str,
    pub title: &'a str,
    pub repository_path: &'a str,
    pub source_commit: &'a str,
    pub source_tree_oid: &'a str,
    pub manifest_path: &'a str,
    pub manifest_blob_oid: &'a str,
    pub manifest_sha256: &'a str,
    pub readme_path: Option<&'a str>,
    pub readme_blob_oid: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub image_blob_oid: Option<&'a str>,
    pub metadata_json: &'a str,
    pub published_by: &'a str,
    pub published_github_user_id: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = mod_version_dependencies)]
pub(crate) struct NewModVersionDependencyRow<'a> {
    pub version_id: &'a str,
    pub relation_kind: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
    pub position: i32,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = registry_scans)]
pub(crate) struct NewRegistryScanRow<'a> {
    pub id: &'a str,
    pub publisher_uuid: &'a str,
    pub github_user_id: i64,
    pub github_repository_id: i64,
    pub repository_owner: &'a str,
    pub repository_name: &'a str,
    pub repository_url: &'a str,
    pub base_path: &'a str,
    pub requested_ref: &'a str,
    pub resolved_commit: &'a str,
    pub root_tree_oid: &'a str,
    pub warnings_json: &'a str,
    pub errors_json: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = registry_scan_entries)]
pub(crate) struct NewRegistryScanEntryRow<'a> {
    pub id: &'a str,
    pub scan_id: &'a str,
    pub project_kind: &'a str,
    pub project_id: &'a str,
    pub version: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub repository_path: &'a str,
    pub source_tree_oid: &'a str,
    pub manifest_path: &'a str,
    pub manifest_blob_oid: &'a str,
    pub manifest_sha256: &'a str,
    pub readme_path: Option<&'a str>,
    pub readme_blob_oid: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub image_blob_oid: Option<&'a str>,
    pub status: &'a str,
    pub metadata_json: &'a str,
    pub dependencies_json: &'a str,
    pub warnings_json: &'a str,
    pub errors_json: &'a str,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = modpacks)]
pub(crate) struct NewModpackRow<'a> {
    pub id: &'a str,
    pub publisher_uuid: &'a str,
    pub repository_id: &'a str,
    pub source_base_path: &'a str,
    pub latest_version_id: Option<&'a str>,
    pub downloads: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = modpack_versions)]
pub(crate) struct NewModpackVersionRow<'a> {
    pub id: &'a str,
    pub modpack_id: &'a str,
    pub version: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub repository_path: &'a str,
    pub source_commit: &'a str,
    pub source_tree_oid: &'a str,
    pub manifest_path: &'a str,
    pub manifest_blob_oid: &'a str,
    pub manifest_sha256: &'a str,
    pub readme_path: Option<&'a str>,
    pub readme_blob_oid: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub image_blob_oid: Option<&'a str>,
    pub metadata_json: &'a str,
    pub published_by: &'a str,
    pub published_github_user_id: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = modpack_version_dependencies)]
pub(crate) struct NewModpackVersionDependencyRow<'a> {
    pub version_id: &'a str,
    pub relation_kind: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
    pub position: i32,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = web_sessions)]
pub(crate) struct NewWebSessionRow<'a> {
    pub token_hash: &'a str,
    pub account_uuid: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = oauth_authorization_codes)]
pub(crate) struct NewOAuthAuthorizationCodeRow<'a> {
    pub code_hash: &'a str,
    pub account_uuid: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = app_tokens)]
pub(crate) struct NewAppTokenRow<'a> {
    pub token_hash: &'a str,
    pub account_uuid: &'a str,
    pub label: Option<&'a str>,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = github_accounts)]
pub(crate) struct NewGithubAccountRow<'a> {
    pub account_uuid: &'a str,
    pub github_user_id: i64,
    pub github_login: &'a str,
    pub github_avatar_url: &'a str,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = github_oauth_states)]
pub(crate) struct NewGithubOAuthStateRow<'a> {
    pub state_hash: &'a str,
    pub account_uuid: &'a str,
    pub completion_url: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = game_server_instances)]
pub(crate) struct NewGameServerInstanceRow<'a> {
    pub server_id: &'a str,
    pub secret_hash: &'a str,
    pub status: &'a str,
    pub last_seen_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = game_launch_tickets)]
pub(crate) struct NewGameLaunchTicketRow<'a> {
    pub ticket_hash: &'a str,
    pub account_uuid: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = game_process_sessions)]
pub(crate) struct NewGameProcessSessionRow<'a> {
    pub id: &'a str,
    pub token_hash: &'a str,
    pub account_uuid: &'a str,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = game_player_sessions)]
pub(crate) struct NewGamePlayerSessionRow<'a> {
    pub id: &'a str,
    pub account_uuid: &'a str,
    pub process_session_id: &'a str,
    pub current_server_id: &'a str,
    pub status: &'a str,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = game_transfer_tickets)]
pub(crate) struct NewGameTransferTicketRow<'a> {
    pub id: &'a str,
    pub ticket_hash: &'a str,
    pub player_session_id: &'a str,
    pub process_session_id: &'a str,
    pub account_uuid: &'a str,
    pub source_server_id: &'a str,
    pub target_server_id: Option<&'a str>,
    pub target_handshake_id: Option<&'a str>,
    pub status: &'a str,
    pub expires_at: NaiveDateTime,
    pub reservation_expires_at: Option<NaiveDateTime>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = game_handshakes)]
pub(crate) struct NewGameHandshakeRow<'a> {
    pub id: &'a str,
    pub protocol_version: i32,
    pub server_id: &'a str,
    pub server_public_key: &'a str,
    pub server_nonce: &'a str,
    pub status: &'a str,
    pub expires_at: NaiveDateTime,
}
