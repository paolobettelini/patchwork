use chrono::NaiveDateTime;
use diesel::OptionalExtension;
use diesel::prelude::*;
use url::Url;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{GithubAccount, GithubOAuthState, NewGithubAccountRow, NewGithubOAuthStateRow};
use crate::schema::{github_accounts, github_oauth_states};

impl Database {
    pub fn create_github_oauth_state(
        &self,
        state_hash: &str,
        account_uuid: &str,
        completion_url: &str,
        now: NaiveDateTime,
        expires_at: NaiveDateTime,
    ) -> Result<()> {
        let state_hash = normalize_sha256_hex("state_hash", state_hash)?;
        let account_uuid = normalize_uuid("account_uuid", account_uuid)?;
        let completion_url = normalize_bounded("completion_url", completion_url, 1, 2048)?;
        if expires_at <= now {
            return Err(DatabaseError::Validation {
                field: "expires_at",
                message: "must be in the future".to_owned(),
            });
        }
        let row = NewGithubOAuthStateRow {
            state_hash: &state_hash,
            account_uuid: &account_uuid,
            completion_url: &completion_url,
            expires_at,
        };

        let mut connection = self.connection()?;
        diesel::delete(
            github_oauth_states::table.filter(
                github_oauth_states::expires_at
                    .le(now)
                    .or(github_oauth_states::used_at.is_not_null()),
            ),
        )
        .execute(&mut connection)?;
        diesel::insert_into(github_oauth_states::table)
            .values(&row)
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "github_oauth_state", &state_hash))?;
        Ok(())
    }

    pub fn consume_github_oauth_state(
        &self,
        state_hash: &str,
        now: NaiveDateTime,
    ) -> Result<Option<GithubOAuthState>> {
        let state_hash = normalize_sha256_hex("state_hash", state_hash)?;
        let mut connection = self.connection()?;
        connection.transaction::<Option<GithubOAuthState>, DatabaseError, _>(|connection| {
            let Some(state) = github_oauth_states::table
                .find(&state_hash)
                .select(GithubOAuthState::as_select())
                .first(connection)
                .optional()?
            else {
                return Ok(None);
            };

            if state.used_at.is_some() || state.expires_at <= now {
                return Ok(None);
            }

            let updated = diesel::update(
                github_oauth_states::table
                    .filter(github_oauth_states::state_hash.eq(&state_hash))
                    .filter(github_oauth_states::used_at.is_null())
                    .filter(github_oauth_states::expires_at.gt(now)),
            )
            .set(github_oauth_states::used_at.eq(Some(now)))
            .execute(connection)?;

            Ok((updated == 1).then_some(state))
        })
    }

    pub fn get_github_account(&self, account_uuid: Uuid) -> Result<Option<GithubAccount>> {
        let account_uuid = account_uuid.hyphenated().to_string();
        let mut connection = self.connection()?;
        Ok(github_accounts::table
            .find(account_uuid)
            .select(GithubAccount::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn get_github_account_by_user_id(
        &self,
        github_user_id: i64,
    ) -> Result<Option<GithubAccount>> {
        validate_github_user_id(github_user_id)?;
        let mut connection = self.connection()?;
        Ok(github_accounts::table
            .filter(github_accounts::github_user_id.eq(github_user_id))
            .select(GithubAccount::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn link_github_account(
        &self,
        account_uuid: Uuid,
        github_user_id: i64,
        github_login: &str,
        github_avatar_url: &str,
        now: NaiveDateTime,
    ) -> Result<GithubAccount> {
        validate_github_user_id(github_user_id)?;
        let account_uuid = account_uuid.hyphenated().to_string();
        let github_login = normalize_bounded("github_login", github_login, 1, 255)?;
        let github_avatar_url = normalize_avatar_url(github_avatar_url)?;
        let mut connection = self.connection()?;

        connection.transaction::<GithubAccount, DatabaseError, _>(|connection| {
            if let Some(owner) = github_accounts::table
                .filter(github_accounts::github_user_id.eq(github_user_id))
                .select(GithubAccount::as_select())
                .first(connection)
                .optional()?
            {
                if owner.account_uuid != account_uuid {
                    return Err(DatabaseError::Conflict {
                        entity: "github_account",
                        key: github_user_id.to_string(),
                    });
                }
            }

            let updated = diesel::update(github_accounts::table.find(&account_uuid))
                .set((
                    github_accounts::github_user_id.eq(github_user_id),
                    github_accounts::github_login.eq(&github_login),
                    github_accounts::github_avatar_url.eq(&github_avatar_url),
                    github_accounts::updated_at.eq(now),
                ))
                .execute(connection)
                .map_err(|error| {
                    map_write_error(error, "github_account", github_user_id.to_string())
                })?;

            if updated == 0 {
                let row = NewGithubAccountRow {
                    account_uuid: &account_uuid,
                    github_user_id,
                    github_login: &github_login,
                    github_avatar_url: &github_avatar_url,
                };
                diesel::insert_into(github_accounts::table)
                    .values(&row)
                    .execute(connection)
                    .map_err(|error| {
                        map_write_error(error, "github_account", github_user_id.to_string())
                    })?;
            }

            github_accounts::table
                .find(&account_uuid)
                .select(GithubAccount::as_select())
                .first(connection)
                .map_err(DatabaseError::from)
        })
    }

    pub fn unlink_github_account(&self, account_uuid: Uuid) -> Result<usize> {
        let account_uuid = account_uuid.hyphenated().to_string();
        let mut connection = self.connection()?;
        Ok(diesel::delete(github_accounts::table.find(account_uuid)).execute(&mut connection)?)
    }
}

fn validate_github_user_id(value: i64) -> Result<()> {
    if value > 0 {
        Ok(())
    } else {
        Err(DatabaseError::Validation {
            field: "github_user_id",
            message: "must be greater than zero".to_owned(),
        })
    }
}

fn normalize_avatar_url(value: &str) -> Result<String> {
    let value = normalize_bounded("github_avatar_url", value, 1, 2048)?;
    let url = Url::parse(&value).map_err(|error| DatabaseError::Validation {
        field: "github_avatar_url",
        message: error.to_string(),
    })?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(DatabaseError::Validation {
            field: "github_avatar_url",
            message: "must be an absolute HTTPS URL".to_owned(),
        });
    }
    Ok(value)
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

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use super::*;
    use crate::models::CreateAccount;

    #[test]
    fn state_is_single_use_and_github_user_id_is_unique() {
        let directory = tempdir().unwrap();
        let database = Database::connect(directory.path().join("test.sqlite").to_string_lossy())
            .expect("database should open");
        let first_uuid = Uuid::new_v4();
        let second_uuid = Uuid::new_v4();
        database
            .create_account(CreateAccount {
                uuid: first_uuid,
                nickname: "first".to_owned(),
                email: "first@example.com".to_owned(),
                password_hash: None,
            })
            .unwrap();
        database
            .create_account(CreateAccount {
                uuid: second_uuid,
                nickname: "second".to_owned(),
                email: "second@example.com".to_owned(),
                password_hash: None,
            })
            .unwrap();

        let now = Utc::now().naive_utc();
        let state_hash = "a".repeat(64);
        database
            .create_github_oauth_state(
                &state_hash,
                &first_uuid.to_string(),
                "http://127.0.0.1:51342/github-connected",
                now,
                now + Duration::minutes(10),
            )
            .unwrap();
        assert!(
            database
                .consume_github_oauth_state(&state_hash, now)
                .unwrap()
                .is_some()
        );
        assert!(
            database
                .consume_github_oauth_state(&state_hash, now)
                .unwrap()
                .is_none()
        );

        database
            .link_github_account(
                first_uuid,
                42,
                "octocat",
                "https://avatars.githubusercontent.com/u/42?v=4",
                now,
            )
            .unwrap();
        let conflict = database
            .link_github_account(
                second_uuid,
                42,
                "octocat-renamed",
                "https://avatars.githubusercontent.com/u/42?v=4",
                now,
            )
            .unwrap_err();
        assert!(matches!(conflict, DatabaseError::Conflict { .. }));
    }
}
