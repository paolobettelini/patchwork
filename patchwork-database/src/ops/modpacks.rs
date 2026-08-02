use std::collections::HashSet;

use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::{Database, DbConnection};
use crate::error::{DatabaseError, Result, map_write_error};
use crate::models::{
    DependencyInput, DependencyKind, Modpack, ModpackDependency, ModpackWithDependencies,
    NewModpackDependencyRow, NewModpackRow, Pagination, PublishModpack,
};
use crate::schema::{modpack_dependencies, modpacks, mods};
use crate::validation::{
    normalize_description, normalize_https_url, normalize_package_id, normalize_repo_path,
    normalize_repository_url, normalize_source_ref, normalize_title,
};

impl Database {
    pub fn publish_modpack(&self, input: PublishModpack) -> Result<ModpackWithDependencies> {
        let id = normalize_package_id("modpack_id", &input.id)?;
        let title = normalize_title(&input.title)?;
        let description = normalize_description(&input.description)?;
        let publisher_uuid = input.publisher_uuid.hyphenated().to_string();
        let repository_url = normalize_repository_url(&input.repository_url)?;
        let manifest_path = normalize_repo_path("manifest_path", &input.manifest_path, false)?;
        let source_ref = normalize_source_ref(&input.source_ref)?;
        let logo_url = input
            .logo_url
            .as_deref()
            .map(|url| normalize_https_url("logo_url", url))
            .transpose()?;
        let dependencies = normalize_dependencies(&id, &input.dependencies)?;

        let row = NewModpackRow {
            id: &id,
            title: &title,
            description: &description,
            downloads: 0,
            publisher_uuid: &publisher_uuid,
            repository_url: &repository_url,
            manifest_path: &manifest_path,
            source_ref: &source_ref,
            logo_url: logo_url.as_deref(),
        };

        let mut connection = self.connection()?;
        validate_dependency_targets(&mut connection, &dependencies)?;

        connection
            .transaction::<_, diesel::result::Error, _>(|connection| {
                diesel::insert_into(modpacks::table)
                    .values(&row)
                    .execute(connection)?;

                if !dependencies.is_empty() {
                    let rows = dependency_rows(&id, &dependencies);
                    diesel::insert_into(modpack_dependencies::table)
                        .values(&rows)
                        .execute(connection)?;
                }
                Ok(())
            })
            .map_err(|error| map_write_error(error, "modpack", &id))?;

        self.get_modpack(&id)?.ok_or(DatabaseError::NotFound {
            entity: "modpack",
            id,
        })
    }

    pub fn get_modpack(&self, id: &str) -> Result<Option<ModpackWithDependencies>> {
        let id = normalize_package_id("modpack_id", id)?;
        let mut connection = self.connection()?;
        let Some(modpack) = modpacks::table
            .find(&id)
            .select(Modpack::as_select())
            .first(&mut connection)
            .optional()?
        else {
            return Ok(None);
        };

        let dependencies = ModpackDependency::belonging_to(&modpack)
            .order(modpack_dependencies::position.asc())
            .select(ModpackDependency::as_select())
            .load(&mut connection)?;

        Ok(Some(ModpackWithDependencies {
            modpack,
            dependencies,
        }))
    }

    pub fn replace_modpack_dependencies(
        &self,
        modpack_id: &str,
        dependencies: &[DependencyInput],
    ) -> Result<Vec<ModpackDependency>> {
        let modpack_id = normalize_package_id("modpack_id", modpack_id)?;
        let dependencies = normalize_dependencies(&modpack_id, dependencies)?;
        let mut connection = self.connection()?;

        let exists = modpacks::table
            .find(&modpack_id)
            .select(modpacks::id)
            .first::<String>(&mut connection)
            .optional()?
            .is_some();
        if !exists {
            return Err(DatabaseError::NotFound {
                entity: "modpack",
                id: modpack_id,
            });
        }

        validate_dependency_targets(&mut connection, &dependencies)?;
        connection.transaction::<_, diesel::result::Error, _>(|connection| {
            diesel::delete(
                modpack_dependencies::table
                    .filter(modpack_dependencies::modpack_id.eq(&modpack_id)),
            )
            .execute(connection)?;

            if !dependencies.is_empty() {
                let rows = dependency_rows(&modpack_id, &dependencies);
                diesel::insert_into(modpack_dependencies::table)
                    .values(&rows)
                    .execute(connection)?;
            }
            Ok(())
        })?;

        Ok(modpack_dependencies::table
            .filter(modpack_dependencies::modpack_id.eq(&modpack_id))
            .order(modpack_dependencies::position.asc())
            .select(ModpackDependency::as_select())
            .load(&mut connection)?)
    }

    pub fn search_modpacks(&self, query: &str, pagination: Pagination) -> Result<Vec<Modpack>> {
        let query = query.trim();
        if query.chars().count() > 200 {
            return Err(DatabaseError::Validation {
                field: "query",
                message: "must contain at most 200 characters".to_owned(),
            });
        }

        let mut connection = self.connection()?;
        if query.is_empty() {
            return Ok(modpacks::table
                .order(modpacks::published_at.desc())
                .limit(pagination.limit)
                .offset(pagination.offset)
                .select(Modpack::as_select())
                .load(&mut connection)?);
        }

        let pattern = format!("%{query}%");
        Ok(modpacks::table
            .filter(
                modpacks::id
                    .like(&pattern)
                    .or(modpacks::title.like(&pattern))
                    .or(modpacks::description.like(&pattern)),
            )
            .order(modpacks::published_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .select(Modpack::as_select())
            .load(&mut connection)?)
    }

    pub fn find_modpacks_referencing(
        &self,
        kind: DependencyKind,
        target_id: &str,
        pagination: Pagination,
    ) -> Result<Vec<Modpack>> {
        let target_id = normalize_package_id("target_id", target_id)?;
        let mut connection = self.connection()?;

        Ok(modpacks::table
            .inner_join(modpack_dependencies::table)
            .filter(modpack_dependencies::relation_kind.eq(kind.as_db_str()))
            .filter(modpack_dependencies::target_id.eq(target_id))
            .order(modpacks::published_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .select(Modpack::as_select())
            .load(&mut connection)?)
    }

    pub fn list_modpacks_by_publisher(
        &self,
        publisher: Uuid,
        pagination: Pagination,
    ) -> Result<Vec<Modpack>> {
        let publisher = publisher.hyphenated().to_string();
        let mut connection = self.connection()?;
        Ok(modpacks::table
            .filter(modpacks::publisher_uuid.eq(publisher))
            .order(modpacks::published_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .select(Modpack::as_select())
            .load(&mut connection)?)
    }
}

fn normalize_dependencies(
    owner_modpack_id: &str,
    dependencies: &[DependencyInput],
) -> Result<Vec<DependencyInput>> {
    if dependencies.len() > 10_000 {
        return Err(DatabaseError::Validation {
            field: "dependencies",
            message: "must contain at most 10000 entries".to_owned(),
        });
    }

    let mut seen = HashSet::with_capacity(dependencies.len());
    let mut normalized = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        let target_id = normalize_package_id("target_id", &dependency.target_id)?;
        if dependency.kind == DependencyKind::Modpack && target_id == owner_modpack_id {
            return Err(DatabaseError::Validation {
                field: "dependencies",
                message: "a modpack cannot depend on itself".to_owned(),
            });
        }
        let key = (dependency.kind, target_id.clone());
        if !seen.insert(key) {
            return Err(DatabaseError::Validation {
                field: "dependencies",
                message: format!(
                    "duplicate {} dependency `{target_id}`",
                    dependency.kind.as_db_str()
                ),
            });
        }
        normalized.push(DependencyInput {
            kind: dependency.kind,
            target_id,
        });
    }
    Ok(normalized)
}

fn dependency_rows<'a>(
    modpack_id: &'a str,
    dependencies: &'a [DependencyInput],
) -> Vec<NewModpackDependencyRow<'a>> {
    dependencies
        .iter()
        .enumerate()
        .map(|(position, dependency)| NewModpackDependencyRow {
            modpack_id,
            relation_kind: dependency.kind.as_db_str(),
            target_id: &dependency.target_id,
            position: i32::try_from(position).expect("dependency limit fits in i32"),
        })
        .collect()
}

fn validate_dependency_targets(
    connection: &mut DbConnection,
    dependencies: &[DependencyInput],
) -> Result<()> {
    for dependency in dependencies {
        let exists = match dependency.kind {
            DependencyKind::Mod => mods::table
                .find(&dependency.target_id)
                .select(mods::id)
                .first::<String>(connection)
                .optional()?
                .is_some(),
            DependencyKind::Modpack => modpacks::table
                .find(&dependency.target_id)
                .select(modpacks::id)
                .first::<String>(connection)
                .optional()?
                .is_some(),
            // Ignore entries are deliberately allowed to reference something absent.
            // This supports manifests that suppress optional/transitive packages.
            DependencyKind::Ignore => true,
        };

        if !exists {
            return Err(DatabaseError::NotFound {
                entity: dependency.kind.as_db_str(),
                id: dependency.target_id.clone(),
            });
        }
    }
    Ok(())
}
