use chrono::NaiveDateTime;
use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{
    Account, AppToken, NewAppTokenRow, NewOAuthAuthorizationCodeRow, NewWebSessionRow,
    OAuthAuthorizationCode,
};
use crate::schema::{accounts, app_tokens, oauth_authorization_codes, web_sessions};

impl Database {
    pub fn create_web_session(
        &self,
        token_hash: &str,
        account_uuid: &str,
        expires_at: NaiveDateTime,
    ) -> Result<()> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let account_uuid = normalize_uuid("account_uuid", account_uuid)?;
        let row = NewWebSessionRow {
            token_hash: &token_hash,
            account_uuid: &account_uuid,
            expires_at,
        };

        let mut connection = self.connection()?;
        diesel::insert_into(web_sessions::table)
            .values(&row)
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "web_session", &token_hash))?;
        Ok(())
    }

    pub fn account_for_web_session(
        &self,
        token_hash: &str,
        now: NaiveDateTime,
    ) -> Result<Option<Account>> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let mut connection = self.connection()?;
        Ok(web_sessions::table
            .inner_join(accounts::table)
            .filter(web_sessions::token_hash.eq(token_hash))
            .filter(web_sessions::expires_at.gt(now))
            .select(Account::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn delete_web_session(&self, token_hash: &str) -> Result<usize> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let mut connection = self.connection()?;
        Ok(diesel::delete(web_sessions::table.find(token_hash)).execute(&mut connection)?)
    }

    pub fn create_oauth_authorization_code(
        &self,
        code_hash: &str,
        account_uuid: &str,
        client_id: &str,
        redirect_uri: &str,
        code_challenge: &str,
        expires_at: NaiveDateTime,
    ) -> Result<()> {
        let code_hash = normalize_sha256_hex("code_hash", code_hash)?;
        let account_uuid = normalize_uuid("account_uuid", account_uuid)?;
        let client_id = normalize_bounded("client_id", client_id, 1, 128)?;
        let redirect_uri = normalize_bounded("redirect_uri", redirect_uri, 1, 2048)?;
        let code_challenge = normalize_bounded("code_challenge", code_challenge, 43, 128)?;
        let row = NewOAuthAuthorizationCodeRow {
            code_hash: &code_hash,
            account_uuid: &account_uuid,
            client_id: &client_id,
            redirect_uri: &redirect_uri,
            code_challenge: &code_challenge,
            expires_at,
        };

        let mut connection = self.connection()?;
        diesel::insert_into(oauth_authorization_codes::table)
            .values(&row)
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "oauth_authorization_code", &code_hash))?;
        Ok(())
    }

    pub fn consume_oauth_authorization_code(
        &self,
        code_hash: &str,
        now: NaiveDateTime,
    ) -> Result<Option<OAuthAuthorizationCode>> {
        let code_hash = normalize_sha256_hex("code_hash", code_hash)?;
        let mut connection = self.connection()?;
        let code = connection
            .transaction::<Option<OAuthAuthorizationCode>, diesel::result::Error, _>(
                |connection| {
                    let Some(code) = oauth_authorization_codes::table
                        .find(&code_hash)
                        .select(OAuthAuthorizationCode::as_select())
                        .first(connection)
                        .optional()?
                    else {
                        return Ok(None);
                    };

                    if code.used_at.is_some() || code.expires_at <= now {
                        return Ok(None);
                    }

                    diesel::update(oauth_authorization_codes::table.find(&code_hash))
                        .set(oauth_authorization_codes::used_at.eq(Some(now)))
                        .execute(connection)?;

                    Ok(Some(code))
                },
            )?;
        Ok(code)
    }

    pub fn create_app_token(
        &self,
        token_hash: &str,
        account_uuid: &str,
        label: Option<&str>,
        expires_at: NaiveDateTime,
    ) -> Result<()> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let account_uuid = normalize_uuid("account_uuid", account_uuid)?;
        let label = label
            .map(|value| normalize_bounded("label", value, 1, 128))
            .transpose()?;
        let row = NewAppTokenRow {
            token_hash: &token_hash,
            account_uuid: &account_uuid,
            label: label.as_deref(),
            expires_at,
        };

        let mut connection = self.connection()?;
        diesel::insert_into(app_tokens::table)
            .values(&row)
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "app_token", &token_hash))?;
        Ok(())
    }

    pub fn account_for_app_token(
        &self,
        token_hash: &str,
        now: NaiveDateTime,
    ) -> Result<Option<Account>> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let mut connection = self.connection()?;
        let account = app_tokens::table
            .inner_join(accounts::table)
            .filter(app_tokens::token_hash.eq(&token_hash))
            .filter(app_tokens::expires_at.gt(now))
            .select(Account::as_select())
            .first(&mut connection)
            .optional()?;

        if account.is_some() {
            diesel::update(app_tokens::table.find(token_hash))
                .set(app_tokens::last_used_at.eq(Some(now)))
                .execute(&mut connection)?;
        }

        Ok(account)
    }

    pub fn get_app_token(&self, token_hash: &str) -> Result<Option<AppToken>> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let mut connection = self.connection()?;
        Ok(app_tokens::table
            .find(token_hash)
            .select(AppToken::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn delete_app_token(&self, token_hash: &str) -> Result<usize> {
        let token_hash = normalize_sha256_hex("token_hash", token_hash)?;
        let mut connection = self.connection()?;
        Ok(diesel::delete(app_tokens::table.find(token_hash)).execute(&mut connection)?)
    }
}

fn normalize_uuid(field: &'static str, value: &str) -> Result<String> {
    Uuid::parse_str(value)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|error| DatabaseError::Validation {
            field,
            message: error.to_string(),
        })
}

fn normalize_sha256_hex(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(DatabaseError::Validation {
            field,
            message: "must be a SHA-256 hex digest".to_owned(),
        })
    }
}

fn normalize_bounded(
    field: &'static str,
    value: &str,
    min_len: usize,
    max_len: usize,
) -> Result<String> {
    let value = value.trim();
    let length = value.chars().count();
    if length < min_len || length > max_len {
        return Err(DatabaseError::Validation {
            field,
            message: format!("must contain between {min_len} and {max_len} characters"),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(DatabaseError::Validation {
            field,
            message: "must not contain control characters".to_owned(),
        });
    }
    Ok(value.to_owned())
}
