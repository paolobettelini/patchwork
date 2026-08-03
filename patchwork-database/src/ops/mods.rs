use diesel::OptionalExtension;
use diesel::prelude::*;
use uuid::Uuid;

use crate::db::Database;
use crate::error::{DatabaseError, Result};
use crate::models::{
    Mod, ModVersion, ModVersionDependency, Pagination, PublishedMod, RegistryModState, Repository,
};
use crate::schema::{mod_version_dependencies, mod_versions, mods, repositories};
use crate::validation::normalize_package_id;

impl Database {
    pub fn increment_mod_downloads(&self, id: &str) -> Result<i64> {
        let id = normalize_package_id("mod_id", id)?;
        let mut connection = self.connection()?;
        let changed = diesel::update(mods::table.find(&id))
            .set(mods::downloads.eq(mods::downloads + 1))
            .execute(&mut connection)?;
        if changed == 0 {
            return Err(DatabaseError::NotFound { entity: "mod", id });
        }
        mods::table
            .find(&id)
            .select(mods::downloads)
            .first(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn get_mod(&self, id: &str) -> Result<Option<Mod>> {
        let id = normalize_package_id("mod_id", id)?;
        let mut connection = self.connection()?;
        Ok(mods::table
            .find(id)
            .select(Mod::as_select())
            .first(&mut connection)
            .optional()?)
    }

    pub fn get_registry_mod_state(&self, id: &str) -> Result<Option<RegistryModState>> {
        let id = normalize_package_id("mod_id", id)?;
        let mut connection = self.connection()?;
        let Some(mod_record) = mods::table
            .find(&id)
            .select(Mod::as_select())
            .first(&mut connection)
            .optional()?
        else {
            return Ok(None);
        };
        let repository = repositories::table
            .find(&mod_record.repository_id)
            .select(Repository::as_select())
            .first(&mut connection)?;
        let versions = mod_versions::table
            .filter(mod_versions::mod_id.eq(&id))
            .order(mod_versions::published_at.desc())
            .select(ModVersion::as_select())
            .load(&mut connection)?;
        Ok(Some(RegistryModState {
            mod_record,
            repository,
            versions,
        }))
    }

    pub fn list_mod_version_dependencies(
        &self,
        version_id: &str,
    ) -> Result<Vec<ModVersionDependency>> {
        let mut connection = self.connection()?;
        mod_version_dependencies::table
            .filter(mod_version_dependencies::version_id.eq(version_id))
            .order(mod_version_dependencies::position.asc())
            .select(ModVersionDependency::as_select())
            .load(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn list_mods_by_publisher(
        &self,
        publisher: Uuid,
        pagination: Pagination,
    ) -> Result<Vec<PublishedMod>> {
        let publisher = publisher.hyphenated().to_string();
        let mut connection = self.connection()?;
        mods::table
            .inner_join(repositories::table)
            .inner_join(
                mod_versions::table.on(mods::latest_version_id.eq(mod_versions::id.nullable())),
            )
            .select((
                mods::id,
                mod_versions::title,
                mod_versions::version,
                mods::downloads,
                mods::publisher_uuid,
                repositories::canonical_url,
                mod_versions::repository_path,
                mod_versions::source_commit,
                mod_versions::source_tree_oid,
                mod_versions::manifest_sha256,
                mod_versions::readme_path,
                mod_versions::image_path,
            ))
            .filter(mods::publisher_uuid.eq(publisher))
            .order(mods::created_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .load(&mut connection)
            .map_err(DatabaseError::from)
    }

    pub fn search_mods(&self, query: &str, pagination: Pagination) -> Result<Vec<PublishedMod>> {
        let query = query.trim();
        if query.chars().count() > 200 {
            return Err(DatabaseError::Validation {
                field: "query",
                message: "must contain at most 200 characters".to_owned(),
            });
        }

        let mut connection = self.connection()?;
        let mut statement = mods::table
            .inner_join(repositories::table)
            .inner_join(
                mod_versions::table.on(mods::latest_version_id.eq(mod_versions::id.nullable())),
            )
            .select((
                mods::id,
                mod_versions::title,
                mod_versions::version,
                mods::downloads,
                mods::publisher_uuid,
                repositories::canonical_url,
                mod_versions::repository_path,
                mod_versions::source_commit,
                mod_versions::source_tree_oid,
                mod_versions::manifest_sha256,
                mod_versions::readme_path,
                mod_versions::image_path,
            ))
            .into_boxed();
        if !query.is_empty() {
            let pattern = format!("%{query}%");
            statement = statement.filter(
                mods::id
                    .like(pattern.clone())
                    .or(mod_versions::title.like(pattern.clone()))
                    .or(repositories::owner.like(pattern.clone()))
                    .or(repositories::name.like(pattern)),
            );
        }
        statement
            .order(mods::created_at.desc())
            .limit(pagination.limit)
            .offset(pagination.offset)
            .load(&mut connection)
            .map_err(DatabaseError::from)
    }
}
