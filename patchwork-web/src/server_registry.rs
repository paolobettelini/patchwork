use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::path::{Component, Path};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use actix_web::{HttpRequest, HttpResponse, Result, error, web};
use chrono::{Duration, SecondsFormat, Utc};
use flate2::{Compression, write::GzEncoder};
use futures_util::{StreamExt, stream};
use patchwork::{
    RegistryDependencyTargetKind, RegistryModpackManifest, RegistryWorkspaceManifest,
    parse_registry_dependency, parse_registry_mod_manifest, parse_registry_modpack_manifest,
};
use patchwork_database::{
    Account, CreateRegistryScan, CreateRegistryScanEntry, Database, DatabaseError, Pagination,
    PublishedMod, PublishedModpack, RegistryModState, RegistryModpackState, RegistryPublishResult,
    RegistryScanWithEntries,
};
use patchwork_registry_types::{
    RegistryBrowseProject, RegistryBrowseResponse, RegistryBrowseSource, RegistryDependency,
    RegistryDependencyKind, RegistryProjectDetails, RegistryProjectKind, RegistryPublishRequest,
    RegistryPublishResponse, RegistryPublishedVersion, RegistryRepository, RegistryScan,
    RegistryScanEntry, RegistryScanJobStarted, RegistryScanPhase, RegistryScanProgress,
    RegistryScanRequest, RegistryScanStatus, is_generated_mod_id, registry_search_rank,
};
use semver::Version;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::github::{AuthorizedGithubRepository, GithubClient, GithubTree, GithubTreeEntry};
use crate::server_auth;

const SCAN_TTL_MINUTES: i64 = 20;
const MAX_TREE_ENTRIES: usize = 100_000;
const MAX_DIRECTORIES: usize = 10_000;
const MAX_DIRECTORY_DEPTH: usize = 64;
const MAX_MANIFESTS: usize = 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const GITHUB_BLOB_CONCURRENCY: usize = 12;
const SCAN_JOB_RETENTION_MINUTES: u64 = 30;
const MAX_PUBLISHED_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PUBLISHED_README_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PUBLISHED_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PUBLISHED_SOURCE_FILES: usize = 20_000;
const MAX_PUBLISHED_SOURCE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PUBLISHED_SOURCE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct RegistryState {
    database: Database,
    github: GithubClient,
    base_path: String,
    jobs: Arc<Mutex<HashMap<Uuid, StoredScanJob>>>,
}

impl RegistryState {
    pub(crate) fn new(database: Database, github: GithubClient, base_path: String) -> Self {
        Self {
            database,
            github,
            base_path,
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn route(&self, route: &str) -> String {
        crate::config::prefixed_route(&self.base_path, route)
    }

    fn start_job(&self, publisher_uuid: Uuid) -> (ScanReporter, RegistryScanJobStarted) {
        let job_id = Uuid::new_v4();
        let progress = RegistryScanProgress {
            job_id: job_id.hyphenated().to_string(),
            phase: RegistryScanPhase::Queued,
            completed: 0,
            total: None,
            entries: Vec::new(),
            scan: None,
            error: None,
        };
        let mut jobs = self.jobs.lock().unwrap_or_else(|lock| lock.into_inner());
        jobs.retain(|_, job| {
            job.updated_at.elapsed() < StdDuration::from_secs(SCAN_JOB_RETENTION_MINUTES * 60)
        });
        jobs.insert(
            job_id,
            StoredScanJob {
                publisher_uuid,
                progress,
                updated_at: Instant::now(),
            },
        );
        (
            ScanReporter {
                job_id,
                jobs: Arc::clone(&self.jobs),
            },
            RegistryScanJobStarted {
                job_id: job_id.hyphenated().to_string(),
            },
        )
    }

    fn job_progress(&self, job_id: Uuid, publisher_uuid: Uuid) -> Option<RegistryScanProgress> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|lock| lock.into_inner());
        if jobs.get(&job_id).is_some_and(|job| {
            job.updated_at.elapsed() >= StdDuration::from_secs(SCAN_JOB_RETENTION_MINUTES * 60)
        }) {
            jobs.remove(&job_id);
        }
        jobs.get(&job_id)
            .filter(|job| job.publisher_uuid == publisher_uuid)
            .map(|job| job.progress.clone())
    }
}

struct StoredScanJob {
    publisher_uuid: Uuid,
    progress: RegistryScanProgress,
    updated_at: Instant,
}

#[derive(Clone)]
struct ScanReporter {
    job_id: Uuid,
    jobs: Arc<Mutex<HashMap<Uuid, StoredScanJob>>>,
}

impl ScanReporter {
    fn phase(&self, phase: RegistryScanPhase, completed: usize, total: Option<usize>) {
        self.with_progress(|progress| {
            progress.phase = phase;
            progress.completed = count_u32(completed);
            progress.total = total.map(count_u32);
        });
    }

    fn entry(&self, entry: RegistryScanEntry, completed: usize, total: usize) {
        self.with_progress(|progress| {
            progress.phase = RegistryScanPhase::ValidatingProjects;
            progress.completed = count_u32(completed);
            progress.total = Some(count_u32(total));
            progress.entries.push(entry);
        });
    }

    fn complete(&self, scan: RegistryScan) {
        self.with_progress(|progress| {
            progress.phase = RegistryScanPhase::Complete;
            progress.completed = count_u32(scan.entries.len());
            progress.total = Some(count_u32(scan.entries.len()));
            progress.entries = scan.entries.clone();
            progress.scan = Some(scan);
            progress.error = None;
        });
    }

    fn fail(&self, message: String) {
        self.with_progress(|progress| {
            progress.phase = RegistryScanPhase::Failed;
            progress.error = Some(message);
        });
    }

    fn with_progress(&self, update: impl FnOnce(&mut RegistryScanProgress)) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|lock| lock.into_inner());
        if let Some(job) = jobs.get_mut(&self.job_id) {
            update(&mut job.progress);
            job.updated_at = Instant::now();
        }
    }
}

fn count_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(crate) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/registry/search", web::get().to(search_registry))
        .route(
            "/registry/projects/{project_kind}/{project_id}",
            web::get().to(get_project_details),
        )
        .route(
            "/registry/projects/{project_kind}/{project_id}/source",
            web::get().to(get_published_source),
        )
        .route(
            "/registry/projects/{project_kind}/{project_id}/{artifact}",
            web::get().to(get_published_artifact),
        )
        .route("/registry/scans", web::post().to(create_scan))
        .route("/registry/scan-jobs", web::post().to(start_scan_job))
        .route("/registry/scan-jobs/{job_id}", web::get().to(get_scan_job))
        .route("/registry/scans/{scan_id}", web::get().to(get_scan))
        .route(
            "/registry/scans/{scan_id}/publish",
            web::post().to(publish_scan),
        )
        .route("/registry/mods/{mod_id}/rescan", web::post().to(rescan_mod))
        .route(
            "/registry/mods/{mod_id}/rescan-job",
            web::post().to(start_rescan_job),
        )
        .route(
            "/registry/projects/{project_kind}/{project_id}/rescan",
            web::post().to(rescan_project),
        )
        .route(
            "/registry/projects/{project_kind}/{project_id}/rescan-job",
            web::post().to(start_project_rescan_job),
        );
}

#[derive(Deserialize)]
struct RegistrySearchQuery {
    #[serde(default, alias = "query")]
    q: String,
    mods: Option<bool>,
    modpacks: Option<bool>,
}

async fn search_registry(
    state: web::Data<RegistryState>,
    query: web::Query<RegistrySearchQuery>,
) -> Result<HttpResponse> {
    let pagination = Pagination::new(100, 0).map_err(database_http_error)?;
    let mut projects = Vec::new();
    if query.mods.unwrap_or(true) {
        projects.extend(
            state
                .database
                .search_mods(&query.q, pagination)
                .map_err(database_http_error)?
                .into_iter()
                .map(|project| browse_mod_dto(project, &state)),
        );
    }
    if query.modpacks.unwrap_or(true) {
        projects.extend(
            state
                .database
                .search_modpacks(&query.q, pagination)
                .map_err(database_http_error)?
                .into_iter()
                .map(|project| browse_modpack_dto(project, &state)),
        );
    }
    projects.retain(|project| {
        project.project_kind != RegistryProjectKind::Mod
            || !is_generated_mod_id(&project.project_id)
    });
    projects.sort_by(|left, right| {
        registry_search_rank(left, &query.q)
            .cmp(&registry_search_rank(right, &query.q))
            .then_with(|| right.downloads.cmp(&left.downloads))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            .then_with(|| left.project_id.cmp(&right.project_id))
    });
    Ok(HttpResponse::Ok().json(RegistryBrowseResponse {
        projects,
        warnings: Vec::new(),
    }))
}

fn browse_mod_dto(project: PublishedMod, state: &RegistryState) -> RegistryBrowseProject {
    browse_project_dto(
        RegistryProjectKind::Mod,
        project.id,
        project.title,
        String::new(),
        project.latest_version,
        project.downloads,
        project.repository_url,
        project.repository_path,
        project.source_commit,
        project.source_tree_oid,
        project.manifest_sha256,
        project.readme_path.is_some(),
        project.image_path.is_some(),
        state,
    )
}

fn browse_modpack_dto(project: PublishedModpack, state: &RegistryState) -> RegistryBrowseProject {
    browse_project_dto(
        RegistryProjectKind::Modpack,
        project.id,
        project.title,
        project.description,
        project.latest_version,
        project.downloads,
        project.repository_url,
        project.repository_path,
        project.source_commit,
        project.source_tree_oid,
        project.manifest_sha256,
        project.readme_path.is_some(),
        project.image_path.is_some(),
        state,
    )
}

#[allow(clippy::too_many_arguments)]
fn browse_project_dto(
    project_kind: RegistryProjectKind,
    project_id: String,
    title: String,
    description: String,
    version: String,
    downloads: i64,
    repository_url: String,
    repository_path: String,
    source_commit: String,
    source_tree_oid: String,
    manifest_sha256: String,
    has_readme: bool,
    has_image: bool,
    state: &RegistryState,
) -> RegistryBrowseProject {
    let route = state.route(&format!(
        "/registry/projects/{}/{project_id}",
        project_kind.route_segment()
    ));
    let source_label = Url::parse(&repository_url)
        .ok()
        .and_then(|url| {
            let mut segments = url.path_segments()?;
            Some(format!("{}/{}", segments.next()?, segments.next()?))
        })
        .unwrap_or_else(|| "Patchwork registry".to_owned());
    RegistryBrowseProject {
        project_kind,
        project_id,
        title,
        description,
        version,
        downloads,
        source: RegistryBrowseSource::Remote,
        source_label,
        repository_url: Some(repository_url),
        repository_path: Some(repository_path),
        source_commit: Some(source_commit),
        source_tree_oid: Some(source_tree_oid),
        manifest_sha256: Some(manifest_sha256),
        manifest_url: Some(format!("{route}/manifest")),
        readme_url: has_readme.then(|| format!("{route}/readme")),
        image_url: has_image.then(|| format!("{route}/image")),
        local_manifest_path: None,
    }
}

struct PublishedSource {
    repository_id: i64,
    owner: String,
    repository: String,
    tree_oid: String,
}

async fn get_published_source(
    state: web::Data<RegistryState>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    let (project_kind, project_id) = path.into_inner();
    let source = published_source(&state.database, &project_kind, &project_id)?
        .ok_or_else(|| error::ErrorNotFound("published source was not found"))?;
    let (tree, access_token) = state
        .github
        .published_tree(
            &source.owner,
            &source.repository,
            source.repository_id,
            &source.tree_oid,
        )
        .await
        .map_err(error::ErrorBadGateway)?;
    if tree.sha != source.tree_oid {
        return Err(error::ErrorBadGateway(
            "GitHub returned a different source tree OID",
        ));
    }

    if tree
        .entries
        .iter()
        .any(|entry| !matches!(entry.kind.as_str(), "blob" | "tree") || entry.mode == "160000")
    {
        return Err(error::ErrorUnprocessableEntity(
            "published source contains a Git submodule or unsupported tree entry",
        ));
    }
    let mut entries = tree
        .entries
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .collect::<Vec<_>>();
    if entries.len() > MAX_PUBLISHED_SOURCE_FILES {
        return Err(error::ErrorPayloadTooLarge(
            "published source contains too many files",
        ));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let declared_size = entries
        .iter()
        .map(|entry| entry.size.unwrap_or(MAX_PUBLISHED_SOURCE_FILE_BYTES + 1))
        .try_fold(0_u64, u64::checked_add)
        .ok_or_else(|| error::ErrorPayloadTooLarge("published source is too large"))?;
    if declared_size > MAX_PUBLISHED_SOURCE_BYTES {
        return Err(error::ErrorPayloadTooLarge("published source is too large"));
    }
    if entries.iter().any(|entry| entry.mode == "120000") {
        return Err(error::ErrorUnprocessableEntity(
            "published source contains symbolic links, which are not supported",
        ));
    }
    if entries.iter().any(|entry| !safe_archive_path(&entry.path)) {
        return Err(error::ErrorInternalServerError(
            "published source contains an unsafe path",
        ));
    }

    let github = state.github.clone();
    let owner = source.owner.clone();
    let repository = source.repository.clone();
    let mut downloads = stream::iter(entries.into_iter().map(|entry| {
        let github = github.clone();
        let owner = owner.clone();
        let repository = repository.clone();
        let access_token = access_token.clone();
        async move {
            let bytes = github
                .published_blob_with_token(
                    &owner,
                    &repository,
                    &access_token,
                    &entry.sha,
                    MAX_PUBLISHED_SOURCE_FILE_BYTES,
                )
                .await?;
            Ok::<_, String>((entry, bytes))
        }
    }))
    .buffer_unordered(GITHUB_BLOB_CONCURRENCY);
    let mut files = Vec::new();
    let mut actual_size = 0_u64;
    while let Some(result) = downloads.next().await {
        let (entry, bytes) = result.map_err(error::ErrorBadGateway)?;
        actual_size = actual_size
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error::ErrorPayloadTooLarge("published source is too large"))?;
        if actual_size > MAX_PUBLISHED_SOURCE_BYTES {
            return Err(error::ErrorPayloadTooLarge("published source is too large"));
        }
        files.push((entry, bytes));
    }
    files.sort_by(|left, right| left.0.path.cmp(&right.0.path));

    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = tar::Builder::new(encoder);
    for (entry, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header
            .set_path(&entry.path)
            .map_err(error::ErrorInternalServerError)?;
        header.set_size(bytes.len() as u64);
        header.set_mode(if entry.mode == "100755" { 0o755 } else { 0o644 });
        header.set_mtime(0);
        header.set_cksum();
        archive
            .append(&header, Cursor::new(bytes))
            .map_err(error::ErrorInternalServerError)?;
    }
    let encoder = archive
        .into_inner()
        .map_err(error::ErrorInternalServerError)?;
    let bytes = encoder.finish().map_err(error::ErrorInternalServerError)?;
    increment_project_download(&state.database, &project_kind, &project_id)?;
    Ok(HttpResponse::Ok()
        .content_type("application/gzip")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{project_id}.tar.gz\""),
        ))
        .body(bytes))
}

fn safe_archive_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn published_source(
    database: &Database,
    project_kind: &str,
    project_id: &str,
) -> Result<Option<PublishedSource>> {
    if project_kind != "mods" {
        return Ok(None);
    }
    let Some(state) = database
        .get_registry_mod_state(project_id)
        .map_err(database_http_error)?
    else {
        return Ok(None);
    };
    let Some(latest_id) = state.mod_record.latest_version_id.as_deref() else {
        return Ok(None);
    };
    let version = state
        .versions
        .iter()
        .find(|version| version.id == latest_id)
        .ok_or_else(|| error::ErrorInternalServerError("latest mod version is missing"))?;
    Ok(Some(PublishedSource {
        repository_id: state.repository.provider_repository_id,
        owner: state.repository.owner,
        repository: state.repository.name,
        tree_oid: version.source_tree_oid.clone(),
    }))
}

#[derive(Clone, Copy)]
enum PublishedArtifactKind {
    Manifest,
    Readme,
    Image,
}

struct PublishedArtifact {
    repository_id: i64,
    owner: String,
    repository: String,
    path: String,
    blob_oid: String,
}

async fn get_published_artifact(
    state: web::Data<RegistryState>,
    path: web::Path<(String, String, String)>,
) -> Result<HttpResponse> {
    let (project_kind, project_id, artifact) = path.into_inner();
    let artifact_kind = match artifact.as_str() {
        "manifest" => PublishedArtifactKind::Manifest,
        "readme" => PublishedArtifactKind::Readme,
        "image" => PublishedArtifactKind::Image,
        _ => return Err(error::ErrorNotFound("published artifact not found")),
    };
    let artifact = published_artifact(&state.database, &project_kind, &project_id, artifact_kind)?
        .ok_or_else(|| error::ErrorNotFound("published artifact not found"))?;
    let maximum_size = match artifact_kind {
        PublishedArtifactKind::Manifest => MAX_PUBLISHED_MANIFEST_BYTES,
        PublishedArtifactKind::Readme => MAX_PUBLISHED_README_BYTES,
        PublishedArtifactKind::Image => MAX_PUBLISHED_IMAGE_BYTES,
    };
    let bytes = state
        .github
        .published_blob(
            &artifact.owner,
            &artifact.repository,
            artifact.repository_id,
            &artifact.blob_oid,
            maximum_size,
        )
        .await
        .map_err(error::ErrorBadGateway)?;
    if matches!(artifact_kind, PublishedArtifactKind::Manifest) {
        increment_project_download(&state.database, &project_kind, &project_id)?;
    }
    let filename = Path::new(&artifact.path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        })
        .unwrap_or("artifact")
        .to_owned();
    Ok(HttpResponse::Ok()
        .content_type(artifact_content_type(artifact_kind, &artifact.path))
        .insert_header(("X-Patchwork-Filename", filename))
        .body(bytes))
}

fn increment_project_download(
    database: &Database,
    project_kind: &str,
    project_id: &str,
) -> Result<i64> {
    match project_kind {
        "mods" => database
            .increment_mod_downloads(project_id)
            .map_err(database_http_error),
        "modpacks" => database
            .increment_modpack_downloads(project_id)
            .map_err(database_http_error),
        _ => Err(error::ErrorNotFound("published project was not found")),
    }
}

fn published_artifact(
    database: &Database,
    project_kind: &str,
    project_id: &str,
    artifact_kind: PublishedArtifactKind,
) -> Result<Option<PublishedArtifact>> {
    if project_kind == "mods" {
        let Some(state) = database
            .get_registry_mod_state(project_id)
            .map_err(database_http_error)?
        else {
            return Ok(None);
        };
        let Some(latest_id) = state.mod_record.latest_version_id.as_deref() else {
            return Ok(None);
        };
        let version = state
            .versions
            .iter()
            .find(|version| version.id == latest_id)
            .ok_or_else(|| error::ErrorInternalServerError("latest mod version is missing"))?;
        let coordinate = match artifact_kind {
            PublishedArtifactKind::Manifest => Some((
                version.manifest_path.clone(),
                version.manifest_blob_oid.clone(),
            )),
            PublishedArtifactKind::Readme => version
                .readme_path
                .clone()
                .zip(version.readme_blob_oid.clone()),
            PublishedArtifactKind::Image => version
                .image_path
                .clone()
                .zip(version.image_blob_oid.clone()),
        };
        return Ok(coordinate.map(|(path, blob_oid)| PublishedArtifact {
            repository_id: state.repository.provider_repository_id,
            owner: state.repository.owner,
            repository: state.repository.name,
            path,
            blob_oid,
        }));
    }
    if project_kind == "modpacks" {
        let Some(state) = database
            .get_registry_modpack_state(project_id)
            .map_err(database_http_error)?
        else {
            return Ok(None);
        };
        let Some(latest_id) = state.modpack_record.latest_version_id.as_deref() else {
            return Ok(None);
        };
        let version = state
            .versions
            .iter()
            .find(|version| version.id == latest_id)
            .ok_or_else(|| error::ErrorInternalServerError("latest modpack version is missing"))?;
        let coordinate = match artifact_kind {
            PublishedArtifactKind::Manifest => Some((
                version.manifest_path.clone(),
                version.manifest_blob_oid.clone(),
            )),
            PublishedArtifactKind::Readme => version
                .readme_path
                .clone()
                .zip(version.readme_blob_oid.clone()),
            PublishedArtifactKind::Image => version
                .image_path
                .clone()
                .zip(version.image_blob_oid.clone()),
        };
        return Ok(coordinate.map(|(path, blob_oid)| PublishedArtifact {
            repository_id: state.repository.provider_repository_id,
            owner: state.repository.owner,
            repository: state.repository.name,
            path,
            blob_oid,
        }));
    }
    Ok(None)
}

fn artifact_content_type(kind: PublishedArtifactKind, path: &str) -> &'static str {
    match kind {
        PublishedArtifactKind::Manifest => "application/toml; charset=utf-8",
        PublishedArtifactKind::Readme => "text/markdown; charset=utf-8",
        PublishedArtifactKind::Image => match Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("webp") => "image/webp",
            Some("jpg" | "jpeg") => "image/jpeg",
            _ => "image/png",
        },
    }
}

async fn create_scan(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    body: web::Json<RegistryScanRequest>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let scan = scan_repository(&state, &account, body.into_inner(), None).await?;
    Ok(HttpResponse::Created().json(scan))
}

async fn start_scan_job(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    body: web::Json<RegistryScanRequest>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let publisher_uuid = account_uuid(&account)?;
    let input = body.into_inner();
    let (reporter, started) = state.start_job(publisher_uuid);
    let task_state = state.get_ref().clone();
    actix_web::rt::spawn(async move {
        match scan_repository(&task_state, &account, input, Some(&reporter)).await {
            Ok(scan) => reporter.complete(scan),
            Err(scan_error) => reporter.fail(scan_error.to_string()),
        }
    });
    Ok(HttpResponse::Accepted().json(started))
}

async fn get_scan_job(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    job_id: web::Path<String>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let publisher_uuid = account_uuid(&account)?;
    let job_id = parse_uuid("job_id", &job_id)?;
    let progress = state
        .job_progress(job_id, publisher_uuid)
        .ok_or_else(|| error::ErrorNotFound("registry scan job was not found"))?;
    Ok(HttpResponse::Ok().json(progress))
}

async fn get_scan(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    scan_id: web::Path<String>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let publisher_uuid = account_uuid(&account)?;
    let scan_id = parse_uuid("scan_id", &scan_id)?;
    let scan = state
        .database
        .get_registry_scan(scan_id, publisher_uuid)
        .map_err(database_http_error)?
        .ok_or_else(|| error::ErrorNotFound("registry scan was not found"))?;
    Ok(HttpResponse::Ok().json(scan_dto(scan)?))
}

async fn publish_scan(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    scan_id: web::Path<String>,
    body: web::Json<RegistryPublishRequest>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let publisher_uuid = account_uuid(&account)?;
    let github = linked_github(&state.database, publisher_uuid)?;
    let scan_id = parse_uuid("scan_id", &scan_id)?;
    let entry_ids = body
        .entry_ids
        .iter()
        .map(|value| parse_uuid("entry_id", value))
        .collect::<Result<Vec<_>>>()?;
    let published = state
        .database
        .publish_registry_scan(
            scan_id,
            publisher_uuid,
            github.github_user_id,
            &entry_ids,
            Utc::now().naive_utc(),
        )
        .map_err(database_http_error)?;
    Ok(HttpResponse::Ok().json(publish_dto(published)))
}

async fn rescan_mod(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    mod_id: web::Path<String>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let input = rescan_input(&state.database, &account, &mod_id)?;
    let scan = scan_repository(&state, &account, input, None).await?;
    Ok(HttpResponse::Created().json(scan))
}

async fn start_rescan_job(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    mod_id: web::Path<String>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let publisher_uuid = account_uuid(&account)?;
    let input = rescan_input(&state.database, &account, &mod_id)?;
    let (reporter, started) = state.start_job(publisher_uuid);
    let task_state = state.get_ref().clone();
    actix_web::rt::spawn(async move {
        match scan_repository(&task_state, &account, input, Some(&reporter)).await {
            Ok(scan) => reporter.complete(scan),
            Err(scan_error) => reporter.fail(scan_error.to_string()),
        }
    });
    Ok(HttpResponse::Accepted().json(started))
}

fn rescan_input(
    database: &Database,
    account: &Account,
    mod_id: &str,
) -> Result<RegistryScanRequest> {
    let publisher_uuid = account_uuid(account)?;
    let current = database
        .get_registry_mod_state(mod_id)
        .map_err(database_http_error)?
        .ok_or_else(|| error::ErrorNotFound("mod was not found"))?;
    if current.mod_record.publisher_uuid != publisher_uuid.hyphenated().to_string() {
        return Err(error::ErrorNotFound("mod was not found"));
    }
    Ok(RegistryScanRequest {
        repository_url: current.repository.canonical_url,
        base_path: current.mod_record.source_base_path,
    })
}

async fn get_project_details(
    state: web::Data<RegistryState>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    let (project_kind, project_id) = path.into_inner();
    if project_kind == "mods" && is_generated_mod_id(&project_id) {
        return Err(error::ErrorNotFound("published project was not found"));
    }
    let details = match project_kind.as_str() {
        "mods" => mod_details(&state, &project_id)?,
        "modpacks" => modpack_details(&state, &project_id)?,
        _ => None,
    }
    .ok_or_else(|| error::ErrorNotFound("published project was not found"))?;
    Ok(HttpResponse::Ok().json(details))
}

fn mod_details(
    registry: &RegistryState,
    project_id: &str,
) -> Result<Option<RegistryProjectDetails>> {
    let database = &registry.database;
    let Some(state) = database
        .get_registry_mod_state(project_id)
        .map_err(database_http_error)?
    else {
        return Ok(None);
    };
    let Some(latest_id) = state.mod_record.latest_version_id.as_deref() else {
        return Ok(None);
    };
    let version = state
        .versions
        .iter()
        .find(|version| version.id == latest_id)
        .ok_or_else(|| error::ErrorInternalServerError("latest mod version is missing"))?;
    let publisher = published_project_account(database, &state.mod_record.publisher_uuid)?;
    let mut dependencies = database
        .list_mod_version_dependencies(&version.id)
        .map_err(database_http_error)?
        .into_iter()
        .map(|dependency| {
            registry_dependency(
                database,
                &dependency.relation_kind,
                &dependency.target_kind,
                dependency.target_id,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let metadata = serde_json::from_str::<patchwork::ModInfo>(&version.metadata_json)
        .map_err(|_| error::ErrorInternalServerError("stored mod metadata is invalid"))?;
    if let Some(provided_api) = metadata.provides {
        dependencies.push(registry_dependency(
            database,
            "provides",
            "mod",
            provided_api,
        )?);
    }
    let route = registry.route(&format!("/registry/projects/mods/{}", state.mod_record.id));
    Ok(Some(RegistryProjectDetails {
        project_kind: RegistryProjectKind::Mod,
        project_id: state.mod_record.id,
        title: version.title.clone(),
        description: String::new(),
        version: version.version.clone(),
        downloads: Some(state.mod_record.downloads),
        publisher_uuid: publisher.uuid,
        publisher_name: publisher.nickname,
        published_at: version
            .published_at
            .and_utc()
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        repository_url: state.repository.canonical_url,
        repository_path: version.repository_path.clone(),
        source_commit: version.source_commit.clone(),
        source_tree_oid: version.source_tree_oid.clone(),
        manifest_sha256: version.manifest_sha256.clone(),
        manifest_url: format!("{route}/manifest"),
        source_url: Some(format!("{route}/source")),
        readme_url: version
            .readme_path
            .as_ref()
            .map(|_| format!("{route}/readme")),
        image_url: version
            .image_path
            .as_ref()
            .map(|_| format!("{route}/image")),
        dependencies,
    }))
}

fn modpack_details(
    registry: &RegistryState,
    project_id: &str,
) -> Result<Option<RegistryProjectDetails>> {
    let database = &registry.database;
    let Some(state) = database
        .get_registry_modpack_state(project_id)
        .map_err(database_http_error)?
    else {
        return Ok(None);
    };
    let Some(latest_id) = state.modpack_record.latest_version_id.as_deref() else {
        return Ok(None);
    };
    let version = state
        .versions
        .iter()
        .find(|version| version.id == latest_id)
        .ok_or_else(|| error::ErrorInternalServerError("latest modpack version is missing"))?;
    let publisher = published_project_account(database, &state.modpack_record.publisher_uuid)?;
    let dependencies = database
        .list_modpack_version_dependencies(&version.id)
        .map_err(database_http_error)?
        .into_iter()
        .map(|dependency| {
            registry_dependency(
                database,
                &dependency.relation_kind,
                &dependency.target_kind,
                dependency.target_id,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let route = registry.route(&format!(
        "/registry/projects/modpacks/{}",
        state.modpack_record.id
    ));
    Ok(Some(RegistryProjectDetails {
        project_kind: RegistryProjectKind::Modpack,
        project_id: state.modpack_record.id,
        title: version.title.clone(),
        description: version.description.clone(),
        version: version.version.clone(),
        downloads: Some(state.modpack_record.downloads),
        publisher_uuid: publisher.uuid,
        publisher_name: publisher.nickname,
        published_at: version
            .published_at
            .and_utc()
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        repository_url: state.repository.canonical_url,
        repository_path: version.repository_path.clone(),
        source_commit: version.source_commit.clone(),
        source_tree_oid: version.source_tree_oid.clone(),
        manifest_sha256: version.manifest_sha256.clone(),
        manifest_url: format!("{route}/manifest"),
        source_url: None,
        readme_url: version
            .readme_path
            .as_ref()
            .map(|_| format!("{route}/readme")),
        image_url: version
            .image_path
            .as_ref()
            .map(|_| format!("{route}/image")),
        dependencies,
    }))
}

fn published_project_account(database: &Database, publisher_uuid: &str) -> Result<Account> {
    let publisher_uuid = Uuid::parse_str(publisher_uuid)
        .map_err(|_| error::ErrorInternalServerError("stored publisher UUID is invalid"))?;
    database
        .get_account(publisher_uuid)
        .map_err(database_http_error)?
        .ok_or_else(|| error::ErrorInternalServerError("project publisher is missing"))
}

fn registry_dependency(
    database: &Database,
    relation_kind: &str,
    target_kind: &str,
    target_id: String,
) -> Result<RegistryDependency> {
    let kind = match relation_kind {
        "init" => RegistryDependencyKind::Init,
        "run" => RegistryDependencyKind::Run,
        "ownership" => RegistryDependencyKind::Ownership,
        "provides" => RegistryDependencyKind::Provides,
        "mod" => RegistryDependencyKind::Mod,
        "modpack" => RegistryDependencyKind::Modpack,
        "ignore" => RegistryDependencyKind::Ignore,
        _ => {
            return Err(error::ErrorInternalServerError(
                "invalid stored dependency kind",
            ));
        }
    };
    let target_kind = match target_kind {
        "mod" => RegistryProjectKind::Mod,
        "modpack" => RegistryProjectKind::Modpack,
        _ => {
            return Err(error::ErrorInternalServerError(
                "invalid stored project kind",
            ));
        }
    };
    let available = match target_kind {
        RegistryProjectKind::Mod if is_generated_mod_id(&target_id) => false,
        RegistryProjectKind::Mod => database
            .get_mod(&target_id)
            .map_err(database_http_error)?
            .is_some(),
        RegistryProjectKind::Modpack => database
            .get_modpack(&target_id)
            .map_err(database_http_error)?
            .is_some(),
    };
    Ok(RegistryDependency {
        kind,
        target_kind,
        target_id,
        available,
    })
}

async fn rescan_project(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let (project_kind, project_id) = path.into_inner();
    let input = project_rescan_input(&state.database, &account, &project_kind, &project_id)?;
    let scan = scan_repository(&state, &account, input, None).await?;
    Ok(HttpResponse::Created().json(scan))
}

async fn start_project_rescan_job(
    state: web::Data<RegistryState>,
    request: HttpRequest,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    let account = require_account(&state.database, &request)?;
    let publisher_uuid = account_uuid(&account)?;
    let (project_kind, project_id) = path.into_inner();
    let input = project_rescan_input(&state.database, &account, &project_kind, &project_id)?;
    let (reporter, started) = state.start_job(publisher_uuid);
    let task_state = state.get_ref().clone();
    actix_web::rt::spawn(async move {
        match scan_repository(&task_state, &account, input, Some(&reporter)).await {
            Ok(scan) => reporter.complete(scan),
            Err(scan_error) => reporter.fail(scan_error.to_string()),
        }
    });
    Ok(HttpResponse::Accepted().json(started))
}

fn project_rescan_input(
    database: &Database,
    account: &Account,
    project_kind: &str,
    project_id: &str,
) -> Result<RegistryScanRequest> {
    if project_kind == "mods" {
        return rescan_input(database, account, project_id);
    }
    if project_kind != "modpacks" {
        return Err(error::ErrorBadRequest(
            "project_kind must be mods or modpacks",
        ));
    }
    let publisher_uuid = account_uuid(account)?;
    let current = database
        .get_registry_modpack_state(project_id)
        .map_err(database_http_error)?
        .ok_or_else(|| error::ErrorNotFound("modpack was not found"))?;
    if current.modpack_record.publisher_uuid != publisher_uuid.hyphenated().to_string() {
        return Err(error::ErrorNotFound("modpack was not found"));
    }
    Ok(RegistryScanRequest {
        repository_url: current.repository.canonical_url,
        base_path: current.modpack_record.source_base_path,
    })
}

async fn scan_repository(
    state: &RegistryState,
    account: &Account,
    input: RegistryScanRequest,
    reporter: Option<&ScanReporter>,
) -> Result<RegistryScan> {
    report_phase(reporter, RegistryScanPhase::Authorizing, 0, None);
    let publisher_uuid = account_uuid(account)?;
    let linked = linked_github(&state.database, publisher_uuid)?;
    let coordinates = normalize_repository_url(&input.repository_url)?;
    let base_path = normalize_base_path(&input.base_path)?;

    let repository = state
        .github
        .authorize_repository(&coordinates.owner, &coordinates.name, linked.github_user_id)
        .await
        .map_err(error::ErrorForbidden)?;

    state
        .database
        .link_github_account(
            publisher_uuid,
            repository.github_user.id,
            &repository.github_user.login,
            &repository.github_user.avatar_url,
            Utc::now().naive_utc(),
        )
        .map_err(database_http_error)?;

    let commit = state
        .github
        .resolve_commit(&repository, &repository.default_branch)
        .await
        .map_err(error::ErrorBadGateway)?;
    report_phase(reporter, RegistryScanPhase::IndexingRepository, 0, None);
    let indexed = index_repository(
        &state.github,
        &repository,
        &commit.tree_sha,
        &base_path,
        reporter,
    )
    .await
    .map_err(error::ErrorBadGateway)?;
    report_phase(
        reporter,
        RegistryScanPhase::FetchingManifests,
        0,
        Some(indexed.manifests.len()),
    );
    let manifests = fetch_manifests(&state.github, &repository, indexed.manifests, reporter)
        .await
        .map_err(error::ErrorBadGateway)?;

    let (mut candidates, mut scan_warnings, scan_errors) =
        parse_candidates(&manifests, &indexed.directories, &base_path);
    scan_warnings.extend(indexed.warnings);
    if candidates.is_empty() {
        scan_warnings.push(format!(
            "No Patchwork mods or modpacks were found below {}.",
            display_path(&base_path)
        ));
    }

    report_phase(
        reporter,
        RegistryScanPhase::ValidatingProjects,
        0,
        Some(candidates.len()),
    );
    validate_candidates(
        &state.database,
        publisher_uuid,
        repository.id,
        &mut candidates,
        reporter,
    )?;

    report_phase(reporter, RegistryScanPhase::Persisting, 0, Some(1));
    let now = Utc::now().naive_utc();
    let stored = state
        .database
        .create_registry_scan(
            CreateRegistryScan {
                id: Uuid::new_v4(),
                publisher_uuid,
                github_user_id: linked.github_user_id,
                github_repository_id: repository.id,
                repository_owner: repository.owner,
                repository_name: repository.name,
                repository_url: repository.canonical_url,
                base_path,
                requested_ref: repository.default_branch,
                resolved_commit: commit.sha,
                root_tree_oid: indexed.base_tree_oid,
                warnings_json: json_string(&scan_warnings)?,
                errors_json: json_string(&scan_errors)?,
                expires_at: now + Duration::minutes(SCAN_TTL_MINUTES),
                entries: candidates
                    .into_iter()
                    .map(Candidate::into_database_entry)
                    .collect::<Result<Vec<_>>>()?,
            },
            now,
        )
        .map_err(database_http_error)?;
    scan_dto(stored)
}

fn report_phase(
    reporter: Option<&ScanReporter>,
    phase: RegistryScanPhase,
    completed: usize,
    total: Option<usize>,
) {
    if let Some(reporter) = reporter {
        reporter.phase(phase, completed, total);
    }
}

struct RepositoryCoordinates {
    owner: String,
    name: String,
}

fn normalize_repository_url(value: &str) -> Result<RepositoryCoordinates> {
    let value = value.trim();
    if value.is_empty() {
        return Err(error::ErrorBadRequest("repository URL is required"));
    }
    let url = Url::parse(value).map_err(|_| {
        error::ErrorBadRequest("repository URL must look like https://github.com/owner/repository")
    })?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("github.com") | Some("www.github.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error::ErrorBadRequest(
            "repository URL must be a plain HTTPS github.com URL",
        ));
    }
    let parts = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if parts.len() != 2 || parts.iter().any(|part| part.contains('%')) {
        return Err(error::ErrorBadRequest(
            "repository URL must identify exactly one GitHub repository",
        ));
    }
    let owner = parts[0];
    let name = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
    if !valid_coordinate(owner) || !valid_coordinate(name) {
        return Err(error::ErrorBadRequest(
            "invalid GitHub owner or repository name",
        ));
    }
    Ok(RepositoryCoordinates {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn valid_coordinate(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn normalize_base_path(value: &str) -> Result<String> {
    let value = value.trim().replace('\\', "/");
    if value.is_empty() || value == "." {
        return Ok(".".to_owned());
    }
    if value.starts_with('/') || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(error::ErrorBadRequest(
            "base path must be a relative repository path of at most 1024 bytes",
        ));
    }
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(error::ErrorBadRequest("base path must not contain '..'"));
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        Ok(".".to_owned())
    } else if parts.len() > MAX_DIRECTORY_DEPTH {
        Err(error::ErrorBadRequest(format!(
            "base path must contain at most {MAX_DIRECTORY_DEPTH} path components"
        )))
    } else {
        Ok(parts.join("/"))
    }
}

struct IndexedRepository {
    base_tree_oid: String,
    directories: BTreeMap<String, DirectorySnapshot>,
    manifests: Vec<ManifestReference>,
    warnings: Vec<String>,
}

#[derive(Clone)]
struct DirectorySnapshot {
    tree_oid: String,
    blobs: Vec<GithubTreeEntry>,
}

#[derive(Clone)]
struct ManifestReference {
    path: String,
    blob_oid: String,
    size: Option<u64>,
    scan_candidate: bool,
    kind: ManifestKind,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ManifestKind {
    Cargo,
    Modpack,
}

async fn index_repository(
    github: &GithubClient,
    repository: &AuthorizedGithubRepository,
    commit_tree_oid: &str,
    base_path: &str,
    reporter: Option<&ScanReporter>,
) -> std::result::Result<IndexedRepository, String> {
    let mut manifest_map = BTreeMap::<String, ManifestReference>::new();
    let mut current_path = ".".to_owned();
    let mut current_tree = github.tree(repository, commit_tree_oid).await?;

    if base_path != "." {
        let components = base_path.split('/').collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            remember_manifest(&mut manifest_map, &current_path, &current_tree, false);
            let child = current_tree
                .entries
                .iter()
                .find(|entry| entry.path == *component)
                .ok_or_else(|| {
                    format!(
                        "path '{}' does not exist at commit {}",
                        base_path, commit_tree_oid
                    )
                })?;
            let is_last = index + 1 == components.len();
            if is_last
                && child.kind == "blob"
                && is_regular_blob(&child.mode)
                && component.to_ascii_lowercase().ends_with(".toml")
                && *component != "Cargo.toml"
            {
                let directory = DirectorySnapshot {
                    tree_oid: current_tree.sha.clone(),
                    blobs: current_tree
                        .entries
                        .iter()
                        .filter(|entry| entry.kind == "blob" && is_regular_blob(&entry.mode))
                        .cloned()
                        .collect(),
                };
                manifest_map.insert(
                    base_path.to_owned(),
                    ManifestReference {
                        path: base_path.to_owned(),
                        blob_oid: child.sha.clone(),
                        size: child.size,
                        scan_candidate: true,
                        kind: ManifestKind::Modpack,
                    },
                );
                let base_tree_oid = current_tree.sha.clone();
                report_phase(reporter, RegistryScanPhase::IndexingRepository, 1, Some(1));
                return Ok(IndexedRepository {
                    base_tree_oid,
                    directories: BTreeMap::from([(current_path, directory)]),
                    manifests: manifest_map.into_values().collect(),
                    warnings: Vec::new(),
                });
            }
            if child.kind != "tree" {
                return Err(format!(
                    "path '{}' is not a directory or a loose modpack TOML",
                    base_path
                ));
            }
            current_path = join_repo_path(&current_path, component);
            current_tree = github.tree(repository, &child.sha).await?;
        }
    }

    let base_tree_oid = current_tree.sha.clone();
    let recursive_tree = github.recursive_tree(repository, &base_tree_oid).await?;
    if recursive_tree.entries.len() > MAX_TREE_ENTRIES {
        return Err(format!(
            "scan exceeds the {MAX_TREE_ENTRIES} Git tree entry safety limit"
        ));
    }

    let mut directories = BTreeMap::from([(
        current_path.clone(),
        DirectorySnapshot {
            tree_oid: base_tree_oid.clone(),
            blobs: Vec::new(),
        },
    )]);
    let mut warnings = Vec::new();

    for entry in &recursive_tree.entries {
        let full_path = join_repo_path(&current_path, &entry.path);
        if path_depth(&full_path) > MAX_DIRECTORY_DEPTH {
            return Err(format!(
                "scan exceeds the {MAX_DIRECTORY_DEPTH} level directory depth limit"
            ));
        }
        match entry.kind.as_str() {
            "tree" => {
                directories.insert(
                    full_path,
                    DirectorySnapshot {
                        tree_oid: entry.sha.clone(),
                        blobs: Vec::new(),
                    },
                );
            }
            "commit" => warnings.push(format!("Git submodule {full_path} was not scanned.")),
            _ => {}
        }
    }
    if directories.len() > MAX_DIRECTORIES {
        return Err(format!(
            "scan exceeds the {MAX_DIRECTORIES} directory safety limit"
        ));
    }

    for entry in recursive_tree
        .entries
        .into_iter()
        .filter(|entry| entry.kind == "blob" && is_regular_blob(&entry.mode))
    {
        let relative_directory = repository_parent(&entry.path);
        let directory_path = if relative_directory == "." {
            current_path.clone()
        } else {
            join_repo_path(&current_path, &relative_directory)
        };
        let file_name = entry
            .path
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(&entry.path)
            .to_owned();
        let directory = directories
            .get_mut(&directory_path)
            .ok_or_else(|| format!("GitHub tree omitted parent directory for {}", entry.path))?;
        let mut local_entry = entry.clone();
        local_entry.path = file_name.clone();
        directory.blobs.push(local_entry);

        if file_name.to_ascii_lowercase().ends_with(".toml") {
            let kind = if file_name == "Cargo.toml" {
                ManifestKind::Cargo
            } else {
                ManifestKind::Modpack
            };
            let path = join_repo_path(&directory_path, &file_name);
            manifest_map.insert(
                path.clone(),
                ManifestReference {
                    path,
                    blob_oid: entry.sha,
                    size: entry.size,
                    scan_candidate: true,
                    kind,
                },
            );
        }
    }

    report_phase(
        reporter,
        RegistryScanPhase::IndexingRepository,
        directories.len(),
        Some(directories.len()),
    );

    let scan_manifests = manifest_map
        .values()
        .filter(|manifest| manifest.scan_candidate)
        .count();
    if scan_manifests > MAX_MANIFESTS {
        return Err(format!(
            "scan found {scan_manifests} candidate manifests; the limit is {MAX_MANIFESTS}"
        ));
    }

    Ok(IndexedRepository {
        base_tree_oid,
        directories,
        manifests: manifest_map.into_values().collect(),
        warnings,
    })
}

fn remember_manifest(
    manifests: &mut BTreeMap<String, ManifestReference>,
    directory_path: &str,
    tree: &GithubTree,
    scan_candidate: bool,
) {
    let Some(entry) = tree.entries.iter().find(|entry| {
        entry.path == "Cargo.toml" && entry.kind == "blob" && is_regular_blob(&entry.mode)
    }) else {
        return;
    };
    let path = join_repo_path(directory_path, "Cargo.toml");
    manifests
        .entry(path.clone())
        .and_modify(|manifest| manifest.scan_candidate |= scan_candidate)
        .or_insert_with(|| ManifestReference {
            path,
            blob_oid: entry.sha.clone(),
            size: entry.size,
            scan_candidate,
            kind: ManifestKind::Cargo,
        });
}

fn is_regular_blob(mode: &str) -> bool {
    matches!(mode, "100644" | "100755")
}

fn path_depth(path: &str) -> usize {
    if path == "." {
        0
    } else {
        path.split('/').count()
    }
}

async fn fetch_manifests(
    github: &GithubClient,
    repository: &AuthorizedGithubRepository,
    references: Vec<ManifestReference>,
    reporter: Option<&ScanReporter>,
) -> std::result::Result<Vec<ManifestSource>, String> {
    let total = references.len();
    let mut manifests = Vec::with_capacity(references.len());
    let mut downloads = stream::iter(references)
        .map(|reference| async move {
            if reference.size.is_some_and(|size| size > MAX_MANIFEST_BYTES) {
                return Err(format!(
                    "{} exceeds the {MAX_MANIFEST_BYTES} byte Cargo.toml limit",
                    reference.path
                ));
            }
            let bytes = github
                .blob(repository, &reference.blob_oid, MAX_MANIFEST_BYTES)
                .await?;
            let source = String::from_utf8(bytes)
                .map_err(|_| format!("{} is not valid UTF-8", reference.path))?;
            Ok(ManifestSource {
                path: reference.path,
                blob_oid: reference.blob_oid,
                source,
                scan_candidate: reference.scan_candidate,
                kind: reference.kind,
            })
        })
        .buffer_unordered(GITHUB_BLOB_CONCURRENCY);

    while let Some(manifest) = downloads.next().await {
        manifests.push(manifest?);
        report_phase(
            reporter,
            RegistryScanPhase::FetchingManifests,
            manifests.len(),
            Some(total),
        );
    }
    manifests.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(manifests)
}

struct ManifestSource {
    path: String,
    blob_oid: String,
    source: String,
    scan_candidate: bool,
    kind: ManifestKind,
}

struct Candidate {
    entry_id: Uuid,
    project_kind: RegistryProjectKind,
    project_id: String,
    title: String,
    description: String,
    version: String,
    repository_path: String,
    source_tree_oid: String,
    manifest_path: String,
    manifest_blob_oid: String,
    manifest_sha256: String,
    readme_path: Option<String>,
    readme_blob_oid: Option<String>,
    image_path: Option<String>,
    image_blob_oid: Option<String>,
    status: RegistryScanStatus,
    metadata: Value,
    dependencies: Vec<RegistryDependency>,
    warnings: Vec<String>,
    errors: Vec<String>,
}

impl Candidate {
    fn to_dto(&self) -> RegistryScanEntry {
        RegistryScanEntry {
            entry_id: self.entry_id.hyphenated().to_string(),
            project_kind: self.project_kind,
            project_id: self.project_id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            version: self.version.clone(),
            repository_path: self.repository_path.clone(),
            manifest_path: self.manifest_path.clone(),
            source_tree_oid: self.source_tree_oid.clone(),
            manifest_blob_oid: self.manifest_blob_oid.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            readme_path: self.readme_path.clone(),
            readme_blob_oid: self.readme_blob_oid.clone(),
            image_path: self.image_path.clone(),
            image_blob_oid: self.image_blob_oid.clone(),
            status: self.status,
            dependencies: self.dependencies.clone(),
            warnings: self.warnings.clone(),
            errors: self.errors.clone(),
        }
    }

    fn into_database_entry(self) -> Result<CreateRegistryScanEntry> {
        Ok(CreateRegistryScanEntry {
            id: self.entry_id,
            project_kind: project_kind_to_database(self.project_kind).to_owned(),
            project_id: self.project_id,
            version: self.version,
            title: self.title,
            description: self.description,
            repository_path: self.repository_path,
            source_tree_oid: self.source_tree_oid,
            manifest_path: self.manifest_path,
            manifest_blob_oid: self.manifest_blob_oid,
            manifest_sha256: self.manifest_sha256,
            readme_path: self.readme_path,
            readme_blob_oid: self.readme_blob_oid,
            image_path: self.image_path,
            image_blob_oid: self.image_blob_oid,
            status: status_to_database(self.status).to_owned(),
            metadata_json: json_string(&self.metadata)?,
            dependencies_json: json_string(&self.dependencies)?,
            warnings_json: json_string(&self.warnings)?,
            errors_json: json_string(&self.errors)?,
        })
    }
}

fn parse_candidates(
    manifests: &[ManifestSource],
    directories: &BTreeMap<String, DirectorySnapshot>,
    base_path: &str,
) -> (Vec<Candidate>, Vec<String>, Vec<String>) {
    let workspace_manifests = manifests
        .iter()
        .filter(|manifest| manifest.kind == ManifestKind::Cargo)
        .map(|manifest| RegistryWorkspaceManifest {
            path: Path::new(&manifest.path),
            source: &manifest.source,
        })
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    for manifest in manifests.iter().filter(|manifest| manifest.scan_candidate) {
        let path = Path::new(&manifest.path);
        if manifest.kind == ManifestKind::Modpack {
            match parse_registry_modpack_manifest(&manifest.source, path) {
                Ok(Some(parsed)) => push_modpack_candidate(
                    &mut candidates,
                    &mut warnings,
                    manifest,
                    parsed,
                    directories,
                ),
                Ok(None) => {}
                Err(parse_error) => {
                    if let Some(identity) = fallback_modpack_identity(manifest) {
                        let directory_path = repository_parent(&manifest.path);
                        if let Some(directory) = directories.get(&directory_path) {
                            let (readme_path, readme_blob_oid) = find_modpack_readme(
                                &directory_path,
                                directory,
                                &identity.project_id,
                            );
                            let (image_path, image_blob_oid) =
                                find_image(&directory_path, directory, &identity.project_id);
                            candidates.push(Candidate {
                                entry_id: Uuid::new_v4(),
                                project_kind: RegistryProjectKind::Modpack,
                                project_id: identity.project_id,
                                title: identity.title,
                                description: String::new(),
                                version: identity.version,
                                repository_path: directory_path,
                                source_tree_oid: directory.tree_oid.clone(),
                                manifest_path: manifest.path.clone(),
                                manifest_blob_oid: manifest.blob_oid.clone(),
                                manifest_sha256: sha256_hex(manifest.source.as_bytes()),
                                readme_path,
                                readme_blob_oid,
                                image_path,
                                image_blob_oid,
                                status: RegistryScanStatus::Error,
                                metadata: json!({}),
                                dependencies: Vec::new(),
                                warnings: Vec::new(),
                                errors: vec![parse_error.to_string()],
                            });
                        }
                    } else {
                        warnings.push(format!(
                            "Could not inspect {} as a Patchwork modpack: {}",
                            manifest.path, parse_error
                        ));
                    }
                }
            }
            continue;
        }

        match parse_registry_mod_manifest(&manifest.source, path, &workspace_manifests) {
            Ok(Some(parsed)) => {
                let directory_path = repository_parent(&manifest.path);
                let Some(directory) = directories.get(&directory_path) else {
                    warnings.push(format!(
                        "Could not identify the Git tree for {}.",
                        manifest.path
                    ));
                    continue;
                };
                let (readme_path, readme_blob_oid) = find_readme(&directory_path, directory);
                let (image_path, image_blob_oid) =
                    find_image(&directory_path, directory, &parsed.id);
                let dependencies = dependencies_from_mod_info(&parsed.mod_info);
                let metadata = serde_json::to_value(&parsed.mod_info).unwrap_or_else(|_| json!({}));
                candidates.push(Candidate {
                    entry_id: Uuid::new_v4(),
                    project_kind: RegistryProjectKind::Mod,
                    project_id: parsed.id,
                    title: parsed.title,
                    description: String::new(),
                    version: parsed.version,
                    repository_path: directory_path,
                    source_tree_oid: directory.tree_oid.clone(),
                    manifest_path: manifest.path.clone(),
                    manifest_blob_oid: manifest.blob_oid.clone(),
                    manifest_sha256: sha256_hex(manifest.source.as_bytes()),
                    readme_path,
                    readme_blob_oid,
                    image_path,
                    image_blob_oid,
                    status: RegistryScanStatus::NewMod,
                    metadata,
                    dependencies,
                    warnings: Vec::new(),
                    errors: Vec::new(),
                });
            }
            Ok(None) => {}
            Err(parse_error) => {
                if let Some(identity) = fallback_identity(manifest, manifests) {
                    let directory_path = repository_parent(&manifest.path);
                    if let Some(directory) = directories.get(&directory_path) {
                        let (readme_path, readme_blob_oid) =
                            find_readme(&directory_path, directory);
                        let (image_path, image_blob_oid) =
                            find_image(&directory_path, directory, &identity.mod_id);
                        candidates.push(Candidate {
                            entry_id: Uuid::new_v4(),
                            project_kind: RegistryProjectKind::Mod,
                            project_id: identity.mod_id,
                            title: identity.title,
                            description: String::new(),
                            version: identity.version,
                            repository_path: directory_path,
                            source_tree_oid: directory.tree_oid.clone(),
                            manifest_path: manifest.path.clone(),
                            manifest_blob_oid: manifest.blob_oid.clone(),
                            manifest_sha256: sha256_hex(manifest.source.as_bytes()),
                            readme_path,
                            readme_blob_oid,
                            image_path,
                            image_blob_oid,
                            status: RegistryScanStatus::Error,
                            metadata: json!({}),
                            dependencies: Vec::new(),
                            warnings: Vec::new(),
                            errors: vec![parse_error.to_string()],
                        });
                    }
                } else if declares_patchwork_mod(&manifest.source) {
                    errors.push(format!(
                        "Invalid Patchwork mod at {}: {}",
                        manifest.path, parse_error
                    ));
                } else {
                    warnings.push(format!(
                        "Could not inspect {} as a Patchwork mod: {}",
                        manifest.path, parse_error
                    ));
                }
            }
        }
    }

    if base_path != "." && !manifests.iter().any(|manifest| manifest.scan_candidate) {
        warnings.push(format!(
            "No candidate manifests were found below {}.",
            display_path(base_path)
        ));
    }
    (candidates, warnings, errors)
}

fn push_modpack_candidate(
    candidates: &mut Vec<Candidate>,
    warnings: &mut Vec<String>,
    manifest: &ManifestSource,
    parsed: RegistryModpackManifest,
    directories: &BTreeMap<String, DirectorySnapshot>,
) {
    let directory_path = repository_parent(&manifest.path);
    let Some(directory) = directories.get(&directory_path) else {
        warnings.push(format!(
            "Could not identify the Git tree for {}.",
            manifest.path
        ));
        return;
    };
    let (readme_path, readme_blob_oid) =
        find_modpack_readme(&directory_path, directory, &parsed.id);
    let (image_path, image_blob_oid) = find_image(&directory_path, directory, &parsed.id);
    let dependencies = parsed
        .dependencies
        .iter()
        .map(|dependency| RegistryDependency {
            kind: if dependency.ignored {
                RegistryDependencyKind::Ignore
            } else {
                match dependency.target_kind {
                    RegistryDependencyTargetKind::Mod => RegistryDependencyKind::Mod,
                    RegistryDependencyTargetKind::Modpack => RegistryDependencyKind::Modpack,
                }
            },
            target_kind: registry_project_kind(dependency.target_kind),
            target_id: dependency.target_id.clone(),
            available: false,
        })
        .collect();
    let description = parsed.modpack.description.clone();
    let metadata = serde_json::to_value(&parsed.modpack).unwrap_or_else(|_| json!({}));
    candidates.push(Candidate {
        entry_id: Uuid::new_v4(),
        project_kind: RegistryProjectKind::Modpack,
        project_id: parsed.id,
        title: parsed.title,
        description,
        version: parsed.version,
        repository_path: directory_path,
        source_tree_oid: directory.tree_oid.clone(),
        manifest_path: manifest.path.clone(),
        manifest_blob_oid: manifest.blob_oid.clone(),
        manifest_sha256: sha256_hex(manifest.source.as_bytes()),
        readme_path,
        readme_blob_oid,
        image_path,
        image_blob_oid,
        status: RegistryScanStatus::NewMod,
        metadata,
        dependencies,
        warnings: Vec::new(),
        errors: Vec::new(),
    });
}

fn declares_patchwork_mod(source: &str) -> bool {
    toml::from_str::<toml::Value>(source)
        .ok()
        .and_then(|document| {
            document
                .get("package")?
                .get("metadata")?
                .get("mod")
                .cloned()
        })
        .is_some()
}

struct FallbackIdentity {
    mod_id: String,
    title: String,
    version: String,
}

struct FallbackModpackIdentity {
    project_id: String,
    title: String,
    version: String,
}

fn fallback_modpack_identity(manifest: &ManifestSource) -> Option<FallbackModpackIdentity> {
    let project_id = Path::new(&manifest.path).file_stem()?.to_str()?.to_owned();
    if !valid_mod_id(&project_id) {
        return None;
    }
    let document = toml::from_str::<toml::Value>(&manifest.source).ok()?;
    let table = document.as_table()?;
    if !["modpacks", "mods", "ignore"]
        .iter()
        .any(|key| table.contains_key(*key))
    {
        return None;
    }
    let title = table
        .get("name")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(&project_id)
        .to_owned();
    let version = table
        .get("version")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|version| Version::parse(version).is_ok())
        .unwrap_or("0.0.0")
        .to_owned();
    Some(FallbackModpackIdentity {
        project_id,
        title,
        version,
    })
}

fn fallback_identity(
    manifest: &ManifestSource,
    manifests: &[ManifestSource],
) -> Option<FallbackIdentity> {
    let document = toml::from_str::<toml::Value>(&manifest.source).ok()?;
    let package = document.get("package")?.as_table()?;
    package.get("metadata")?.get("mod")?;
    let mod_id = package.get("name")?.as_str()?.trim().to_owned();
    if !valid_mod_id(&mod_id) {
        return None;
    }
    let title = package
        .get("metadata")
        .and_then(|metadata| metadata.get("mod"))
        .and_then(|metadata| metadata.get("title"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|title| {
            !title.is_empty()
                && title.chars().count() <= 200
                && !title.chars().any(char::is_control)
        })
        .unwrap_or(&mod_id)
        .to_owned();
    let version = fallback_version(package, &manifest.path, manifests)?;
    if version.is_empty() || version.len() > 64 || version.chars().any(char::is_control) {
        return None;
    }
    Version::parse(&version).ok()?;
    Some(FallbackIdentity {
        mod_id,
        title,
        version,
    })
}

fn fallback_version(
    package: &toml::map::Map<String, toml::Value>,
    manifest_path: &str,
    manifests: &[ManifestSource],
) -> Option<String> {
    let version = package.get("version")?;
    if let Some(version) = version.as_str() {
        return Some(version.trim().to_owned());
    }
    if version
        .as_table()
        .and_then(|table| table.get("workspace"))
        .and_then(toml::Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let directory = repository_parent(manifest_path);
    let mut ancestors = manifests
        .iter()
        .filter(|candidate| is_ancestor_path(&repository_parent(&candidate.path), &directory))
        .collect::<Vec<_>>();
    ancestors.sort_by_key(|candidate| std::cmp::Reverse(path_depth(&candidate.path)));
    ancestors.into_iter().find_map(|candidate| {
        toml::from_str::<toml::Value>(&candidate.source)
            .ok()?
            .get("workspace")?
            .get("package")?
            .get("version")?
            .as_str()
            .map(|version| version.trim().to_owned())
    })
}

fn valid_mod_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn dependencies_from_mod_info(info: &patchwork::ModInfo) -> Vec<RegistryDependency> {
    let mut dependencies = Vec::new();
    for (kind, targets) in [
        (RegistryDependencyKind::Init, &info.dependencies.init),
        (RegistryDependencyKind::Run, &info.dependencies.run),
        (
            RegistryDependencyKind::Ownership,
            &info.dependencies.ownership,
        ),
    ] {
        for target_id in targets {
            let (target_kind, target_id) =
                parse_registry_dependency(target_id, Path::new("Cargo.toml"))
                    .expect("core registry validation already checked this dependency");
            dependencies.push(RegistryDependency {
                kind,
                target_kind: registry_project_kind(target_kind),
                target_id,
                available: false,
            });
        }
    }
    if let Some(target_id) = &info.provides {
        dependencies.push(RegistryDependency {
            kind: RegistryDependencyKind::Provides,
            target_kind: RegistryProjectKind::Mod,
            target_id: target_id.clone(),
            available: false,
        });
    }
    dependencies
}

fn registry_project_kind(kind: RegistryDependencyTargetKind) -> RegistryProjectKind {
    match kind {
        RegistryDependencyTargetKind::Mod => RegistryProjectKind::Mod,
        RegistryDependencyTargetKind::Modpack => RegistryProjectKind::Modpack,
    }
}

fn find_readme(
    directory_path: &str,
    directory: &DirectorySnapshot,
) -> (Option<String>, Option<String>) {
    for expected in ["readme.md", "readme.markdown", "readme"] {
        if let Some(entry) = directory
            .blobs
            .iter()
            .filter(|entry| entry.path.eq_ignore_ascii_case(expected))
            .min_by(|left, right| left.path.cmp(&right.path))
        {
            return (
                Some(join_repo_path(directory_path, &entry.path)),
                Some(entry.sha.clone()),
            );
        }
    }
    (None, None)
}

fn find_modpack_readme(
    directory_path: &str,
    directory: &DirectorySnapshot,
    modpack_id: &str,
) -> (Option<String>, Option<String>) {
    let expected = format!("{modpack_id}.md");
    directory
        .blobs
        .iter()
        .filter(|entry| entry.path.eq_ignore_ascii_case(&expected))
        .min_by(|left, right| left.path.cmp(&right.path))
        .map(|entry| {
            (
                Some(join_repo_path(directory_path, &entry.path)),
                Some(entry.sha.clone()),
            )
        })
        .unwrap_or((None, None))
}

fn find_image(
    directory_path: &str,
    directory: &DirectorySnapshot,
    mod_id: &str,
) -> (Option<String>, Option<String>) {
    for extension in ["png", "webp", "jpg", "jpeg"] {
        let expected = format!("{mod_id}.{extension}");
        if let Some(entry) = directory
            .blobs
            .iter()
            .filter(|entry| entry.path.eq_ignore_ascii_case(&expected))
            .min_by(|left, right| left.path.cmp(&right.path))
        {
            return (
                Some(join_repo_path(directory_path, &entry.path)),
                Some(entry.sha.clone()),
            );
        }
    }
    (None, None)
}

fn validate_candidates(
    database: &Database,
    publisher_uuid: Uuid,
    github_repository_id: i64,
    candidates: &mut [Candidate],
    reporter: Option<&ScanReporter>,
) -> Result<()> {
    let mut counts = HashMap::<(RegistryProjectKind, String), usize>::new();
    for candidate in candidates.iter() {
        *counts
            .entry((candidate.project_kind, candidate.project_id.clone()))
            .or_default() += 1;
    }
    for candidate in candidates.iter_mut() {
        if candidate.project_kind == RegistryProjectKind::Mod
            && is_generated_mod_id(&candidate.project_id)
        {
            candidate.status = RegistryScanStatus::Error;
            candidate.errors.push(format!(
                "Mod ID '{}' is reserved for build-generated crates and cannot be published.",
                candidate.project_id
            ));
        }
        if counts
            .get(&(candidate.project_kind, candidate.project_id.clone()))
            .copied()
            .unwrap_or_default()
            > 1
        {
            candidate.status = RegistryScanStatus::Error;
            candidate.errors.push(format!(
                "{} ID '{}' appears more than once in this scan.",
                project_kind_label(candidate.project_kind),
                candidate.project_id
            ));
        }
    }

    let locally_available = candidates
        .iter()
        .filter(|candidate| candidate.errors.is_empty())
        .map(|candidate| (candidate.project_kind, candidate.project_id.clone()))
        .collect::<HashSet<_>>();
    let dependency_ids = candidates
        .iter()
        .flat_map(|candidate| candidate.dependencies.iter())
        .map(|dependency| (dependency.target_kind, dependency.target_id.clone()))
        .filter(|(kind, id)| *kind != RegistryProjectKind::Mod || !is_generated_mod_id(id))
        .collect::<HashSet<_>>();
    let mut registry_available = HashSet::new();
    for dependency in dependency_ids {
        let exists = match dependency.0 {
            RegistryProjectKind::Mod => database
                .get_mod(&dependency.1)
                .map_err(database_http_error)?
                .is_some(),
            RegistryProjectKind::Modpack => database
                .get_modpack(&dependency.1)
                .map_err(database_http_error)?
                .is_some(),
        };
        if locally_available.contains(&dependency) || exists {
            registry_available.insert(dependency);
        }
    }

    let publisher = publisher_uuid.hyphenated().to_string();
    let total = candidates.len();
    for (index, candidate) in candidates.iter_mut().enumerate() {
        for dependency in &mut candidate.dependencies {
            if dependency.kind == RegistryDependencyKind::Ignore {
                dependency.available = true;
                continue;
            }
            if dependency.target_kind == RegistryProjectKind::Mod
                && is_generated_mod_id(&dependency.target_id)
            {
                dependency.available = false;
                continue;
            }
            dependency.available = registry_available
                .contains(&(dependency.target_kind, dependency.target_id.clone()));
            if !dependency.available {
                candidate.warnings.push(format!(
                    "Dependency '{}' ({}) is not currently published in this scan or registry.",
                    dependency.target_id,
                    dependency_kind_label(dependency.kind)
                ));
            }
        }
        if !candidate.errors.is_empty() {
            candidate.status = RegistryScanStatus::Error;
            if let Some(reporter) = reporter {
                reporter.entry(candidate.to_dto(), index + 1, total);
            }
            continue;
        }

        candidate.status = match candidate.project_kind {
            RegistryProjectKind::Mod => {
                let existing = database
                    .get_registry_mod_state(&candidate.project_id)
                    .map_err(database_http_error)?;
                classify_mod_candidate(
                    existing.as_ref(),
                    &publisher,
                    github_repository_id,
                    candidate,
                )
            }
            RegistryProjectKind::Modpack => {
                let existing = database
                    .get_registry_modpack_state(&candidate.project_id)
                    .map_err(database_http_error)?;
                classify_modpack_candidate(
                    existing.as_ref(),
                    &publisher,
                    github_repository_id,
                    candidate,
                )
            }
        };
        if let Some(reporter) = reporter {
            reporter.entry(candidate.to_dto(), index + 1, total);
        }
    }
    Ok(())
}

fn classify_mod_candidate(
    existing: Option<&RegistryModState>,
    publisher_uuid: &str,
    github_repository_id: i64,
    candidate: &mut Candidate,
) -> RegistryScanStatus {
    let Some(existing) = existing else {
        return RegistryScanStatus::NewMod;
    };
    if existing.mod_record.publisher_uuid != publisher_uuid
        || existing.repository.provider != "github"
        || existing.repository.provider_repository_id != github_repository_id
    {
        candidate.errors.push(format!(
            "Mod ID '{}' already belongs to another publisher or GitHub repository.",
            candidate.project_id
        ));
        return RegistryScanStatus::Error;
    }
    let Some(version) = existing
        .versions
        .iter()
        .find(|version| version.version == candidate.version)
    else {
        return RegistryScanStatus::NewVersion;
    };
    if version.source_tree_oid == candidate.source_tree_oid {
        RegistryScanStatus::Unchanged
    } else {
        candidate.errors.push(format!(
            "{} {} is already published with different content. Increase package.version in Cargo.toml (for example {} -> {}).",
            candidate.project_id,
            candidate.version,
            candidate.version,
            suggested_patch_version(&candidate.version)
        ));
        RegistryScanStatus::VersionConflict
    }
}

fn classify_modpack_candidate(
    existing: Option<&RegistryModpackState>,
    publisher_uuid: &str,
    github_repository_id: i64,
    candidate: &mut Candidate,
) -> RegistryScanStatus {
    let Some(existing) = existing else {
        return RegistryScanStatus::NewMod;
    };
    if existing.modpack_record.publisher_uuid != publisher_uuid
        || existing.repository.provider != "github"
        || existing.repository.provider_repository_id != github_repository_id
    {
        candidate.errors.push(format!(
            "Modpack ID '{}' already belongs to another publisher or GitHub repository.",
            candidate.project_id
        ));
        return RegistryScanStatus::Error;
    }
    let Some(version) = existing
        .versions
        .iter()
        .find(|version| version.version == candidate.version)
    else {
        return RegistryScanStatus::NewVersion;
    };
    if version.manifest_blob_oid == candidate.manifest_blob_oid
        && version.readme_blob_oid == candidate.readme_blob_oid
        && version.image_blob_oid == candidate.image_blob_oid
    {
        RegistryScanStatus::Unchanged
    } else {
        candidate.errors.push(format!(
            "{} {} is already published with different content. Increase version in {} (for example {} -> {}).",
            candidate.project_id,
            candidate.version,
            candidate.manifest_path,
            candidate.version,
            suggested_patch_version(&candidate.version)
        ));
        RegistryScanStatus::VersionConflict
    }
}

fn suggested_patch_version(version: &str) -> String {
    Version::parse(version)
        .map(|mut version| {
            version.patch = version.patch.saturating_add(1);
            version.pre = semver::Prerelease::EMPTY;
            version.build = semver::BuildMetadata::EMPTY;
            version.to_string()
        })
        .unwrap_or_else(|_| "the next semantic version".to_owned())
}

fn dependency_kind_label(kind: RegistryDependencyKind) -> &'static str {
    match kind {
        RegistryDependencyKind::Init => "init",
        RegistryDependencyKind::Run => "run",
        RegistryDependencyKind::Ownership => "ownership",
        RegistryDependencyKind::Provides => "provides",
        RegistryDependencyKind::Mod => "mod",
        RegistryDependencyKind::Modpack => "modpack",
        RegistryDependencyKind::Ignore => "ignore",
    }
}

fn project_kind_label(kind: RegistryProjectKind) -> &'static str {
    match kind {
        RegistryProjectKind::Mod => "Mod",
        RegistryProjectKind::Modpack => "Modpack",
    }
}

fn project_kind_to_database(kind: RegistryProjectKind) -> &'static str {
    match kind {
        RegistryProjectKind::Mod => "mod",
        RegistryProjectKind::Modpack => "modpack",
    }
}

fn project_kind_from_database(kind: &str) -> Result<RegistryProjectKind> {
    match kind {
        "mod" => Ok(RegistryProjectKind::Mod),
        "modpack" => Ok(RegistryProjectKind::Modpack),
        _ => Err(error::ErrorInternalServerError(
            "database contains an invalid registry project kind",
        )),
    }
}

fn status_to_database(status: RegistryScanStatus) -> &'static str {
    match status {
        RegistryScanStatus::NewMod => "new_mod",
        RegistryScanStatus::NewVersion => "new_version",
        RegistryScanStatus::Unchanged => "unchanged",
        RegistryScanStatus::VersionConflict => "version_conflict",
        RegistryScanStatus::Error => "error",
    }
}

fn status_from_database(status: &str) -> Result<RegistryScanStatus> {
    match status {
        "new_mod" => Ok(RegistryScanStatus::NewMod),
        "new_version" => Ok(RegistryScanStatus::NewVersion),
        "unchanged" => Ok(RegistryScanStatus::Unchanged),
        "version_conflict" => Ok(RegistryScanStatus::VersionConflict),
        "error" => Ok(RegistryScanStatus::Error),
        _ => Err(error::ErrorInternalServerError(
            "database contains an invalid registry scan status",
        )),
    }
}

fn scan_dto(stored: RegistryScanWithEntries) -> Result<RegistryScan> {
    let scan = stored.scan;
    let entries = stored
        .entries
        .into_iter()
        .map(|entry| {
            Ok(RegistryScanEntry {
                entry_id: entry.id,
                project_kind: project_kind_from_database(&entry.project_kind)?,
                project_id: entry.project_id,
                title: entry.title,
                description: entry.description,
                version: entry.version,
                repository_path: entry.repository_path,
                manifest_path: entry.manifest_path,
                source_tree_oid: entry.source_tree_oid,
                manifest_blob_oid: entry.manifest_blob_oid,
                manifest_sha256: entry.manifest_sha256,
                readme_path: entry.readme_path,
                readme_blob_oid: entry.readme_blob_oid,
                image_path: entry.image_path,
                image_blob_oid: entry.image_blob_oid,
                status: status_from_database(&entry.status)?,
                dependencies: parse_json(&entry.dependencies_json, "dependencies_json")?,
                warnings: parse_json(&entry.warnings_json, "warnings_json")?,
                errors: parse_json(&entry.errors_json, "errors_json")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(RegistryScan {
        scan_id: scan.id,
        repository: RegistryRepository {
            github_repository_id: scan.github_repository_id,
            owner: scan.repository_owner,
            name: scan.repository_name,
            canonical_url: scan.repository_url,
        },
        base_path: scan.base_path,
        requested_ref: scan.requested_ref,
        resolved_commit: scan.resolved_commit,
        created_at: scan
            .created_at
            .and_utc()
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        expires_at: scan
            .expires_at
            .and_utc()
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        published_at: scan.published_at.map(|timestamp| {
            timestamp
                .and_utc()
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        }),
        entries,
        warnings: parse_json(&scan.warnings_json, "warnings_json")?,
        errors: parse_json(&scan.errors_json, "errors_json")?,
    })
}

fn publish_dto(published: RegistryPublishResult) -> RegistryPublishResponse {
    RegistryPublishResponse {
        scan_id: published.scan_id,
        published: published
            .published
            .into_iter()
            .map(|version| RegistryPublishedVersion {
                project_kind: project_kind_from_database(&version.project_kind)
                    .expect("database publish only returns valid project kinds"),
                project_id: version.project_id,
                version: version.version,
                version_id: version.version_id,
            })
            .collect(),
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(value: &str, field: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|parse_error| {
        error::ErrorInternalServerError(format!("invalid stored {field}: {parse_error}"))
    })
}

fn json_string<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(error::ErrorInternalServerError)
}

fn require_account(database: &Database, request: &HttpRequest) -> Result<Account> {
    server_auth::authenticated_account(database, request)?
        .ok_or_else(|| error::ErrorUnauthorized("not authenticated"))
}

fn linked_github(
    database: &Database,
    account_uuid: Uuid,
) -> Result<patchwork_database::GithubAccount> {
    database
        .get_github_account(account_uuid)
        .map_err(database_http_error)?
        .ok_or_else(|| error::ErrorForbidden("connect a GitHub account before publishing"))
}

fn account_uuid(account: &Account) -> Result<Uuid> {
    parse_uuid("account_uuid", &account.uuid)
        .map_err(|_| error::ErrorInternalServerError("stored account UUID is invalid"))
}

fn parse_uuid(field: &str, value: &str) -> Result<Uuid> {
    Uuid::parse_str(value)
        .map_err(|_| error::ErrorBadRequest(format!("{field} must be a valid UUID")))
}

fn database_http_error(database_error: DatabaseError) -> actix_web::Error {
    match database_error {
        DatabaseError::Validation { .. } => error::ErrorBadRequest(database_error.to_string()),
        DatabaseError::Conflict { .. } => error::ErrorConflict(database_error.to_string()),
        DatabaseError::NotFound { .. } => error::ErrorNotFound(database_error.to_string()),
        other => error::ErrorInternalServerError(other.to_string()),
    }
}

fn repository_parent(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_else(|| ".".to_owned())
}

fn join_repo_path(parent: &str, child: &str) -> String {
    if parent == "." || parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}

fn is_ancestor_path(ancestor: &str, path: &str) -> bool {
    ancestor == "."
        || ancestor == path
        || path
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn display_path(path: &str) -> &str {
    if path == "." {
        "the repository root"
    } else {
        path
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_github_repository_urls() {
        let coordinates =
            normalize_repository_url("https://github.com/Patchwork-RS/example.git").unwrap();
        assert_eq!(coordinates.owner, "Patchwork-RS");
        assert_eq!(coordinates.name, "example");
        assert!(normalize_repository_url("https://github.com/a/b/tree/main").is_err());
        assert!(normalize_repository_url("http://github.com/a/b").is_err());
    }

    #[test]
    fn normalizes_repository_subdirectories() {
        assert_eq!(normalize_base_path("").unwrap(), ".");
        assert_eq!(
            normalize_base_path("/mods").unwrap_err().to_string(),
            "base path must be a relative repository path of at most 1024 bytes"
        );
        assert_eq!(normalize_base_path("./mods/api/").unwrap(), "mods/api");
        assert_eq!(
            normalize_base_path("modpacks/client.toml").unwrap(),
            "modpacks/client.toml"
        );
        assert!(normalize_base_path("mods/../secret").is_err());
    }

    #[test]
    fn version_conflict_suggests_a_patch_bump() {
        assert_eq!(suggested_patch_version("1.2.3"), "1.2.4");
        assert_eq!(suggested_patch_version("1.2.3-beta.1+build"), "1.2.4");
    }
}
