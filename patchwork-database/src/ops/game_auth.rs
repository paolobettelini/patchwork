use chrono::NaiveDateTime;
use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{
    Account, AuthorizedGameProcess, CreatedGameTransfer, GameAdmission, GameHandshake,
    GameLaunchTicket, GamePlayerSession, GameProcessSession, GameServerInstance,
    GameTransferTicket, NewGameHandshakeRow, NewGameLaunchTicketRow, NewGamePlayerSessionRow,
    NewGameProcessSessionRow, NewGameServerInstanceRow, NewGameTransferTicketRow,
};
use crate::schema::{
    accounts, game_handshakes, game_launch_tickets, game_player_sessions, game_process_sessions,
    game_server_instances, game_transfer_tickets,
};
use crate::validation::normalize_sha256_hex;

#[derive(Debug, Clone)]
pub struct AuthorizeGameHandshake {
    pub handshake_id: Uuid,
    pub protocol_version: i32,
    pub server_id: String,
    pub server_public_key: String,
    pub server_nonce: String,
    pub client_public_key: String,
    pub client_nonce: String,
    pub handshake_hash: String,
    pub transfer_ticket_hash: Option<String>,
    pub reservation_expires_at: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub struct RedeemGameHandshake {
    pub handshake_id: Uuid,
    pub client_public_key: String,
    pub client_nonce: String,
    pub handshake_hash: String,
    pub direct_player_session_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct CreateGameTransfer {
    pub transfer_id: Uuid,
    pub ticket_hash: String,
    pub player_session_id: Uuid,
    pub expires_at: NaiveDateTime,
}

impl Database {
    pub fn create_game_server_instance(
        &self,
        server_id: Uuid,
        secret_hash: &str,
        now: NaiveDateTime,
        expires_at: NaiveDateTime,
    ) -> Result<GameServerInstance> {
        let server_id = server_id.hyphenated().to_string();
        let secret_hash = normalize_sha256_hex("secret_hash", secret_hash)?;
        let mut connection = self.connection()?;
        diesel::insert_into(game_server_instances::table)
            .values(NewGameServerInstanceRow {
                server_id: &server_id,
                secret_hash: &secret_hash,
                status: "active",
                last_seen_at: now,
                expires_at,
            })
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "game_server_instance", &server_id))?;

        game_server_instances::table
            .find(server_id)
            .select(GameServerInstance::as_select())
            .first(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn heartbeat_game_server_instance(
        &self,
        server_id: &str,
        secret_hash: &str,
        now: NaiveDateTime,
        expires_at: NaiveDateTime,
    ) -> Result<Option<GameServerInstance>> {
        let server_id = normalize_uuid("server_id", server_id)?;
        let secret_hash = normalize_sha256_hex("secret_hash", secret_hash)?;
        let mut connection = self.connection()?;
        let updated = diesel::update(
            game_server_instances::table
                .filter(game_server_instances::server_id.eq(&server_id))
                .filter(game_server_instances::secret_hash.eq(&secret_hash))
                .filter(game_server_instances::status.eq("active"))
                .filter(game_server_instances::expires_at.gt(now)),
        )
        .set((
            game_server_instances::last_seen_at.eq(now),
            game_server_instances::expires_at.eq(expires_at),
        ))
        .execute(&mut connection)?;
        if updated != 1 {
            return Ok(None);
        }

        Ok(Some(
            game_server_instances::table
                .find(server_id)
                .select(GameServerInstance::as_select())
                .first(&mut connection)?,
        ))
    }

    pub fn close_game_server_instance(
        &self,
        server_id: &str,
        secret_hash: &str,
        now: NaiveDateTime,
    ) -> Result<bool> {
        let server_id = normalize_uuid("server_id", server_id)?;
        let secret_hash = normalize_sha256_hex("secret_hash", secret_hash)?;
        let mut connection = self.connection()?;
        connection.transaction::<bool, DatabaseError, _>(|connection| {
            let closed = diesel::update(
                game_server_instances::table
                    .filter(game_server_instances::server_id.eq(&server_id))
                    .filter(game_server_instances::secret_hash.eq(&secret_hash))
                    .filter(game_server_instances::status.eq("active"))
                    .filter(game_server_instances::expires_at.gt(now)),
            )
            .set((
                game_server_instances::status.eq("closed"),
                game_server_instances::closed_at.eq(Some(now)),
            ))
            .execute(connection)?;
            if closed != 1 {
                return Ok(false);
            }

            disconnect_players_on_servers(connection, &[server_id.clone()], now)?;
            diesel::update(
                game_transfer_tickets::table
                    .filter(game_transfer_tickets::source_server_id.eq(&server_id))
                    .filter(game_transfer_tickets::status.eq_any(["created", "reserved"])),
            )
            .set(game_transfer_tickets::status.eq("expired"))
            .execute(connection)?;
            Ok(true)
        })
    }

    pub fn game_server_for_secret_hash(
        &self,
        secret_hash: &str,
        now: NaiveDateTime,
    ) -> Result<Option<GameServerInstance>> {
        let secret_hash = normalize_sha256_hex("secret_hash", secret_hash)?;
        let mut connection = self.connection()?;
        Ok(game_server_instances::table
            .filter(game_server_instances::secret_hash.eq(secret_hash))
            .filter(game_server_instances::status.eq("active"))
            .filter(game_server_instances::expires_at.gt(now))
            .select(GameServerInstance::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn create_game_launch_ticket(
        &self,
        ticket_hash: &str,
        account_uuid: &str,
        expires_at: NaiveDateTime,
    ) -> Result<()> {
        let ticket_hash = normalize_sha256_hex("ticket_hash", ticket_hash)?;
        let account_uuid = normalize_uuid("account_uuid", account_uuid)?;
        let mut connection = self.connection()?;
        diesel::insert_into(game_launch_tickets::table)
            .values(NewGameLaunchTicketRow {
                ticket_hash: &ticket_hash,
                account_uuid: &account_uuid,
                expires_at,
            })
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "game_launch_ticket", &ticket_hash))?;
        Ok(())
    }

    pub fn consume_game_launch_ticket(
        &self,
        ticket_hash: &str,
        process_session_id: Uuid,
        process_token_hash: &str,
        now: NaiveDateTime,
        process_expires_at: NaiveDateTime,
    ) -> Result<Option<AuthorizedGameProcess>> {
        let ticket_hash = normalize_sha256_hex("ticket_hash", ticket_hash)?;
        let process_token_hash = normalize_sha256_hex("process_token_hash", process_token_hash)?;
        let process_session_id = process_session_id.hyphenated().to_string();
        let mut connection = self.connection()?;

        connection.transaction::<Option<AuthorizedGameProcess>, DatabaseError, _>(|connection| {
            let Some(ticket) = game_launch_tickets::table
                .find(&ticket_hash)
                .select(GameLaunchTicket::as_select())
                .first(connection)
                .optional()?
            else {
                return Ok(None);
            };
            if ticket.consumed_at.is_some() || ticket.expires_at <= now {
                return Ok(None);
            }

            let consumed = diesel::update(
                game_launch_tickets::table
                    .filter(game_launch_tickets::ticket_hash.eq(&ticket_hash))
                    .filter(game_launch_tickets::consumed_at.is_null())
                    .filter(game_launch_tickets::expires_at.gt(now)),
            )
            .set(game_launch_tickets::consumed_at.eq(Some(now)))
            .execute(connection)?;
            if consumed != 1 {
                return Ok(None);
            }

            diesel::insert_into(game_process_sessions::table)
                .values(NewGameProcessSessionRow {
                    id: &process_session_id,
                    token_hash: &process_token_hash,
                    account_uuid: &ticket.account_uuid,
                    expires_at: process_expires_at,
                })
                .execute(connection)
                .map_err(|error| {
                    map_write_error(error, "game_process_session", &process_session_id)
                })?;

            let session = game_process_sessions::table
                .find(&process_session_id)
                .select(GameProcessSession::as_select())
                .first(connection)?;
            let account = accounts::table
                .find(&ticket.account_uuid)
                .select(Account::as_select())
                .first(connection)?;
            Ok(Some(AuthorizedGameProcess { session, account }))
        })
    }

    pub fn game_process_for_token_hash(
        &self,
        token_hash: &str,
        now: NaiveDateTime,
    ) -> Result<Option<AuthorizedGameProcess>> {
        let token_hash = normalize_sha256_hex("process_token_hash", token_hash)?;
        let mut connection = self.connection()?;
        let result = game_process_sessions::table
            .inner_join(accounts::table)
            .filter(game_process_sessions::token_hash.eq(&token_hash))
            .filter(game_process_sessions::expires_at.gt(now))
            .filter(game_process_sessions::revoked_at.is_null())
            .select((GameProcessSession::as_select(), Account::as_select()))
            .first::<(GameProcessSession, Account)>(&mut connection)
            .optional()?;

        if let Some((session, account)) = result {
            diesel::update(game_process_sessions::table.find(&session.id))
                .set(game_process_sessions::last_used_at.eq(Some(now)))
                .execute(&mut connection)?;
            Ok(Some(AuthorizedGameProcess { session, account }))
        } else {
            Ok(None)
        }
    }

    pub fn register_game_handshake(
        &self,
        server_id: &str,
        handshake_id: Uuid,
        protocol_version: i32,
        server_public_key: &str,
        server_nonce: &str,
        now: NaiveDateTime,
        expires_at: NaiveDateTime,
    ) -> Result<GameHandshake> {
        let server_id = normalize_uuid("server_id", server_id)?;
        let id = handshake_id.hyphenated().to_string();
        let mut connection = self.connection()?;
        connection.transaction::<GameHandshake, DatabaseError, _>(|connection| {
            require_active_server(connection, &server_id, now)?;
            diesel::insert_into(game_handshakes::table)
                .values(NewGameHandshakeRow {
                    id: &id,
                    protocol_version,
                    server_id: &server_id,
                    server_public_key,
                    server_nonce,
                    status: "waiting",
                    expires_at,
                })
                .execute(connection)
                .map_err(|error| map_write_error(error, "game_handshake", &id))?;
            game_handshakes::table
                .find(&id)
                .select(GameHandshake::as_select())
                .first(connection)
                .map_err(DatabaseError::from)
        })
    }

    pub fn authorize_game_handshake(
        &self,
        process: &AuthorizedGameProcess,
        request: &AuthorizeGameHandshake,
        now: NaiveDateTime,
    ) -> Result<GameHandshake> {
        let handshake_id = request.handshake_id.hyphenated().to_string();
        let server_id = normalize_uuid("server_id", &request.server_id)?;
        let transfer_ticket_hash = request
            .transfer_ticket_hash
            .as_deref()
            .map(|value| normalize_sha256_hex("transfer_ticket_hash", value))
            .transpose()?;
        let mut connection = self.connection()?;

        connection.transaction::<GameHandshake, DatabaseError, _>(|connection| {
            let handshake = game_handshakes::table
                .find(&handshake_id)
                .select(GameHandshake::as_select())
                .first(connection)
                .optional()?
                .ok_or_else(|| game_error("handshake_not_found", "handshake does not exist"))?;
            validate_waiting_handshake(&handshake, request, &server_id, now)?;
            require_active_server(connection, &server_id, now)?;

            let (kind, transfer_id) = if let Some(ticket_hash) = transfer_ticket_hash.as_deref() {
                let transfer = game_transfer_tickets::table
                    .filter(game_transfer_tickets::ticket_hash.eq(ticket_hash))
                    .select(GameTransferTicket::as_select())
                    .first(connection)
                    .optional()?
                    .ok_or_else(|| {
                        game_error("transfer_not_found", "transfer ticket does not exist")
                    })?;
                if transfer.status != "created" || transfer.expires_at <= now {
                    return Err(game_error(
                        "transfer_unavailable",
                        "transfer ticket is expired, reserved, or consumed",
                    ));
                }
                if transfer.account_uuid != process.account.uuid
                    || transfer.process_session_id != process.session.id
                {
                    return Err(game_error(
                        "transfer_mismatch",
                        "transfer ticket does not belong to this game process",
                    ));
                }
                if transfer.source_server_id == server_id {
                    return Err(game_error(
                        "same_transfer_target",
                        "the target server must differ from the source server",
                    ));
                }
                let player = game_player_sessions::table
                    .find(&transfer.player_session_id)
                    .select(GamePlayerSession::as_select())
                    .first(connection)?;
                if player.status != "active"
                    || player.current_server_id != transfer.source_server_id
                    || player.account_uuid != process.account.uuid
                    || player.process_session_id != process.session.id
                {
                    return Err(game_error(
                        "transfer_stale",
                        "player session is no longer active on the transfer source server",
                    ));
                }

                let reserved = diesel::update(
                    game_transfer_tickets::table
                        .filter(game_transfer_tickets::id.eq(&transfer.id))
                        .filter(game_transfer_tickets::status.eq("created"))
                        .filter(game_transfer_tickets::expires_at.gt(now)),
                )
                .set((
                    game_transfer_tickets::target_server_id.eq(Some(&server_id)),
                    game_transfer_tickets::target_handshake_id.eq(Some(&handshake_id)),
                    game_transfer_tickets::status.eq("reserved"),
                    game_transfer_tickets::reserved_at.eq(Some(now)),
                    game_transfer_tickets::reservation_expires_at
                        .eq(Some(request.reservation_expires_at)),
                ))
                .execute(connection)?;
                if reserved != 1 {
                    return Err(game_error(
                        "transfer_unavailable",
                        "transfer ticket was already reserved",
                    ));
                }
                ("transfer", Some(transfer.id))
            } else {
                ("direct", None)
            };

            let authorized = diesel::update(
                game_handshakes::table
                    .filter(game_handshakes::id.eq(&handshake_id))
                    .filter(game_handshakes::status.eq("waiting"))
                    .filter(game_handshakes::expires_at.gt(now)),
            )
            .set((
                game_handshakes::process_session_id.eq(Some(&process.session.id)),
                game_handshakes::account_uuid.eq(Some(&process.account.uuid)),
                game_handshakes::client_public_key.eq(Some(&request.client_public_key)),
                game_handshakes::client_nonce.eq(Some(&request.client_nonce)),
                game_handshakes::handshake_hash.eq(Some(&request.handshake_hash)),
                game_handshakes::kind.eq(Some(kind)),
                game_handshakes::transfer_id.eq(transfer_id.as_deref()),
                game_handshakes::status.eq("authorized"),
                game_handshakes::authorized_at.eq(Some(now)),
            ))
            .execute(connection)?;
            if authorized != 1 {
                return Err(game_error(
                    "handshake_unavailable",
                    "handshake was concurrently authorized or expired",
                ));
            }

            game_handshakes::table
                .find(&handshake_id)
                .select(GameHandshake::as_select())
                .first(connection)
                .map_err(DatabaseError::from)
        })
    }

    pub fn redeem_game_handshake(
        &self,
        authenticated_server_id: &str,
        request: &RedeemGameHandshake,
        now: NaiveDateTime,
    ) -> Result<GameAdmission> {
        let server_id = normalize_uuid("server_id", authenticated_server_id)?;
        let handshake_id = request.handshake_id.hyphenated().to_string();
        let direct_player_session_id = request.direct_player_session_id.hyphenated().to_string();
        let mut connection = self.connection()?;

        connection.transaction::<GameAdmission, DatabaseError, _>(|connection| {
            require_active_server(connection, &server_id, now)?;
            let handshake = game_handshakes::table
                .find(&handshake_id)
                .select(GameHandshake::as_select())
                .first(connection)
                .optional()?
                .ok_or_else(|| game_error("handshake_not_found", "handshake does not exist"))?;
            if handshake.server_id != server_id {
                return Err(game_error(
                    "server_mismatch",
                    "handshake belongs to a different server",
                ));
            }
            if handshake.status != "authorized" || handshake.expires_at <= now {
                return Err(game_error(
                    "handshake_unavailable",
                    "handshake is not authorized or has expired",
                ));
            }
            if handshake.client_public_key.as_deref() != Some(&request.client_public_key)
                || handshake.client_nonce.as_deref() != Some(&request.client_nonce)
                || handshake.handshake_hash.as_deref() != Some(&request.handshake_hash)
            {
                return Err(game_error(
                    "handshake_mismatch",
                    "client key exchange values do not match authorization",
                ));
            }

            let process_session_id = handshake.process_session_id.as_deref().ok_or_else(|| {
                game_error(
                    "handshake_invalid",
                    "authorized handshake has no process session",
                )
            })?;
            let account_uuid = handshake.account_uuid.as_deref().ok_or_else(|| {
                game_error("handshake_invalid", "authorized handshake has no account")
            })?;
            let process = game_process_sessions::table
                .find(process_session_id)
                .select(GameProcessSession::as_select())
                .first(connection)?;
            if process.account_uuid != account_uuid
                || process.expires_at <= now
                || process.revoked_at.is_some()
            {
                return Err(game_error(
                    "process_session_expired",
                    "the game process session is no longer active",
                ));
            }

            let consumed = diesel::update(
                game_handshakes::table
                    .filter(game_handshakes::id.eq(&handshake_id))
                    .filter(game_handshakes::status.eq("authorized"))
                    .filter(game_handshakes::expires_at.gt(now)),
            )
            .set((
                game_handshakes::status.eq("consumed"),
                game_handshakes::consumed_at.eq(Some(now)),
            ))
            .execute(connection)?;
            if consumed != 1 {
                return Err(game_error(
                    "handshake_unavailable",
                    "handshake was already redeemed",
                ));
            }

            let (player_session, admission, source_server_id) = match handshake.kind.as_deref() {
                Some("direct") => {
                    diesel::insert_into(game_player_sessions::table)
                        .values(NewGamePlayerSessionRow {
                            id: &direct_player_session_id,
                            account_uuid,
                            process_session_id,
                            current_server_id: &server_id,
                            status: "active",
                        })
                        .execute(connection)
                        .map_err(|error| {
                            map_write_error(error, "game_player_session", &direct_player_session_id)
                        })?;
                    let player = game_player_sessions::table
                        .find(&direct_player_session_id)
                        .select(GamePlayerSession::as_select())
                        .first(connection)?;
                    (player, "direct".to_owned(), None)
                }
                Some("transfer") => {
                    let transfer_id = handshake.transfer_id.as_deref().ok_or_else(|| {
                        game_error("handshake_invalid", "transfer handshake has no transfer")
                    })?;
                    let transfer = game_transfer_tickets::table
                        .find(transfer_id)
                        .select(GameTransferTicket::as_select())
                        .first(connection)?;
                    if transfer.status != "reserved"
                        || transfer.expires_at <= now
                        || transfer
                            .reservation_expires_at
                            .is_none_or(|expiry| expiry <= now)
                        || transfer.target_server_id.as_deref() != Some(&server_id)
                        || transfer.target_handshake_id.as_deref() != Some(&handshake_id)
                        || transfer.process_session_id != process_session_id
                        || transfer.account_uuid != account_uuid
                    {
                        return Err(game_error(
                            "transfer_unavailable",
                            "reserved transfer is invalid or expired",
                        ));
                    }

                    let moved = diesel::update(
                        game_player_sessions::table
                            .filter(game_player_sessions::id.eq(&transfer.player_session_id))
                            .filter(
                                game_player_sessions::current_server_id
                                    .eq(&transfer.source_server_id),
                            )
                            .filter(game_player_sessions::status.eq("active")),
                    )
                    .set((
                        game_player_sessions::current_server_id.eq(&server_id),
                        game_player_sessions::updated_at.eq(now),
                    ))
                    .execute(connection)?;
                    if moved != 1 {
                        return Err(game_error(
                            "transfer_stale",
                            "player session is no longer on the source server",
                        ));
                    }
                    let transfer_consumed = diesel::update(
                        game_transfer_tickets::table
                            .filter(game_transfer_tickets::id.eq(&transfer.id))
                            .filter(game_transfer_tickets::status.eq("reserved"))
                            .filter(game_transfer_tickets::reservation_expires_at.gt(now)),
                    )
                    .set((
                        game_transfer_tickets::status.eq("consumed"),
                        game_transfer_tickets::consumed_at.eq(Some(now)),
                    ))
                    .execute(connection)?;
                    if transfer_consumed != 1 {
                        return Err(game_error(
                            "transfer_unavailable",
                            "transfer was already consumed or its reservation expired",
                        ));
                    }
                    let player = game_player_sessions::table
                        .find(&transfer.player_session_id)
                        .select(GamePlayerSession::as_select())
                        .first(connection)?;
                    (
                        player,
                        "transfer".to_owned(),
                        Some(transfer.source_server_id),
                    )
                }
                _ => {
                    return Err(game_error(
                        "handshake_invalid",
                        "authorized handshake has an invalid admission kind",
                    ));
                }
            };

            let account = accounts::table
                .find(account_uuid)
                .select(Account::as_select())
                .first(connection)?;
            Ok(GameAdmission {
                player_session,
                account,
                admission,
                source_server_id,
            })
        })
    }

    pub fn create_game_transfer(
        &self,
        authenticated_server_id: &str,
        request: &CreateGameTransfer,
        now: NaiveDateTime,
    ) -> Result<CreatedGameTransfer> {
        let source_server_id = normalize_uuid("server_id", authenticated_server_id)?;
        let transfer_id = request.transfer_id.hyphenated().to_string();
        let player_session_id = request.player_session_id.hyphenated().to_string();
        let ticket_hash = normalize_sha256_hex("ticket_hash", &request.ticket_hash)?;
        let mut connection = self.connection()?;

        connection.transaction::<CreatedGameTransfer, DatabaseError, _>(|connection| {
            require_active_server(connection, &source_server_id, now)?;
            let player = game_player_sessions::table
                .find(&player_session_id)
                .select(GamePlayerSession::as_select())
                .first(connection)
                .optional()?
                .ok_or_else(|| {
                    game_error("player_session_not_found", "player session does not exist")
                })?;
            if player.status != "active" || player.current_server_id != source_server_id {
                return Err(game_error(
                    "player_session_mismatch",
                    "player session is not active on the authenticated server",
                ));
            }
            let process = game_process_sessions::table
                .find(&player.process_session_id)
                .select(GameProcessSession::as_select())
                .first(connection)?;
            if process.expires_at <= now
                || process.revoked_at.is_some()
                || process.account_uuid != player.account_uuid
            {
                return Err(game_error(
                    "process_session_expired",
                    "the player's game process session is no longer active",
                ));
            }

            diesel::insert_into(game_transfer_tickets::table)
                .values(NewGameTransferTicketRow {
                    id: &transfer_id,
                    ticket_hash: &ticket_hash,
                    player_session_id: &player.id,
                    process_session_id: &player.process_session_id,
                    account_uuid: &player.account_uuid,
                    source_server_id: &source_server_id,
                    target_server_id: None,
                    target_handshake_id: None,
                    status: "created",
                    expires_at: request.expires_at,
                    reservation_expires_at: None,
                })
                .execute(connection)
                .map_err(|error| map_write_error(error, "game_transfer_ticket", &transfer_id))?;
            let transfer = game_transfer_tickets::table
                .find(&transfer_id)
                .select(GameTransferTicket::as_select())
                .first(connection)?;
            Ok(CreatedGameTransfer { transfer })
        })
    }

    pub fn game_transfer_for_source(
        &self,
        transfer_id: &str,
        source_server_id: &str,
        now: NaiveDateTime,
    ) -> Result<Option<GameTransferTicket>> {
        let transfer_id = normalize_uuid("transfer_id", transfer_id)?;
        let source_server_id = normalize_uuid("server_id", source_server_id)?;
        let mut connection = self.connection()?;
        connection.transaction::<Option<GameTransferTicket>, DatabaseError, _>(|connection| {
            let Some(mut transfer) = game_transfer_tickets::table
                .find(&transfer_id)
                .filter(game_transfer_tickets::source_server_id.eq(&source_server_id))
                .select(GameTransferTicket::as_select())
                .first(connection)
                .optional()?
            else {
                return Ok(None);
            };

            if transfer_is_expired(&transfer, now) {
                diesel::update(
                    game_transfer_tickets::table
                        .find(&transfer.id)
                        .filter(game_transfer_tickets::status.eq_any(["created", "reserved"])),
                )
                .set(game_transfer_tickets::status.eq("expired"))
                .execute(connection)?;
                transfer.status = "expired".to_owned();
            }
            Ok(Some(transfer))
        })
    }

    pub fn cleanup_game_auth(
        &self,
        now: NaiveDateTime,
        delete_before: NaiveDateTime,
    ) -> Result<usize> {
        let mut connection = self.connection()?;
        connection.transaction::<usize, DatabaseError, _>(|connection| {
            let expired_server_ids = game_server_instances::table
                .filter(game_server_instances::status.eq("active"))
                .filter(game_server_instances::expires_at.le(now))
                .select(game_server_instances::server_id)
                .load::<String>(connection)?;
            let mut changed = diesel::update(
                game_server_instances::table
                    .filter(game_server_instances::server_id.eq_any(&expired_server_ids)),
            )
            .set(game_server_instances::status.eq("expired"))
            .execute(connection)?;

            let inactive_server_ids = game_server_instances::table
                .filter(game_server_instances::status.ne("active"))
                .select(game_server_instances::server_id)
                .load::<String>(connection)?;
            changed += disconnect_players_on_servers(connection, &inactive_server_ids, now)?;

            let expired_process_ids = game_process_sessions::table
                .filter(
                    game_process_sessions::expires_at
                        .le(now)
                        .or(game_process_sessions::revoked_at.is_not_null()),
                )
                .select(game_process_sessions::id)
                .load::<String>(connection)?;
            changed += diesel::update(
                game_player_sessions::table
                    .filter(game_player_sessions::status.eq("active"))
                    .filter(game_player_sessions::process_session_id.eq_any(expired_process_ids)),
            )
            .set((
                game_player_sessions::status.eq("disconnected"),
                game_player_sessions::updated_at.eq(now),
                game_player_sessions::disconnected_at.eq(Some(now)),
            ))
            .execute(connection)?;

            changed += diesel::update(
                game_transfer_tickets::table
                    .filter(game_transfer_tickets::status.eq_any(["created", "reserved"]))
                    .filter(
                        game_transfer_tickets::expires_at.le(now).or(
                            game_transfer_tickets::reservation_expires_at
                                .is_not_null()
                                .and(game_transfer_tickets::reservation_expires_at.le(now)),
                        ),
                    ),
            )
            .set(game_transfer_tickets::status.eq("expired"))
            .execute(connection)?;

            changed += diesel::delete(
                game_launch_tickets::table.filter(
                    game_launch_tickets::expires_at
                        .lt(delete_before)
                        .or(game_launch_tickets::consumed_at.lt(delete_before)),
                ),
            )
            .execute(connection)?;
            changed += diesel::delete(
                game_handshakes::table.filter(
                    game_handshakes::expires_at
                        .lt(delete_before)
                        .or(game_handshakes::consumed_at.lt(delete_before)),
                ),
            )
            .execute(connection)?;
            changed += diesel::delete(
                game_transfer_tickets::table.filter(
                    game_transfer_tickets::expires_at
                        .lt(delete_before)
                        .or(game_transfer_tickets::consumed_at.lt(delete_before)),
                ),
            )
            .execute(connection)?;
            changed += diesel::delete(
                game_player_sessions::table
                    .filter(game_player_sessions::status.eq("disconnected"))
                    .filter(game_player_sessions::disconnected_at.lt(delete_before)),
            )
            .execute(connection)?;
            changed += diesel::delete(
                game_process_sessions::table
                    .filter(game_process_sessions::expires_at.lt(delete_before)),
            )
            .execute(connection)?;
            changed += diesel::delete(
                game_server_instances::table.filter(
                    game_server_instances::status
                        .eq("expired")
                        .and(game_server_instances::expires_at.lt(delete_before))
                        .or(game_server_instances::status
                            .eq("closed")
                            .and(game_server_instances::closed_at.lt(delete_before))),
                ),
            )
            .execute(connection)?;
            Ok(changed)
        })
    }
}

fn require_active_server(
    connection: &mut crate::db::DbConnection,
    server_id: &str,
    now: NaiveDateTime,
) -> Result<GameServerInstance> {
    game_server_instances::table
        .find(server_id)
        .filter(game_server_instances::status.eq("active"))
        .filter(game_server_instances::expires_at.gt(now))
        .select(GameServerInstance::as_select())
        .first(connection)
        .optional()?
        .ok_or_else(|| game_error("server_instance_expired", "server instance is not active"))
}

fn disconnect_players_on_servers(
    connection: &mut crate::db::DbConnection,
    server_ids: &[String],
    now: NaiveDateTime,
) -> Result<usize> {
    if server_ids.is_empty() {
        return Ok(0);
    }
    Ok(diesel::update(
        game_player_sessions::table
            .filter(game_player_sessions::status.eq("active"))
            .filter(game_player_sessions::current_server_id.eq_any(server_ids)),
    )
    .set((
        game_player_sessions::status.eq("disconnected"),
        game_player_sessions::updated_at.eq(now),
        game_player_sessions::disconnected_at.eq(Some(now)),
    ))
    .execute(connection)?)
}

fn transfer_is_expired(transfer: &GameTransferTicket, now: NaiveDateTime) -> bool {
    matches!(transfer.status.as_str(), "created" | "reserved")
        && (transfer.expires_at <= now
            || transfer
                .reservation_expires_at
                .is_some_and(|expiry| expiry <= now))
}

fn validate_waiting_handshake(
    handshake: &GameHandshake,
    request: &AuthorizeGameHandshake,
    server_id: &str,
    now: NaiveDateTime,
) -> Result<()> {
    if handshake.status != "waiting" || handshake.expires_at <= now {
        return Err(game_error(
            "handshake_unavailable",
            "handshake is expired, authorized, or consumed",
        ));
    }
    if handshake.protocol_version != request.protocol_version
        || handshake.server_id != server_id
        || handshake.server_public_key != request.server_public_key
        || handshake.server_nonce != request.server_nonce
    {
        return Err(game_error(
            "handshake_mismatch",
            "server handshake values do not match the registered handshake",
        ));
    }
    Ok(())
}

fn normalize_uuid(field: &'static str, value: &str) -> Result<String> {
    Uuid::parse_str(value)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|error| DatabaseError::Validation {
            field,
            message: error.to_string(),
        })
}

fn game_error(code: &'static str, message: impl Into<String>) -> DatabaseError {
    DatabaseError::game_auth(code, message)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use tempfile::tempdir;

    use super::*;
    use crate::models::CreateAccount;

    fn database_with_account() -> (Database, Account) {
        let directory = tempdir().unwrap().keep();
        let database =
            Database::connect(directory.join("game-auth.sqlite").to_string_lossy()).unwrap();
        let account = database
            .create_account(CreateAccount {
                uuid: Uuid::new_v4(),
                nickname: "PlayerOne".to_owned(),
                email: "player@example.com".to_owned(),
                password_hash: None,
            })
            .unwrap();
        (database, account)
    }

    fn active_process(
        database: &Database,
        account: &Account,
        now: NaiveDateTime,
    ) -> AuthorizedGameProcess {
        database
            .create_game_launch_ticket(&"11".repeat(32), &account.uuid, now + Duration::minutes(1))
            .unwrap();
        database
            .consume_game_launch_ticket(
                &"11".repeat(32),
                Uuid::new_v4(),
                &"22".repeat(32),
                now,
                now + Duration::hours(8),
            )
            .unwrap()
            .unwrap()
    }

    fn server(database: &Database, secret_byte: &str, now: NaiveDateTime) -> GameServerInstance {
        database
            .create_game_server_instance(
                Uuid::new_v4(),
                &secret_byte.repeat(32),
                now,
                now + Duration::minutes(10),
            )
            .unwrap()
    }

    #[test]
    fn dynamic_server_lease_cannot_be_revived_after_expiry() {
        let (database, _) = database_with_account();
        let now = chrono::Utc::now().naive_utc();
        let server = server(&database, "aa", now);
        assert!(
            database
                .heartbeat_game_server_instance(
                    &server.server_id,
                    &"aa".repeat(32),
                    now + Duration::minutes(1),
                    now + Duration::minutes(11),
                )
                .unwrap()
                .is_some()
        );
        assert!(
            database
                .heartbeat_game_server_instance(
                    &server.server_id,
                    &"aa".repeat(32),
                    now + Duration::minutes(12),
                    now + Duration::minutes(22),
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn transfer_binds_target_during_authorization_and_moves_on_redeem() {
        let (database, account) = database_with_account();
        let now = chrono::Utc::now().naive_utc();
        let source = server(&database, "aa", now);
        let target = server(&database, "bb", now);
        let process = active_process(&database, &account, now);

        let direct_handshake_id = Uuid::new_v4();
        database
            .register_game_handshake(
                &source.server_id,
                direct_handshake_id,
                1,
                &"A".repeat(43),
                &"B".repeat(43),
                now,
                now + Duration::seconds(20),
            )
            .unwrap();
        database
            .authorize_game_handshake(
                &process,
                &AuthorizeGameHandshake {
                    handshake_id: direct_handshake_id,
                    protocol_version: 1,
                    server_id: source.server_id.clone(),
                    server_public_key: "A".repeat(43),
                    server_nonce: "B".repeat(43),
                    client_public_key: "C".repeat(43),
                    client_nonce: "D".repeat(43),
                    handshake_hash: "E".repeat(43),
                    transfer_ticket_hash: None,
                    reservation_expires_at: now + Duration::seconds(20),
                },
                now,
            )
            .unwrap();
        let direct = database
            .redeem_game_handshake(
                &source.server_id,
                &RedeemGameHandshake {
                    handshake_id: direct_handshake_id,
                    client_public_key: "C".repeat(43),
                    client_nonce: "D".repeat(43),
                    handshake_hash: "E".repeat(43),
                    direct_player_session_id: Uuid::new_v4(),
                },
                now,
            )
            .unwrap();

        let transfer_id = Uuid::new_v4();
        let transfer = database
            .create_game_transfer(
                &source.server_id,
                &CreateGameTransfer {
                    transfer_id,
                    ticket_hash: "44".repeat(32),
                    player_session_id: Uuid::parse_str(&direct.player_session.id).unwrap(),
                    expires_at: now + Duration::seconds(60),
                },
                now,
            )
            .unwrap();
        assert!(transfer.transfer.target_server_id.is_none());

        let transfer_handshake_id = Uuid::new_v4();
        database
            .register_game_handshake(
                &target.server_id,
                transfer_handshake_id,
                1,
                &"F".repeat(43),
                &"G".repeat(43),
                now,
                now + Duration::seconds(20),
            )
            .unwrap();
        database
            .authorize_game_handshake(
                &process,
                &AuthorizeGameHandshake {
                    handshake_id: transfer_handshake_id,
                    protocol_version: 1,
                    server_id: target.server_id.clone(),
                    server_public_key: "F".repeat(43),
                    server_nonce: "G".repeat(43),
                    client_public_key: "H".repeat(43),
                    client_nonce: "I".repeat(43),
                    handshake_hash: "J".repeat(43),
                    transfer_ticket_hash: Some("44".repeat(32)),
                    reservation_expires_at: now + Duration::seconds(20),
                },
                now,
            )
            .unwrap();

        let reserved = database
            .game_transfer_for_source(&transfer_id.to_string(), &source.server_id, now)
            .unwrap()
            .unwrap();
        assert_eq!(reserved.status, "reserved");
        assert_eq!(
            reserved.target_server_id.as_deref(),
            Some(target.server_id.as_str())
        );
        assert_eq!(
            reserved.target_handshake_id.as_deref(),
            Some(transfer_handshake_id.to_string().as_str())
        );

        let moved = database
            .redeem_game_handshake(
                &target.server_id,
                &RedeemGameHandshake {
                    handshake_id: transfer_handshake_id,
                    client_public_key: "H".repeat(43),
                    client_nonce: "I".repeat(43),
                    handshake_hash: "J".repeat(43),
                    direct_player_session_id: Uuid::new_v4(),
                },
                now,
            )
            .unwrap();
        assert_eq!(moved.admission, "transfer");
        assert_eq!(
            moved.source_server_id.as_deref(),
            Some(source.server_id.as_str())
        );
        assert_eq!(moved.player_session.id, direct.player_session.id);
        assert_eq!(moved.player_session.current_server_id, target.server_id);

        let cleanup_now = now + Duration::minutes(11);
        database
            .cleanup_game_auth(cleanup_now, now - Duration::hours(24))
            .unwrap();
        let mut connection = database.connection().unwrap();
        let disconnected = game_player_sessions::table
            .find(&moved.player_session.id)
            .select(GamePlayerSession::as_select())
            .first(&mut connection)
            .unwrap();
        assert_eq!(disconnected.status, "disconnected");
        assert_eq!(disconnected.disconnected_at, Some(cleanup_now));
    }
}
