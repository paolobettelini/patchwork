use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result};
use crate::models::{
    Modpack, ModpackVersion, ModpackVersionDependency, Pagination, PublishedModpack,
    RegistryModpackState, Repository,
};
use crate::schema::{modpack_version_dependencies, modpack_versions, modpacks, repositories};
use crate::validation::normalize_package_id;

impl Database {
    pub fn increment_modpack_downloads(&self, id: &str) -> Result<i64> {
        let id = normalize_package_id("modpack_id", id)?;
        let mut connection = self.connection()?;
        let changed = diesel::update(modpacks::table.find(&id))
            .set(modpacks::downloads.eq(modpacks::downloads + 1))
            .execute(&mut connection)?;
        if changed == 0 {
            return Err(DatabaseError::NotFound {
                entity: "modpack",
                id,
            });
        }
        modpacks::table
            .find(&id)
            .select(modpacks::downloads)
            .first(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn get_modpack(&self, id: &str) -> Result<Option<Modpack>> {
        let id = normalize_package_id("modpack_id", id)?;
        let mut connection = self.connection()?;
        Ok(modpacks::table
            .find(id)
            .select(Modpack::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn get_registry_modpack_state(&self, id: &str) -> Result<Option<RegistryModpackState>> {
        let id = normalize_package_id("modpack_id", id)?;
        let mut connection = self.connection()?;
        let Some(modpack_record) = modpacks::table
            .find(&id)
            .select(Modpack::as_select())
            .first(&mut connection)
            .optional()?
        else {
            return Ok(None);
        };
        let repository = repositories::table
            .find(&modpack_record.repository_id)
            .select(Repository::as_select())
            .first(&mut connection)?;
        let versions = modpack_versions::table
            .filter(modpack_versions::modpack_id.eq(&id))
            .order(modpack_versions::published_at.desc())
            .select(ModpackVersion::as_select())
            .load(&mut connection)?;
        Ok(Some(RegistryModpackState {
            modpack_record,
            repository,
            versions,
        }))
    }

    pub fn list_modpack_version_dependencies(
        &self,
        version_id: &str,
    ) -> Result<Vec<ModpackVersionDependency>> {
        let mut connection = self.connection()?;
        modpack_version_dependencies::table
            .filter(modpack_version_dependencies::version_id.eq(version_id))
            .order(modpack_version_dependencies::position.asc())
            .select(ModpackVersionDependency::as_select())
            .load(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn list_modpacks_by_publisher(
        &self,
        publisher: Uuid,
        pagination: Pagination,
    ) -> Result<Vec<PublishedModpack>> {
        let publisher = publisher.hyphenated().to_string();
        let mut connection = self.connection()?;
        modpacks::table
            .inner_join(repositories::table)
            .inner_join(
                modpack_versions::table
                    .on(modpacks::latest_version_id.eq(modpack_versions::id.nullable())),
            )
            .select((
                modpacks::id,
                modpack_versions::title,
                modpack_versions::description,
                modpack_versions::version,
                modpacks::downloads,
                modpacks::publisher_uuid,
                repositories::canonical_url,
                modpack_versions::repository_path,
                modpack_versions::source_commit,
                modpack_versions::source_tree_oid,
                modpack_versions::manifest_sha256,
                modpack_versions::readme_path,
                modpack_versions::image_path,
            ))
            .filter(modpacks::publisher_uuid.eq(publisher))
            .order(modpacks::created_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .load(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn search_modpacks(
        &self,
        query: &str,
        pagination: Pagination,
    ) -> Result<Vec<PublishedModpack>> {
        let query = query.trim();
        if query.chars().count() > 200 {
            return Err(DatabaseError::Validation {
                field: "query",
                message: "must contain at most 200 characters".to_owned(),
            });
        }

        let mut connection = self.connection()?;
        let mut statement = modpacks::table
            .inner_join(repositories::table)
            .inner_join(
                modpack_versions::table
                    .on(modpacks::latest_version_id.eq(modpack_versions::id.nullable())),
            )
            .select((
                modpacks::id,
                modpack_versions::title,
                modpack_versions::description,
                modpack_versions::version,
                modpacks::downloads,
                modpacks::publisher_uuid,
                repositories::canonical_url,
                modpack_versions::repository_path,
                modpack_versions::source_commit,
                modpack_versions::source_tree_oid,
                modpack_versions::manifest_sha256,
                modpack_versions::readme_path,
                modpack_versions::image_path,
            ))
            .into_boxed();
        if !query.is_empty() {
            let pattern = format!("%{query}%");
            statement = statement.filter(
                modpacks::id
                    .like(pattern.clone())
                    .or(modpack_versions::title.like(pattern.clone()))
                    .or(modpack_versions::description.like(pattern.clone()))
                    .or(repositories::owner.like(pattern.clone()))
                    .or(repositories::name.like(pattern)),
            );
        }
        statement
            .order(modpacks::created_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .load(&mut connection)
            .map_err(DatabaseError::from)
    }
}
