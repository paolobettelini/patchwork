use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{Account, CreateAccount, NewAccountRow};
use crate::schema::accounts;
use crate::validation::{normalize_email, normalize_nickname, normalize_password_hash};

impl Database {
    pub fn create_account(&self, input: CreateAccount) -> Result<Account> {
        let uuid = input.uuid.hyphenated().to_string();
        let nickname = normalize_nickname(&input.nickname)?;
        let email = normalize_email(&input.email)?;
        let password_hash = input
            .password_hash
            .as_deref()
            .map(normalize_password_hash)
            .transpose()?;
        let row = NewAccountRow {
            uuid: &uuid,
            nickname: &nickname,
            email: &email,
            password_hash: password_hash.as_deref(),
        };

        let mut connection = self.connection()?;
        diesel::insert_into(accounts::table)
            .values(&row)
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "account", &uuid))?;

        accounts::table
            .find(&uuid)
            .select(Account::as_select())
            .first(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn get_account(&self, account_uuid: Uuid) -> Result<Option<Account>> {
        let uuid = account_uuid.hyphenated().to_string();
        let mut connection = self.connection()?;
        Ok(accounts::table
            .find(uuid)
            .select(Account::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn get_account_by_email(&self, value: &str) -> Result<Option<Account>> {
        let email = normalize_email(value)?;
        let mut connection = self.connection()?;
        Ok(accounts::table
            .filter(accounts::email.eq(email))
            .select(Account::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn get_account_by_nickname(&self, value: &str) -> Result<Option<Account>> {
        let nickname = normalize_nickname(value)?;
        let mut connection = self.connection()?;
        Ok(accounts::table
            .filter(accounts::nickname.eq(nickname))
            .select(Account::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn get_account_by_login_identifier(&self, value: &str) -> Result<Option<Account>> {
        if value.contains('@') {
            self.get_account_by_email(value)
        } else {
            self.get_account_by_nickname(value)
        }
    }

    pub fn update_account_nickname(&self, account_uuid: Uuid, value: &str) -> Result<Account> {
        let uuid = account_uuid.hyphenated().to_string();
        let nickname = normalize_nickname(value)?;
        let mut connection = self.connection()?;
        diesel::update(accounts::table.find(&uuid))
            .set(accounts::nickname.eq(&nickname))
            .execute(&mut connection)
            .map_err(|error| map_write_error(error, "account", &nickname))?;

        accounts::table
            .find(&uuid)
            .select(Account::as_select())
            .first(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn update_account_password_hash(
        &self,
        account_uuid: Uuid,
        password_hash: &str,
    ) -> Result<Account> {
        let uuid = account_uuid.hyphenated().to_string();
        let password_hash = normalize_password_hash(password_hash)?;
        let mut connection = self.connection()?;
        diesel::update(accounts::table.find(&uuid))
            .set(accounts::password_hash.eq(Some(&password_hash)))
            .execute(&mut connection)?;

        accounts::table
            .find(&uuid)
            .select(Account::as_select())
            .first(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn get_or_create_account_by_email(&self, value: &str) -> Result<Account> {
        let email = normalize_email(value)?;
        if let Some(account) = self.get_account_by_email(&email)? {
            return Ok(account);
        }

        let base_nickname = nickname_from_email(&email);
        for attempt in 0..1000 {
            let nickname = candidate_nickname(&base_nickname, attempt);
            match self.create_account(CreateAccount {
                uuid: Uuid::new_v4(),
                nickname,
                email: email.clone(),
                password_hash: None,
            }) {
                Ok(account) => return Ok(account),
                Err(DatabaseError::Conflict {
                    entity: "account", ..
                }) => {
                    if let Some(account) = self.get_account_by_email(&email)? {
                        return Ok(account);
                    }
                }
                Err(error) => return Err(error),
            }
        }

        Err(DatabaseError::Conflict {
            entity: "account",
            key: email,
        })
    }
}

fn nickname_from_email(email: &str) -> String {
    let local_part = email.split('@').next().unwrap_or_default();
    let mut nickname = String::new();
    let mut last_was_separator = false;
    for character in local_part.chars() {
        let next = if character.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(character)
        } else if !last_was_separator {
            last_was_separator = true;
            Some('-')
        } else {
            None
        };

        if let Some(next) = next {
            nickname.push(next);
        }
        if nickname.chars().count() >= 16 {
            break;
        }
    }

    let nickname = nickname.trim_matches('-');
    if nickname.is_empty() {
        "patchworker".to_owned()
    } else {
        nickname.to_owned()
    }
}

fn candidate_nickname(base: &str, attempt: usize) -> String {
    if attempt == 0 {
        return base.to_owned();
    }

    let suffix = format!("-{attempt}");
    let available = 16_usize.saturating_sub(suffix.chars().count());
    let mut candidate = base.chars().take(available).collect::<String>();
    candidate.push_str(&suffix);
    candidate
}
