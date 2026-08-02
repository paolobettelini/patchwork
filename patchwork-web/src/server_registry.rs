use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use actix_web::{HttpRequest, HttpResponse, Result, error, web};
use chrono::{Duration, SecondsFormat, Utc};
use futures_util::{StreamExt, stream};
use patchwork::{RegistryWorkspaceManifest, parse_registry_mod_manifest};
use patchwork_database::{
    Account, CreateRegistryScan, CreateRegistryScanEntry, Database, DatabaseError,
    RegistryModState, RegistryPublishResult, RegistryScanWithEntries,
};
use patchwork_registry_types::{
    RegistryDependency, RegistryDependencyKind, RegistryPublishRequest, RegistryPublishResponse,
    RegistryPublishedVersion, RegistryRepository, RegistryScan, RegistryScanEntry,
    RegistryScanJobStarted, RegistryScanPhase, RegistryScanProgress, RegistryScanRequest,
    RegistryScanStatus,
};
use semver::Version;
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

#[derive(Clone)]
pub(crate) struct RegistryState {
    database: Database,
    github: GithubClient,
    jobs: Arc<Mutex<HashMap<Uuid, StoredScanJob>>>,
}

impl RegistryState {
    pub(crate) fn new(database: Database, github: GithubClient) -> Self {
        Self {
            database,
            github,
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
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
            progress.phase = RegistryScanPhase::ValidatingMods;
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
        );
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
            "No Patchwork mods were found below {}.",
            display_path(&base_path)
        ));
    }

    report_phase(
        reporter,
        RegistryScanPhase::ValidatingMods,
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
            "subdirectory must be a relative repository path of at most 1024 bytes",
        ));
    }
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(error::ErrorBadRequest("subdirectory must not contain '..'"));
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        Ok(".".to_owned())
    } else if parts.len() > MAX_DIRECTORY_DEPTH {
        Err(error::ErrorBadRequest(format!(
            "subdirectory must contain at most {MAX_DIRECTORY_DEPTH} path components"
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
        for component in base_path.split('/') {
            remember_manifest(&mut manifest_map, &current_path, &current_tree, false);
            let child = current_tree
                .entries
                .iter()
                .find(|entry| entry.path == component && entry.kind == "tree")
                .ok_or_else(|| {
                    format!(
                        "subdirectory '{}' does not exist at commit {}",
                        base_path, commit_tree_oid
                    )
                })?;
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

        if file_name == "Cargo.toml" {
            let path = join_repo_path(&directory_path, "Cargo.toml");
            manifest_map.insert(
                path.clone(),
                ManifestReference {
                    path,
                    blob_oid: entry.sha,
                    size: entry.size,
                    scan_candidate: true,
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
            "scan found {scan_manifests} Cargo.toml files; the limit is {MAX_MANIFESTS}"
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
}

struct Candidate {
    entry_id: Uuid,
    mod_id: String,
    title: String,
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
            mod_id: self.mod_id.clone(),
            title: self.title.clone(),
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
            mod_id: self.mod_id,
            version: self.version,
            title: self.title,
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
                    mod_id: parsed.id,
                    title: parsed.title,
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
                            mod_id: identity.mod_id,
                            title: identity.title,
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
            "No Cargo.toml files were found below {}.",
            display_path(base_path)
        ));
    }
    (candidates, warnings, errors)
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
            dependencies.push(RegistryDependency {
                kind,
                target_id: target_id.clone(),
                available: false,
            });
        }
    }
    dependencies
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
    let mut counts = HashMap::<String, usize>::new();
    for candidate in candidates.iter() {
        *counts.entry(candidate.mod_id.clone()).or_default() += 1;
    }
    for candidate in candidates.iter_mut() {
        if counts.get(&candidate.mod_id).copied().unwrap_or_default() > 1 {
            candidate.status = RegistryScanStatus::Error;
            candidate.errors.push(format!(
                "Mod ID '{}' appears more than once in this scan.",
                candidate.mod_id
            ));
        }
    }

    let locally_available = candidates
        .iter()
        .filter(|candidate| candidate.errors.is_empty())
        .map(|candidate| candidate.mod_id.clone())
        .collect::<HashSet<_>>();
    let dependency_ids = candidates
        .iter()
        .flat_map(|candidate| candidate.dependencies.iter())
        .map(|dependency| dependency.target_id.clone())
        .collect::<HashSet<_>>();
    let mut registry_available = HashSet::new();
    for dependency_id in dependency_ids {
        if locally_available.contains(&dependency_id)
            || database
                .get_mod(&dependency_id)
                .map_err(database_http_error)?
                .is_some()
        {
            registry_available.insert(dependency_id);
        }
    }

    let publisher = publisher_uuid.hyphenated().to_string();
    let total = candidates.len();
    for (index, candidate) in candidates.iter_mut().enumerate() {
        for dependency in &mut candidate.dependencies {
            dependency.available = registry_available.contains(&dependency.target_id);
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

        let existing = database
            .get_registry_mod_state(&candidate.mod_id)
            .map_err(database_http_error)?;
        candidate.status = classify_candidate(
            existing.as_ref(),
            &publisher,
            github_repository_id,
            candidate,
        );
        if let Some(reporter) = reporter {
            reporter.entry(candidate.to_dto(), index + 1, total);
        }
    }
    Ok(())
}

fn classify_candidate(
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
            candidate.mod_id
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
            candidate.mod_id,
            candidate.version,
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
                mod_id: entry.mod_id,
                title: entry.title,
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
                mod_id: version.mod_id,
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
            "subdirectory must be a relative repository path of at most 1024 bytes"
        );
        assert_eq!(normalize_base_path("./mods/api/").unwrap(), "mods/api");
        assert!(normalize_base_path("mods/../secret").is_err());
    }

    #[test]
    fn version_conflict_suggests_a_patch_bump() {
        assert_eq!(suggested_patch_version("1.2.3"), "1.2.4");
        assert_eq!(suggested_patch_version("1.2.3-beta.1+build"), "1.2.4");
    }
}
