PRAGMA foreign_keys = ON;

CREATE TABLE accounts (
    uuid TEXT PRIMARY KEY NOT NULL CHECK (length(uuid) = 36),
    nickname TEXT NOT NULL COLLATE NOCASE CHECK (length(nickname) BETWEEN 1 AND 16),
    email TEXT NOT NULL COLLATE NOCASE CHECK (length(email) BETWEEN 3 AND 254),
    password_hash TEXT CHECK (password_hash IS NULL OR length(password_hash) BETWEEN 60 AND 255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT accounts_nickname_unique UNIQUE (nickname),
    CONSTRAINT accounts_email_unique UNIQUE (email)
);

CREATE TABLE pending_registrations (
    verification_id_hash TEXT PRIMARY KEY NOT NULL CHECK (length(verification_id_hash) = 64),
    code_hash TEXT NOT NULL CHECK (length(code_hash) = 64),
    email TEXT NOT NULL COLLATE NOCASE CHECK (length(email) BETWEEN 3 AND 254),
    nickname TEXT NOT NULL COLLATE NOCASE CHECK (length(nickname) BETWEEN 1 AND 16),
    password_hash TEXT NOT NULL CHECK (length(password_hash) BETWEEN 60 AND 255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    CONSTRAINT pending_registrations_email_unique UNIQUE (email),
    CONSTRAINT pending_registrations_nickname_unique UNIQUE (nickname)
);

CREATE INDEX pending_registrations_expires_at_idx ON pending_registrations(expires_at);

CREATE TABLE repositories (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    provider TEXT NOT NULL CHECK (provider = 'github'),
    provider_repository_id BIGINT NOT NULL CHECK (provider_repository_id > 0),
    owner TEXT NOT NULL CHECK (length(owner) BETWEEN 1 AND 255),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 255),
    canonical_url TEXT NOT NULL CHECK (length(canonical_url) BETWEEN 1 AND 2048),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT repositories_provider_id_unique UNIQUE (provider, provider_repository_id)
);

CREATE INDEX repositories_owner_name_idx ON repositories(owner, name);

CREATE TABLE mods (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) BETWEEN 1 AND 128),
    publisher_uuid TEXT NOT NULL,
    repository_id TEXT NOT NULL,
    source_base_path TEXT NOT NULL CHECK (length(source_base_path) BETWEEN 1 AND 1024),
    latest_version_id TEXT,
    downloads INTEGER NOT NULL DEFAULT 0 CHECK (downloads >= 0),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (publisher_uuid) REFERENCES accounts(uuid) ON DELETE RESTRICT,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE RESTRICT
);

CREATE INDEX mods_publisher_idx ON mods(publisher_uuid);
CREATE INDEX mods_repository_idx ON mods(repository_id);
CREATE INDEX mods_created_at_idx ON mods(created_at DESC);

CREATE TABLE mod_versions (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    mod_id TEXT NOT NULL,
    version TEXT NOT NULL CHECK (length(version) BETWEEN 1 AND 64),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    repository_path TEXT NOT NULL CHECK (length(repository_path) BETWEEN 1 AND 1024),
    source_commit TEXT NOT NULL CHECK (length(source_commit) BETWEEN 40 AND 64),
    source_tree_oid TEXT NOT NULL CHECK (length(source_tree_oid) BETWEEN 40 AND 64),
    manifest_path TEXT NOT NULL CHECK (length(manifest_path) BETWEEN 1 AND 1024),
    manifest_blob_oid TEXT NOT NULL CHECK (length(manifest_blob_oid) BETWEEN 40 AND 64),
    manifest_sha256 TEXT NOT NULL CHECK (length(manifest_sha256) = 64),
    readme_path TEXT CHECK (readme_path IS NULL OR length(readme_path) BETWEEN 1 AND 1024),
    readme_blob_oid TEXT CHECK (readme_blob_oid IS NULL OR length(readme_blob_oid) BETWEEN 40 AND 64),
    image_path TEXT CHECK (image_path IS NULL OR length(image_path) BETWEEN 1 AND 1024),
    image_blob_oid TEXT CHECK (image_blob_oid IS NULL OR length(image_blob_oid) BETWEEN 40 AND 64),
    metadata_json TEXT NOT NULL,
    published_by TEXT NOT NULL,
    published_github_user_id BIGINT NOT NULL CHECK (published_github_user_id > 0),
    published_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    yanked_at TIMESTAMP,
    CONSTRAINT mod_versions_mod_version_unique UNIQUE (mod_id, version),
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE RESTRICT,
    FOREIGN KEY (published_by) REFERENCES accounts(uuid) ON DELETE RESTRICT
);

CREATE INDEX mod_versions_mod_idx ON mod_versions(mod_id, published_at DESC);
CREATE INDEX mod_versions_source_idx ON mod_versions(source_commit, source_tree_oid);

CREATE TABLE mod_version_dependencies (
    version_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL CHECK (relation_kind IN ('init', 'run', 'ownership')),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('mod', 'modpack')),
    target_id TEXT NOT NULL CHECK (length(target_id) BETWEEN 1 AND 128),
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (version_id, relation_kind, target_kind, target_id),
    FOREIGN KEY (version_id) REFERENCES mod_versions(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX mod_version_dependencies_position_unique
    ON mod_version_dependencies(version_id, relation_kind, position);
CREATE INDEX mod_version_dependencies_target_idx
    ON mod_version_dependencies(target_kind, target_id);

CREATE TABLE registry_scans (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    publisher_uuid TEXT NOT NULL,
    github_user_id BIGINT NOT NULL CHECK (github_user_id > 0),
    github_repository_id BIGINT NOT NULL CHECK (github_repository_id > 0),
    repository_owner TEXT NOT NULL CHECK (length(repository_owner) BETWEEN 1 AND 255),
    repository_name TEXT NOT NULL CHECK (length(repository_name) BETWEEN 1 AND 255),
    repository_url TEXT NOT NULL CHECK (length(repository_url) BETWEEN 1 AND 2048),
    base_path TEXT NOT NULL CHECK (length(base_path) BETWEEN 1 AND 1024),
    requested_ref TEXT NOT NULL CHECK (length(requested_ref) BETWEEN 1 AND 255),
    resolved_commit TEXT NOT NULL CHECK (length(resolved_commit) BETWEEN 40 AND 64),
    root_tree_oid TEXT NOT NULL CHECK (length(root_tree_oid) BETWEEN 40 AND 64),
    warnings_json TEXT NOT NULL DEFAULT '[]',
    errors_json TEXT NOT NULL DEFAULT '[]',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    published_at TIMESTAMP,
    FOREIGN KEY (publisher_uuid) REFERENCES accounts(uuid) ON DELETE CASCADE
);

CREATE INDEX registry_scans_publisher_idx ON registry_scans(publisher_uuid, created_at DESC);
CREATE INDEX registry_scans_expires_at_idx ON registry_scans(expires_at);

CREATE TABLE registry_scan_entries (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    scan_id TEXT NOT NULL,
    project_kind TEXT NOT NULL CHECK (project_kind IN ('mod', 'modpack')),
    project_id TEXT NOT NULL CHECK (length(project_id) BETWEEN 1 AND 128),
    version TEXT NOT NULL CHECK (length(version) BETWEEN 1 AND 64),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 4000),
    repository_path TEXT NOT NULL CHECK (length(repository_path) BETWEEN 1 AND 1024),
    source_tree_oid TEXT NOT NULL CHECK (length(source_tree_oid) BETWEEN 40 AND 64),
    manifest_path TEXT NOT NULL CHECK (length(manifest_path) BETWEEN 1 AND 1024),
    manifest_blob_oid TEXT NOT NULL CHECK (length(manifest_blob_oid) BETWEEN 40 AND 64),
    manifest_sha256 TEXT NOT NULL CHECK (length(manifest_sha256) = 64),
    readme_path TEXT CHECK (readme_path IS NULL OR length(readme_path) BETWEEN 1 AND 1024),
    readme_blob_oid TEXT CHECK (readme_blob_oid IS NULL OR length(readme_blob_oid) BETWEEN 40 AND 64),
    image_path TEXT CHECK (image_path IS NULL OR length(image_path) BETWEEN 1 AND 1024),
    image_blob_oid TEXT CHECK (image_blob_oid IS NULL OR length(image_blob_oid) BETWEEN 40 AND 64),
    status TEXT NOT NULL CHECK (status IN ('new_mod', 'new_version', 'unchanged', 'version_conflict', 'error')),
    metadata_json TEXT NOT NULL,
    dependencies_json TEXT NOT NULL DEFAULT '[]',
    warnings_json TEXT NOT NULL DEFAULT '[]',
    errors_json TEXT NOT NULL DEFAULT '[]',
    FOREIGN KEY (scan_id) REFERENCES registry_scans(id) ON DELETE CASCADE
);

CREATE INDEX registry_scan_entries_scan_idx ON registry_scan_entries(scan_id);
CREATE INDEX registry_scan_entries_project_idx
    ON registry_scan_entries(project_kind, project_id, version);

CREATE TABLE modpacks (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) BETWEEN 1 AND 128),
    publisher_uuid TEXT NOT NULL,
    repository_id TEXT NOT NULL,
    source_base_path TEXT NOT NULL CHECK (length(source_base_path) BETWEEN 1 AND 1024),
    latest_version_id TEXT,
    downloads INTEGER NOT NULL DEFAULT 0 CHECK (downloads >= 0),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (publisher_uuid) REFERENCES accounts(uuid) ON DELETE RESTRICT,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE RESTRICT
);

CREATE INDEX modpacks_publisher_idx ON modpacks(publisher_uuid);
CREATE INDEX modpacks_repository_idx ON modpacks(repository_id);
CREATE INDEX modpacks_created_at_idx ON modpacks(created_at DESC);

CREATE TABLE modpack_versions (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    modpack_id TEXT NOT NULL,
    version TEXT NOT NULL CHECK (length(version) BETWEEN 1 AND 64),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL CHECK (length(description) <= 4000),
    repository_path TEXT NOT NULL CHECK (length(repository_path) BETWEEN 1 AND 1024),
    source_commit TEXT NOT NULL CHECK (length(source_commit) BETWEEN 40 AND 64),
    source_tree_oid TEXT NOT NULL CHECK (length(source_tree_oid) BETWEEN 40 AND 64),
    manifest_path TEXT NOT NULL CHECK (length(manifest_path) BETWEEN 1 AND 1024),
    manifest_blob_oid TEXT NOT NULL CHECK (length(manifest_blob_oid) BETWEEN 40 AND 64),
    manifest_sha256 TEXT NOT NULL CHECK (length(manifest_sha256) = 64),
    readme_path TEXT CHECK (readme_path IS NULL OR length(readme_path) BETWEEN 1 AND 1024),
    readme_blob_oid TEXT CHECK (readme_blob_oid IS NULL OR length(readme_blob_oid) BETWEEN 40 AND 64),
    image_path TEXT CHECK (image_path IS NULL OR length(image_path) BETWEEN 1 AND 1024),
    image_blob_oid TEXT CHECK (image_blob_oid IS NULL OR length(image_blob_oid) BETWEEN 40 AND 64),
    metadata_json TEXT NOT NULL,
    published_by TEXT NOT NULL,
    published_github_user_id BIGINT NOT NULL CHECK (published_github_user_id > 0),
    published_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    yanked_at TIMESTAMP,
    CONSTRAINT modpack_versions_modpack_version_unique UNIQUE (modpack_id, version),
    FOREIGN KEY (modpack_id) REFERENCES modpacks(id) ON DELETE RESTRICT,
    FOREIGN KEY (published_by) REFERENCES accounts(uuid) ON DELETE RESTRICT
);

CREATE INDEX modpack_versions_modpack_idx ON modpack_versions(modpack_id, published_at DESC);
CREATE INDEX modpack_versions_source_idx ON modpack_versions(source_commit, source_tree_oid);

CREATE TABLE modpack_version_dependencies (
    version_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL CHECK (relation_kind IN ('mod', 'modpack', 'ignore')),
    target_kind TEXT NOT NULL CHECK (target_kind IN ('mod', 'modpack')),
    target_id TEXT NOT NULL CHECK (length(target_id) BETWEEN 1 AND 128),
    position INTEGER NOT NULL CHECK (position >= 0),
    PRIMARY KEY (version_id, relation_kind, target_kind, target_id),
    FOREIGN KEY (version_id) REFERENCES modpack_versions(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX modpack_version_dependencies_position_unique
    ON modpack_version_dependencies(version_id, relation_kind, position);
CREATE INDEX modpack_version_dependencies_target_idx
    ON modpack_version_dependencies(target_kind, target_id);

CREATE TABLE web_sessions (
    token_hash TEXT PRIMARY KEY NOT NULL CHECK (length(token_hash) = 64),
    account_uuid TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    FOREIGN KEY (account_uuid) REFERENCES accounts(uuid) ON DELETE CASCADE
);

CREATE INDEX web_sessions_account_idx ON web_sessions(account_uuid);
CREATE INDEX web_sessions_expires_at_idx ON web_sessions(expires_at);

CREATE TABLE oauth_authorization_codes (
    code_hash TEXT PRIMARY KEY NOT NULL CHECK (length(code_hash) = 64),
    account_uuid TEXT NOT NULL,
    client_id TEXT NOT NULL CHECK (length(client_id) BETWEEN 1 AND 128),
    redirect_uri TEXT NOT NULL CHECK (length(redirect_uri) BETWEEN 1 AND 2048),
    code_challenge TEXT NOT NULL CHECK (length(code_challenge) BETWEEN 43 AND 128),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    used_at TIMESTAMP,
    FOREIGN KEY (account_uuid) REFERENCES accounts(uuid) ON DELETE CASCADE
);

CREATE INDEX oauth_authorization_codes_account_idx ON oauth_authorization_codes(account_uuid);
CREATE INDEX oauth_authorization_codes_expires_at_idx ON oauth_authorization_codes(expires_at);

CREATE TABLE app_tokens (
    token_hash TEXT PRIMARY KEY NOT NULL CHECK (length(token_hash) = 64),
    account_uuid TEXT NOT NULL,
    label TEXT CHECK (label IS NULL OR length(label) <= 128),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    last_used_at TIMESTAMP,
    FOREIGN KEY (account_uuid) REFERENCES accounts(uuid) ON DELETE CASCADE
);

CREATE INDEX app_tokens_account_idx ON app_tokens(account_uuid);
CREATE INDEX app_tokens_expires_at_idx ON app_tokens(expires_at);

CREATE TABLE github_accounts (
    account_uuid TEXT PRIMARY KEY NOT NULL,
    github_user_id BIGINT NOT NULL UNIQUE CHECK (github_user_id > 0),
    github_login TEXT NOT NULL CHECK (length(github_login) BETWEEN 1 AND 255),
    github_avatar_url TEXT NOT NULL CHECK (length(github_avatar_url) BETWEEN 1 AND 2048),
    linked_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (account_uuid) REFERENCES accounts(uuid) ON DELETE CASCADE
);

CREATE INDEX github_accounts_login_idx ON github_accounts(github_login);

CREATE TABLE github_oauth_states (
    state_hash TEXT PRIMARY KEY NOT NULL CHECK (length(state_hash) = 64),
    account_uuid TEXT NOT NULL,
    completion_url TEXT NOT NULL CHECK (length(completion_url) BETWEEN 1 AND 2048),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    used_at TIMESTAMP,
    FOREIGN KEY (account_uuid) REFERENCES accounts(uuid) ON DELETE CASCADE
);

CREATE INDEX github_oauth_states_account_idx ON github_oauth_states(account_uuid);
CREATE INDEX github_oauth_states_expires_at_idx ON github_oauth_states(expires_at);
