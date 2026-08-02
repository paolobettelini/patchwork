use std::collections::HashSet;

use chrono::NaiveDateTime;
use diesel::OptionalExtension;
use diesel::prelude::*;
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{
    CreateRegistryScan, CreateRegistryScanEntry, Mod, NewModRow, NewModVersionDependencyRow,
    NewModVersionRow, NewRegistryScanEntryRow, NewRegistryScanRow, NewRepositoryRow,
    PublishedRegistryVersion, RegistryPublishResult, RegistryScan, RegistryScanEntry,
    RegistryScanWithEntries, Repository,
};
use crate::schema::{
    mod_version_dependencies, mod_versions, mods, registry_scan_entries, registry_scans,
    repositories,
};
use crate::validation::{
    normalize_package_id, normalize_repo_path, normalize_repository_url, normalize_title,
};

const PROVIDER_GITHUB: &str = "github";
const MAX_SCAN_ENTRIES: usize = 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDependency {
    kind: String,
    target_id: String,
    #[allow(dead_code)]
    available: bool,
}

impl Database {
    pub fn create_registry_scan(
        &self,
        input: CreateRegistryScan,
        now: NaiveDateTime,
    ) -> Result<RegistryScanWithEntries> {
        if input.entries.len() > MAX_SCAN_ENTRIES {
            return validation(
                "entries",
                format!("must contain at most {MAX_SCAN_ENTRIES} mods"),
            );
        }
        if input.github_user_id <= 0 || input.github_repository_id <= 0 {
            return validation("github_id", "GitHub IDs must be greater than zero");
        }
        if input.expires_at <= now {
            return validation("expires_at", "must be in the future");
        }

        let scan_id = input.id.hyphenated().to_string();
        let publisher_uuid = input.publisher_uuid.hyphenated().to_string();
        let repository_owner =
            normalize_bounded("repository_owner", &input.repository_owner, 1, 255)?;
        let repository_name = normalize_bounded("repository_name", &input.repository_name, 1, 255)?;
        let repository_url = normalize_repository_url(&input.repository_url)?;
        let base_path = normalize_repo_path("base_path", &input.base_path, true)?;
        let requested_ref = normalize_bounded("requested_ref", &input.requested_ref, 1, 255)?;
        let resolved_commit = normalize_oid("resolved_commit", &input.resolved_commit)?;
        let root_tree_oid = normalize_oid("root_tree_oid", &input.root_tree_oid)?;
        let warnings_json = normalize_json_array("warnings_json", &input.warnings_json)?;
        let errors_json = normalize_json_array("errors_json", &input.errors_json)?;
        let row = NewRegistryScanRow {
            id: &scan_id,
            publisher_uuid: &publisher_uuid,
            github_user_id: input.github_user_id,
            github_repository_id: input.github_repository_id,
            repository_owner: &repository_owner,
            repository_name: &repository_name,
            repository_url: &repository_url,
            base_path: &base_path,
            requested_ref: &requested_ref,
            resolved_commit: &resolved_commit,
            root_tree_oid: &root_tree_oid,
            warnings_json: &warnings_json,
            errors_json: &errors_json,
            expires_at: input.expires_at,
        };

        let mut connection = self.connection()?;
        connection.transaction::<RegistryScanWithEntries, DatabaseError, _>(|connection| {
            diesel::delete(
                registry_scans::table
                    .filter(registry_scans::expires_at.le(now))
                    .filter(registry_scans::published_at.is_null()),
            )
            .execute(connection)?;

            diesel::insert_into(registry_scans::table)
                .values(&row)
                .execute(connection)
                .map_err(|error| map_write_error(error, "registry_scan", &scan_id))?;

            for entry in &input.entries {
                insert_scan_entry(connection, &scan_id, entry)?;
            }

            load_scan(connection, &scan_id)
        })
    }

    pub fn get_registry_scan(
        &self,
        scan_id: Uuid,
        publisher_uuid: Uuid,
    ) -> Result<Option<RegistryScanWithEntries>> {
        let scan_id = scan_id.hyphenated().to_string();
        let publisher_uuid = publisher_uuid.hyphenated().to_string();
        let mut connection = self.connection()?;
        let exists = registry_scans::table
            .filter(registry_scans::id.eq(&scan_id))
            .filter(registry_scans::publisher_uuid.eq(publisher_uuid))
            .select(registry_scans::id)
            .first::<String>(&mut connection)
            .optional()?
            .is_some();
        if exists {
            load_scan(&mut connection, &scan_id).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn publish_registry_scan(
        &self,
        scan_id: Uuid,
        publisher_uuid: Uuid,
        github_user_id: i64,
        selected_entry_ids: &[Uuid],
        now: NaiveDateTime,
    ) -> Result<RegistryPublishResult> {
        if github_user_id <= 0 {
            return validation("github_user_id", "must be greater than zero");
        }
        if selected_entry_ids.is_empty() || selected_entry_ids.len() > MAX_SCAN_ENTRIES {
            return validation(
                "entry_ids",
                format!("must contain between 1 and {MAX_SCAN_ENTRIES} entries"),
            );
        }
        let entry_ids = selected_entry_ids
            .iter()
            .map(|id| id.hyphenated().to_string())
            .collect::<Vec<_>>();
        if entry_ids.iter().collect::<HashSet<_>>().len() != entry_ids.len() {
            return validation("entry_ids", "must not contain duplicates");
        }

        let scan_id = scan_id.hyphenated().to_string();
        let publisher_uuid = publisher_uuid.hyphenated().to_string();
        let mut connection = self.connection()?;
        connection.transaction::<RegistryPublishResult, DatabaseError, _>(|connection| {
            let scan = registry_scans::table
                .find(&scan_id)
                .select(RegistryScan::as_select())
                .first(connection)
                .optional()?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "registry_scan",
                    id: scan_id.clone(),
                })?;
            if scan.publisher_uuid != publisher_uuid {
                return Err(DatabaseError::NotFound {
                    entity: "registry_scan",
                    id: scan_id.clone(),
                });
            }
            if scan.github_user_id != github_user_id {
                return Err(DatabaseError::Conflict {
                    entity: "registry_scan",
                    key: "the linked GitHub account changed after the scan".to_owned(),
                });
            }
            if scan.expires_at <= now {
                return Err(DatabaseError::Conflict {
                    entity: "registry_scan",
                    key: "the scan expired; run it again".to_owned(),
                });
            }
            if scan.published_at.is_some() {
                return Err(DatabaseError::Conflict {
                    entity: "registry_scan",
                    key: "the scan was already published".to_owned(),
                });
            }

            let entries = registry_scan_entries::table
                .filter(registry_scan_entries::scan_id.eq(&scan_id))
                .filter(registry_scan_entries::id.eq_any(&entry_ids))
                .select(RegistryScanEntry::as_select())
                .load::<RegistryScanEntry>(connection)?;
            if entries.len() != entry_ids.len() {
                return validation("entry_ids", "every selected entry must belong to this scan");
            }
            for entry in &entries {
                if !matches!(entry.status.as_str(), "new_mod" | "new_version")
                    || !json_array_is_empty(&entry.errors_json)?
                {
                    return validation(
                        "entry_ids",
                        format!("entry {} is not publishable", entry.id),
                    );
                }
            }

            let repository = upsert_repository(connection, &scan, now)?;
            let mut published = Vec::with_capacity(entries.len());
            for entry in entries {
                let existing_mod = mods::table
                    .find(&entry.mod_id)
                    .select(Mod::as_select())
                    .first(connection)
                    .optional()?;

                match (entry.status.as_str(), existing_mod.as_ref()) {
                    ("new_mod", None) => {
                        let row = NewModRow {
                            id: &entry.mod_id,
                            publisher_uuid: &publisher_uuid,
                            repository_id: &repository.id,
                            source_base_path: &scan.base_path,
                            latest_version_id: None,
                            downloads: 0,
                        };
                        diesel::insert_into(mods::table)
                            .values(&row)
                            .execute(connection)
                            .map_err(|error| map_write_error(error, "mod", &entry.mod_id))?;
                    }
                    ("new_version", Some(existing)) => {
                        if existing.publisher_uuid != publisher_uuid
                            || existing.repository_id != repository.id
                        {
                            return Err(DatabaseError::Conflict {
                                entity: "mod",
                                key: format!(
                                    "{} belongs to another publisher or repository",
                                    entry.mod_id
                                ),
                            });
                        }
                    }
                    _ => {
                        return Err(DatabaseError::Conflict {
                            entity: "registry_scan",
                            key: format!(
                                "registry state changed for {}; run the scan again",
                                entry.mod_id
                            ),
                        });
                    }
                }

                if mod_versions::table
                    .filter(mod_versions::mod_id.eq(&entry.mod_id))
                    .filter(mod_versions::version.eq(&entry.version))
                    .select(mod_versions::id)
                    .first::<String>(connection)
                    .optional()?
                    .is_some()
                {
                    return Err(DatabaseError::Conflict {
                        entity: "registry_scan",
                        key: format!(
                            "{} {} was published after this scan; scan again",
                            entry.mod_id, entry.version
                        ),
                    });
                }

                let version_id = Uuid::new_v4().hyphenated().to_string();
                let version_row = NewModVersionRow {
                    id: &version_id,
                    mod_id: &entry.mod_id,
                    version: &entry.version,
                    title: &entry.title,
                    repository_path: &entry.repository_path,
                    source_commit: &scan.resolved_commit,
                    source_tree_oid: &entry.source_tree_oid,
                    manifest_path: &entry.manifest_path,
                    manifest_blob_oid: &entry.manifest_blob_oid,
                    manifest_sha256: &entry.manifest_sha256,
                    readme_path: entry.readme_path.as_deref(),
                    readme_blob_oid: entry.readme_blob_oid.as_deref(),
                    image_path: entry.image_path.as_deref(),
                    image_blob_oid: entry.image_blob_oid.as_deref(),
                    metadata_json: &entry.metadata_json,
                    published_by: &publisher_uuid,
                    published_github_user_id: github_user_id,
                };
                diesel::insert_into(mod_versions::table)
                    .values(&version_row)
                    .execute(connection)
                    .map_err(|error| {
                        map_write_error(
                            error,
                            "mod_version",
                            format!("{} {}", entry.mod_id, entry.version),
                        )
                    })?;
                insert_version_dependencies(connection, &version_id, &entry.dependencies_json)?;
                update_latest_version(connection, &entry.mod_id, &version_id, &entry.version)?;
                published.push(PublishedRegistryVersion {
                    mod_id: entry.mod_id,
                    version: entry.version,
                    version_id,
                });
            }

            let updated = diesel::update(
                registry_scans::table
                    .filter(registry_scans::id.eq(&scan_id))
                    .filter(registry_scans::published_at.is_null()),
            )
            .set(registry_scans::published_at.eq(Some(now)))
            .execute(connection)?;
            if updated != 1 {
                return Err(DatabaseError::Conflict {
                    entity: "registry_scan",
                    key: "the scan was published concurrently".to_owned(),
                });
            }

            Ok(RegistryPublishResult { scan_id, published })
        })
    }
}

fn insert_scan_entry(
    connection: &mut crate::db::DbConnection,
    scan_id: &str,
    input: &CreateRegistryScanEntry,
) -> Result<()> {
    let id = input.id.hyphenated().to_string();
    let mod_id = normalize_package_id("mod_id", &input.mod_id)?;
    let version = normalize_version(&input.version)?;
    let title = normalize_title(&input.title)?;
    let repository_path = normalize_repo_path("repository_path", &input.repository_path, true)?;
    let source_tree_oid = normalize_oid("source_tree_oid", &input.source_tree_oid)?;
    let manifest_path = normalize_repo_path("manifest_path", &input.manifest_path, false)?;
    let manifest_blob_oid = normalize_oid("manifest_blob_oid", &input.manifest_blob_oid)?;
    let manifest_sha256 = normalize_sha256("manifest_sha256", &input.manifest_sha256)?;
    let readme_path = normalize_optional_path("readme_path", input.readme_path.as_deref())?;
    let readme_blob_oid =
        normalize_optional_oid("readme_blob_oid", input.readme_blob_oid.as_deref())?;
    let image_path = normalize_optional_path("image_path", input.image_path.as_deref())?;
    let image_blob_oid = normalize_optional_oid("image_blob_oid", input.image_blob_oid.as_deref())?;
    validate_optional_pair("readme", readme_path.as_deref(), readme_blob_oid.as_deref())?;
    validate_optional_pair("image", image_path.as_deref(), image_blob_oid.as_deref())?;
    let status = normalize_status(&input.status)?;
    let metadata_json = normalize_json_object("metadata_json", &input.metadata_json)?;
    let dependencies_json = normalize_json_array("dependencies_json", &input.dependencies_json)?;
    let warnings_json = normalize_json_array("warnings_json", &input.warnings_json)?;
    let errors_json = normalize_json_array("errors_json", &input.errors_json)?;

    let row = NewRegistryScanEntryRow {
        id: &id,
        scan_id,
        mod_id: &mod_id,
        version: &version,
        title: &title,
        repository_path: &repository_path,
        source_tree_oid: &source_tree_oid,
        manifest_path: &manifest_path,
        manifest_blob_oid: &manifest_blob_oid,
        manifest_sha256: &manifest_sha256,
        readme_path: readme_path.as_deref(),
        readme_blob_oid: readme_blob_oid.as_deref(),
        image_path: image_path.as_deref(),
        image_blob_oid: image_blob_oid.as_deref(),
        status: &status,
        metadata_json: &metadata_json,
        dependencies_json: &dependencies_json,
        warnings_json: &warnings_json,
        errors_json: &errors_json,
    };
    diesel::insert_into(registry_scan_entries::table)
        .values(&row)
        .execute(connection)
        .map_err(|error| map_write_error(error, "registry_scan_entry", &id))?;
    Ok(())
}

fn load_scan(
    connection: &mut crate::db::DbConnection,
    scan_id: &str,
) -> Result<RegistryScanWithEntries> {
    let scan = registry_scans::table
        .find(scan_id)
        .select(RegistryScan::as_select())
        .first(connection)?;
    let entries = registry_scan_entries::table
        .filter(registry_scan_entries::scan_id.eq(scan_id))
        .order(registry_scan_entries::manifest_path.asc())
        .select(RegistryScanEntry::as_select())
        .load(connection)?;
    Ok(RegistryScanWithEntries { scan, entries })
}

fn upsert_repository(
    connection: &mut crate::db::DbConnection,
    scan: &RegistryScan,
    now: NaiveDateTime,
) -> Result<Repository> {
    let existing = repositories::table
        .filter(repositories::provider.eq(PROVIDER_GITHUB))
        .filter(repositories::provider_repository_id.eq(scan.github_repository_id))
        .select(Repository::as_select())
        .first(connection)
        .optional()?;
    let id = existing
        .as_ref()
        .map(|repository| repository.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().hyphenated().to_string());

    if existing.is_some() {
        diesel::update(repositories::table.find(&id))
            .set((
                repositories::owner.eq(&scan.repository_owner),
                repositories::name.eq(&scan.repository_name),
                repositories::canonical_url.eq(&scan.repository_url),
                repositories::updated_at.eq(now),
            ))
            .execute(connection)?;
    } else {
        let row = NewRepositoryRow {
            id: &id,
            provider: PROVIDER_GITHUB,
            provider_repository_id: scan.github_repository_id,
            owner: &scan.repository_owner,
            name: &scan.repository_name,
            canonical_url: &scan.repository_url,
        };
        diesel::insert_into(repositories::table)
            .values(&row)
            .execute(connection)
            .map_err(|error| {
                map_write_error(error, "repository", scan.github_repository_id.to_string())
            })?;
    }

    repositories::table
        .find(id)
        .select(Repository::as_select())
        .first(connection)
        .map_err(DatabaseError::from)
}

fn insert_version_dependencies(
    connection: &mut crate::db::DbConnection,
    version_id: &str,
    dependencies_json: &str,
) -> Result<()> {
    let dependencies =
        serde_json::from_str::<Vec<StoredDependency>>(dependencies_json).map_err(|error| {
            DatabaseError::Validation {
                field: "dependencies_json",
                message: error.to_string(),
            }
        })?;
    let mut seen = HashSet::new();
    let mut positions = [0_i32; 3];
    for dependency in dependencies {
        let kind_index = match dependency.kind.as_str() {
            "init" => 0,
            "run" => 1,
            "ownership" => 2,
            _ => return validation("dependency.kind", "must be init, run, or ownership"),
        };
        let target_id = normalize_package_id("dependency.target_id", &dependency.target_id)?;
        if !seen.insert((dependency.kind.clone(), target_id.clone())) {
            continue;
        }
        let row = NewModVersionDependencyRow {
            version_id,
            relation_kind: &dependency.kind,
            target_id: &target_id,
            position: positions[kind_index],
        };
        positions[kind_index] += 1;
        diesel::insert_into(mod_version_dependencies::table)
            .values(&row)
            .execute(connection)?;
    }
    Ok(())
}

fn update_latest_version(
    connection: &mut crate::db::DbConnection,
    mod_id: &str,
    new_version_id: &str,
    new_version: &str,
) -> Result<()> {
    let current = mods::table
        .find(mod_id)
        .select(mods::latest_version_id)
        .first::<Option<String>>(connection)?;
    let should_update = if let Some(current_id) = current {
        let current_version = mod_versions::table
            .find(current_id)
            .select(mod_versions::version)
            .first::<String>(connection)?;
        Version::parse(new_version).map_err(version_error)?
            > Version::parse(&current_version).map_err(version_error)?
    } else {
        true
    };
    if should_update {
        diesel::update(mods::table.find(mod_id))
            .set(mods::latest_version_id.eq(Some(new_version_id)))
            .execute(connection)?;
    }
    Ok(())
}

fn normalize_version(value: &str) -> Result<String> {
    let value = normalize_bounded("version", value, 1, 64)?;
    Version::parse(&value).map_err(version_error)?;
    Ok(value)
}

fn version_error(error: semver::Error) -> DatabaseError {
    DatabaseError::Validation {
        field: "version",
        message: error.to_string(),
    }
}

fn normalize_oid(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if (40..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        validation(field, "must be a 40-64 character hexadecimal Git object ID")
    }
}

fn normalize_sha256(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        validation(field, "must be a SHA-256 hexadecimal digest")
    }
}

fn normalize_optional_path(field: &'static str, value: Option<&str>) -> Result<Option<String>> {
    value
        .map(|value| normalize_repo_path(field, value, false))
        .transpose()
}

fn normalize_optional_oid(field: &'static str, value: Option<&str>) -> Result<Option<String>> {
    value.map(|value| normalize_oid(field, value)).transpose()
}

fn validate_optional_pair(
    field: &'static str,
    path: Option<&str>,
    oid: Option<&str>,
) -> Result<()> {
    if path.is_some() == oid.is_some() {
        Ok(())
    } else {
        validation(
            field,
            "path and blob OID must either both be set or both be absent",
        )
    }
}

fn normalize_status(value: &str) -> Result<String> {
    match value {
        "new_mod" | "new_version" | "unchanged" | "version_conflict" | "error" => {
            Ok(value.to_owned())
        }
        _ => validation("status", "contains an unsupported scan status"),
    }
}

fn normalize_json_array(field: &'static str, value: &str) -> Result<String> {
    normalize_json(field, value, Value::is_array)
}

fn normalize_json_object(field: &'static str, value: &str) -> Result<String> {
    normalize_json(field, value, Value::is_object)
}

fn normalize_json(
    field: &'static str,
    value: &str,
    expected_shape: fn(&Value) -> bool,
) -> Result<String> {
    if value.len() > 256 * 1024 {
        return validation(field, "must be at most 256 KiB");
    }
    let parsed =
        serde_json::from_str::<Value>(value).map_err(|error| DatabaseError::Validation {
            field,
            message: error.to_string(),
        })?;
    if !expected_shape(&parsed) {
        return validation(field, "has the wrong JSON shape");
    }
    serde_json::to_string(&parsed).map_err(|error| DatabaseError::Validation {
        field,
        message: error.to_string(),
    })
}

fn json_array_is_empty(value: &str) -> Result<bool> {
    let parsed =
        serde_json::from_str::<Value>(value).map_err(|error| DatabaseError::Validation {
            field: "errors_json",
            message: error.to_string(),
        })?;
    Ok(parsed.as_array().is_some_and(Vec::is_empty))
}

fn normalize_bounded(
    field: &'static str,
    value: &str,
    minimum: usize,
    maximum: usize,
) -> Result<String> {
    let value = value.trim();
    let length = value.chars().count();
    if !(minimum..=maximum).contains(&length) || value.chars().any(char::is_control) {
        validation(
            field,
            format!("must contain between {minimum} and {maximum} non-control characters"),
        )
    } else {
        Ok(value.to_owned())
    }
}

fn validation<T>(field: &'static str, message: impl Into<String>) -> Result<T> {
    Err(DatabaseError::Validation {
        field,
        message: message.into(),
    })
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    use super::*;
    use crate::models::CreateAccount;

    fn database_with_publisher() -> (Database, Uuid) {
        let directory = tempdir().unwrap().keep();
        let database = Database::connect(directory.join("registry.sqlite").to_string_lossy())
            .expect("database should open");
        let publisher = Uuid::new_v4();
        database
            .create_account(CreateAccount {
                uuid: publisher,
                nickname: "publisher".to_owned(),
                email: "publisher@example.com".to_owned(),
                password_hash: None,
            })
            .unwrap();
        database
            .link_github_account(
                publisher,
                42,
                "publisher",
                "https://avatars.githubusercontent.com/u/42?v=4",
                Utc::now().naive_utc(),
            )
            .unwrap();
        (database, publisher)
    }

    fn scan(
        publisher: Uuid,
        scan_id: Uuid,
        entry_id: Uuid,
        status: &str,
        version: &str,
        tree_oid: char,
    ) -> CreateRegistryScan {
        let now = Utc::now().naive_utc();
        CreateRegistryScan {
            id: scan_id,
            publisher_uuid: publisher,
            github_user_id: 42,
            github_repository_id: 1001,
            repository_owner: "publisher".to_owned(),
            repository_name: "mods".to_owned(),
            repository_url: "https://github.com/publisher/mods".to_owned(),
            base_path: "mods".to_owned(),
            requested_ref: "main".to_owned(),
            resolved_commit: "a".repeat(40),
            root_tree_oid: "b".repeat(40),
            warnings_json: "[]".to_owned(),
            errors_json: "[]".to_owned(),
            expires_at: now + Duration::minutes(20),
            entries: vec![CreateRegistryScanEntry {
                id: entry_id,
                mod_id: "example-mod".to_owned(),
                version: version.to_owned(),
                title: "Example Mod".to_owned(),
                repository_path: "mods/example-mod".to_owned(),
                source_tree_oid: tree_oid.to_string().repeat(40),
                manifest_path: "mods/example-mod/Cargo.toml".to_owned(),
                manifest_blob_oid: "d".repeat(40),
                manifest_sha256: "e".repeat(64),
                readme_path: Some("mods/example-mod/README.md".to_owned()),
                readme_blob_oid: Some("f".repeat(40)),
                image_path: None,
                image_blob_oid: None,
                status: status.to_owned(),
                metadata_json: "{}".to_owned(),
                dependencies_json: r#"[{"kind":"run","targetId":"support-api","available":false}]"#
                    .to_owned(),
                warnings_json: "[]".to_owned(),
                errors_json: "[]".to_owned(),
            }],
        }
    }

    #[test]
    fn publish_uses_authoritative_scan_data_and_is_single_use() {
        let (database, publisher) = database_with_publisher();
        let now = Utc::now().naive_utc();
        let scan_id = Uuid::new_v4();
        let entry_id = Uuid::new_v4();
        database
            .create_registry_scan(
                scan(publisher, scan_id, entry_id, "new_mod", "1.0.0", 'c'),
                now,
            )
            .unwrap();

        let published = database
            .publish_registry_scan(scan_id, publisher, 42, &[entry_id], now)
            .unwrap();
        assert_eq!(published.published.len(), 1);
        let state = database
            .get_registry_mod_state("example-mod")
            .unwrap()
            .unwrap();
        assert_eq!(state.versions[0].version, "1.0.0");
        assert_eq!(state.versions[0].source_tree_oid, "c".repeat(40));
        assert_eq!(state.versions[0].source_commit, "a".repeat(40));

        let repeated = database
            .publish_registry_scan(scan_id, publisher, 42, &[entry_id], now)
            .unwrap_err();
        assert!(matches!(repeated, DatabaseError::Conflict { .. }));
    }

    #[test]
    fn publish_revalidates_version_uniqueness_after_the_scan() {
        let (database, publisher) = database_with_publisher();
        let now = Utc::now().naive_utc();
        let first_scan = Uuid::new_v4();
        let first_entry = Uuid::new_v4();
        database
            .create_registry_scan(
                scan(publisher, first_scan, first_entry, "new_mod", "1.0.0", 'c'),
                now,
            )
            .unwrap();

        let stale_scan = Uuid::new_v4();
        let stale_entry = Uuid::new_v4();
        database
            .create_registry_scan(
                scan(
                    publisher,
                    stale_scan,
                    stale_entry,
                    "new_version",
                    "1.0.0",
                    '9',
                ),
                now,
            )
            .unwrap();

        database
            .publish_registry_scan(first_scan, publisher, 42, &[first_entry], now)
            .unwrap();
        let conflict = database
            .publish_registry_scan(stale_scan, publisher, 42, &[stale_entry], now)
            .unwrap_err();
        assert!(matches!(conflict, DatabaseError::Conflict { .. }));
    }
}
