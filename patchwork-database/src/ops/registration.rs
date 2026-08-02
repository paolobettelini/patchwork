use chrono::{Duration, NaiveDateTime};
use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{
    Account, CreatePendingRegistration, NewAccountRow, NewPendingRegistrationRow,
    PendingRegistration, PendingRegistrationVerification,
};
use crate::schema::{accounts, pending_registrations};
use crate::validation::{
    normalize_email, normalize_nickname, normalize_password_hash, normalize_sha256_hex,
};

const REQUEST_COOLDOWN_SECONDS: i64 = 60;
const MAX_VERIFICATION_ATTEMPTS: i32 = 5;

impl Database {
    pub fn create_pending_registration(
        &self,
        input: CreatePendingRegistration,
        now: NaiveDateTime,
    ) -> Result<PendingRegistration> {
        let verification_id_hash =
            normalize_sha256_hex("verification_id_hash", &input.verification_id_hash)?;
        let code_hash = normalize_sha256_hex("code_hash", &input.code_hash)?;
        let email = normalize_email(&input.email)?;
        let nickname = normalize_nickname(&input.nickname)?;
        let password_hash = normalize_password_hash(&input.password_hash)?;
        if input.expires_at <= now {
            return Err(DatabaseError::Validation {
                field: "expires_at",
                message: "must be in the future".to_owned(),
            });
        }

        let row = NewPendingRegistrationRow {
            verification_id_hash: &verification_id_hash,
            code_hash: &code_hash,
            email: &email,
            nickname: &nickname,
            password_hash: &password_hash,
            expires_at: input.expires_at,
        };
        let cooldown_cutoff = now - Duration::seconds(REQUEST_COOLDOWN_SECONDS);
        let mut connection = self.connection()?;

        connection.transaction::<PendingRegistration, DatabaseError, _>(|connection| {
            diesel::delete(
                pending_registrations::table.filter(pending_registrations::expires_at.le(now)),
            )
            .execute(connection)?;

            let existing = pending_registrations::table
                .filter(
                    pending_registrations::email
                        .eq(&email)
                        .or(pending_registrations::nickname.eq(&nickname)),
                )
                .select(PendingRegistration::as_select())
                .first(connection)
                .optional()?;

            if existing
                .as_ref()
                .is_some_and(|pending| pending.created_at > cooldown_cutoff)
            {
                return Err(DatabaseError::Conflict {
                    entity: "email verification",
                    key: "a code was already requested recently".to_owned(),
                });
            }

            diesel::delete(
                pending_registrations::table.filter(
                    pending_registrations::email
                        .eq(&email)
                        .or(pending_registrations::nickname.eq(&nickname)),
                ),
            )
            .execute(connection)?;

            diesel::insert_into(pending_registrations::table)
                .values(&row)
                .execute(connection)
                .map_err(|error| {
                    map_write_error(error, "pending registration", &verification_id_hash)
                })?;

            pending_registrations::table
                .find(&verification_id_hash)
                .select(PendingRegistration::as_select())
                .first(connection)
                .map_err(DatabaseError::from)
        })
    }

    pub fn verify_pending_registration(
        &self,
        verification_id_hash: &str,
        code_hash: &str,
        account_uuid: Uuid,
        now: NaiveDateTime,
    ) -> Result<PendingRegistrationVerification> {
        let verification_id_hash =
            normalize_sha256_hex("verification_id_hash", verification_id_hash)?;
        let code_hash = normalize_sha256_hex("code_hash", code_hash)?;
        let account_uuid = account_uuid.hyphenated().to_string();
        let mut connection = self.connection()?;

        connection.transaction::<PendingRegistrationVerification, DatabaseError, _>(|connection| {
            let Some(pending) = pending_registrations::table
                .find(&verification_id_hash)
                .select(PendingRegistration::as_select())
                .first(connection)
                .optional()?
            else {
                return Ok(PendingRegistrationVerification::ExpiredOrMissing);
            };

            if pending.expires_at <= now || pending.attempts >= MAX_VERIFICATION_ATTEMPTS {
                diesel::delete(pending_registrations::table.find(&verification_id_hash))
                    .execute(connection)?;
                return Ok(PendingRegistrationVerification::ExpiredOrMissing);
            }

            if pending.code_hash != code_hash {
                let attempts = pending.attempts + 1;
                let attempts_remaining = MAX_VERIFICATION_ATTEMPTS - attempts;
                if attempts_remaining == 0 {
                    diesel::delete(pending_registrations::table.find(&verification_id_hash))
                        .execute(connection)?;
                } else {
                    diesel::update(pending_registrations::table.find(&verification_id_hash))
                        .set(pending_registrations::attempts.eq(attempts))
                        .execute(connection)?;
                }
                return Ok(PendingRegistrationVerification::InvalidCode { attempts_remaining });
            }

            let account_row = NewAccountRow {
                uuid: &account_uuid,
                nickname: &pending.nickname,
                email: &pending.email,
                password_hash: Some(&pending.password_hash),
            };
            diesel::insert_into(accounts::table)
                .values(&account_row)
                .execute(connection)
                .map_err(|error| map_write_error(error, "account", &account_uuid))?;
            diesel::delete(pending_registrations::table.find(&verification_id_hash))
                .execute(connection)?;
            let account = accounts::table
                .find(&account_uuid)
                .select(Account::as_select())
                .first(connection)?;
            Ok(PendingRegistrationVerification::Verified(account))
        })
    }

    pub fn delete_pending_registration(&self, verification_id_hash: &str) -> Result<usize> {
        let verification_id_hash =
            normalize_sha256_hex("verification_id_hash", verification_id_hash)?;
        let mut connection = self.connection()?;
        Ok(
            diesel::delete(pending_registrations::table.find(verification_id_hash))
                .execute(&mut connection)?,
        )
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use super::*;

    fn database() -> Database {
        let directory = tempdir().unwrap().keep();
        Database::connect(directory.join("registration.sqlite").to_string_lossy()).unwrap()
    }

    fn pending(now: NaiveDateTime) -> CreatePendingRegistration {
        CreatePendingRegistration {
            verification_id_hash: "a".repeat(64),
            code_hash: "b".repeat(64),
            email: "person@example.com".to_owned(),
            nickname: "Patchworker".to_owned(),
            password_hash: "p".repeat(60),
            expires_at: now + Duration::minutes(10),
        }
    }

    #[test]
    fn verification_counts_failures_and_consumes_a_correct_code() {
        let database = database();
        let now = Utc::now().naive_utc();
        database
            .create_pending_registration(pending(now), now)
            .unwrap();

        let wrong = database
            .verify_pending_registration(&"a".repeat(64), &"c".repeat(64), Uuid::new_v4(), now)
            .unwrap();
        assert!(matches!(
            wrong,
            PendingRegistrationVerification::InvalidCode {
                attempts_remaining: 4
            }
        ));

        let verified = database
            .verify_pending_registration(&"a".repeat(64), &"b".repeat(64), Uuid::new_v4(), now)
            .unwrap();
        assert!(matches!(
            verified,
            PendingRegistrationVerification::Verified(_)
        ));

        let consumed = database
            .verify_pending_registration(&"a".repeat(64), &"b".repeat(64), Uuid::new_v4(), now)
            .unwrap();
        assert!(matches!(
            consumed,
            PendingRegistrationVerification::ExpiredOrMissing
        ));
    }

    #[test]
    fn expired_registration_cannot_be_verified() {
        let database = database();
        let now = Utc::now().naive_utc();
        database
            .create_pending_registration(pending(now), now)
            .unwrap();

        let result = database
            .verify_pending_registration(
                &"a".repeat(64),
                &"b".repeat(64),
                Uuid::new_v4(),
                now + Duration::minutes(11),
            )
            .unwrap();
        assert!(matches!(
            result,
            PendingRegistrationVerification::ExpiredOrMissing
        ));
    }
}
