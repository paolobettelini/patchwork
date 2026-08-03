CREATE TABLE accounts (
    uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    nickname VARCHAR(16) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    email VARCHAR(254) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    password_hash VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT accounts_nickname_unique UNIQUE (nickname),
    CONSTRAINT accounts_email_unique UNIQUE (email),
    CONSTRAINT accounts_uuid_length CHECK (CHAR_LENGTH(uuid) = 36),
    CONSTRAINT accounts_nickname_length CHECK (CHAR_LENGTH(nickname) BETWEEN 1 AND 16)
) ENGINE=InnoDB;

CREATE TABLE pending_registrations (
    verification_id_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    code_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    email VARCHAR(254) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    nickname VARCHAR(16) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    password_hash VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL,
    attempts INT NOT NULL DEFAULT 0,
    CONSTRAINT pending_registrations_email_unique UNIQUE (email),
    CONSTRAINT pending_registrations_nickname_unique UNIQUE (nickname),
    CONSTRAINT pending_registrations_attempts_check CHECK (attempts BETWEEN 0 AND 5),
    INDEX pending_registrations_expires_at_idx (expires_at)
) ENGINE=InnoDB;

CREATE TABLE repositories (
    id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    provider VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    provider_repository_id BIGINT NOT NULL,
    owner VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    name VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    canonical_url VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT repositories_provider_check CHECK (provider = 'github'),
    CONSTRAINT repositories_provider_id_positive CHECK (provider_repository_id > 0),
    CONSTRAINT repositories_provider_id_unique UNIQUE (provider, provider_repository_id),
    INDEX repositories_owner_name_idx (owner, name)
) ENGINE=InnoDB;

CREATE TABLE mods (
    id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    publisher_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    repository_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_base_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    latest_version_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NULL,
    downloads BIGINT NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT mods_downloads_check CHECK (downloads >= 0),
    CONSTRAINT mods_publisher_fk FOREIGN KEY (publisher_uuid)
        REFERENCES accounts(uuid) ON DELETE RESTRICT,
    CONSTRAINT mods_repository_fk FOREIGN KEY (repository_id)
        REFERENCES repositories(id) ON DELETE RESTRICT,
    INDEX mods_publisher_idx (publisher_uuid),
    INDEX mods_repository_idx (repository_id),
    INDEX mods_created_at_idx (created_at)
) ENGINE=InnoDB;

CREATE TABLE mod_versions (
    id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    mod_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    title VARCHAR(200) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    repository_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    source_commit VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_tree_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    manifest_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    manifest_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    manifest_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    readme_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    readme_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    image_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    image_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    metadata_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    published_by CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    published_github_user_id BIGINT NOT NULL,
    published_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    yanked_at DATETIME NULL,
    CONSTRAINT mod_versions_mod_version_unique UNIQUE (mod_id, version),
    CONSTRAINT mod_versions_mod_fk FOREIGN KEY (mod_id)
        REFERENCES mods(id) ON DELETE RESTRICT,
    CONSTRAINT mod_versions_publisher_fk FOREIGN KEY (published_by)
        REFERENCES accounts(uuid) ON DELETE RESTRICT,
    CONSTRAINT mod_versions_github_user_positive CHECK (published_github_user_id > 0),
    INDEX mod_versions_mod_idx (mod_id, published_at),
    INDEX mod_versions_source_idx (source_commit, source_tree_oid)
) ENGINE=InnoDB;

CREATE TABLE mod_version_dependencies (
    version_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    relation_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    target_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    target_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    position INT NOT NULL,
    PRIMARY KEY (version_id, relation_kind, target_kind, target_id),
    CONSTRAINT mod_version_dependencies_version_fk FOREIGN KEY (version_id)
        REFERENCES mod_versions(id) ON DELETE CASCADE,
    CONSTRAINT mod_version_dependencies_kind_check
        CHECK (relation_kind IN ('init', 'run', 'ownership')),
    CONSTRAINT mod_version_dependencies_target_kind_check
        CHECK (target_kind IN ('mod', 'modpack')),
    CONSTRAINT mod_version_dependencies_position_check CHECK (position >= 0),
    CONSTRAINT mod_version_dependencies_position_unique
        UNIQUE (version_id, relation_kind, position),
    INDEX mod_version_dependencies_target_idx (target_kind, target_id)
) ENGINE=InnoDB;

CREATE TABLE registry_scans (
    id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    publisher_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    github_user_id BIGINT NOT NULL,
    github_repository_id BIGINT NOT NULL,
    repository_owner VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    repository_name VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    repository_url VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    base_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    requested_ref VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    resolved_commit VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    root_tree_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    warnings_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    errors_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL,
    published_at DATETIME NULL,
    CONSTRAINT registry_scans_publisher_fk FOREIGN KEY (publisher_uuid)
        REFERENCES accounts(uuid) ON DELETE CASCADE,
    CONSTRAINT registry_scans_github_user_positive CHECK (github_user_id > 0),
    CONSTRAINT registry_scans_github_repository_positive CHECK (github_repository_id > 0),
    INDEX registry_scans_publisher_idx (publisher_uuid, created_at),
    INDEX registry_scans_expires_at_idx (expires_at)
) ENGINE=InnoDB;

CREATE TABLE registry_scan_entries (
    id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    scan_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    project_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    project_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    title VARCHAR(200) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    description TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    repository_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    source_tree_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    manifest_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    manifest_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    manifest_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    readme_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    readme_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    image_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    image_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    status VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    metadata_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    dependencies_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    warnings_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    errors_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    CONSTRAINT registry_scan_entries_scan_fk FOREIGN KEY (scan_id)
        REFERENCES registry_scans(id) ON DELETE CASCADE,
    CONSTRAINT registry_scan_entries_status_check
        CHECK (status IN ('new_mod', 'new_version', 'unchanged', 'version_conflict', 'error')),
    CONSTRAINT registry_scan_entries_project_kind_check
        CHECK (project_kind IN ('mod', 'modpack')),
    INDEX registry_scan_entries_scan_idx (scan_id),
    INDEX registry_scan_entries_project_idx (project_kind, project_id, version)
) ENGINE=InnoDB;

CREATE TABLE modpacks (
    id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    publisher_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    repository_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_base_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    latest_version_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NULL,
    downloads BIGINT NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT modpacks_downloads_check CHECK (downloads >= 0),
    CONSTRAINT modpacks_publisher_fk FOREIGN KEY (publisher_uuid)
        REFERENCES accounts(uuid) ON DELETE RESTRICT,
    CONSTRAINT modpacks_repository_fk FOREIGN KEY (repository_id)
        REFERENCES repositories(id) ON DELETE RESTRICT,
    INDEX modpacks_publisher_idx (publisher_uuid),
    INDEX modpacks_created_at_idx (created_at),
    INDEX modpacks_repository_idx (repository_id)
) ENGINE=InnoDB;

CREATE TABLE modpack_versions (
    id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    modpack_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    title VARCHAR(200) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    description TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    repository_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    source_commit VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_tree_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    manifest_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    manifest_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    manifest_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    readme_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    readme_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    image_path VARCHAR(1024) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NULL,
    image_blob_oid VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    metadata_json TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    published_by CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    published_github_user_id BIGINT NOT NULL,
    published_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    yanked_at DATETIME NULL,
    CONSTRAINT modpack_versions_modpack_version_unique UNIQUE (modpack_id, version),
    CONSTRAINT modpack_versions_modpack_fk FOREIGN KEY (modpack_id)
        REFERENCES modpacks(id) ON DELETE RESTRICT,
    CONSTRAINT modpack_versions_publisher_fk FOREIGN KEY (published_by)
        REFERENCES accounts(uuid) ON DELETE RESTRICT,
    CONSTRAINT modpack_versions_github_user_positive CHECK (published_github_user_id > 0),
    INDEX modpack_versions_modpack_idx (modpack_id, published_at),
    INDEX modpack_versions_source_idx (source_commit, source_tree_oid)
) ENGINE=InnoDB;

CREATE TABLE modpack_version_dependencies (
    version_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    relation_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    target_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    target_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    position INT NOT NULL,
    PRIMARY KEY (version_id, relation_kind, target_kind, target_id),
    CONSTRAINT modpack_version_dependencies_version_fk FOREIGN KEY (version_id)
        REFERENCES modpack_versions(id) ON DELETE CASCADE,
    CONSTRAINT modpack_version_dependencies_kind_check
        CHECK (relation_kind IN ('mod', 'modpack', 'ignore')),
    CONSTRAINT modpack_version_dependencies_target_kind_check
        CHECK (target_kind IN ('mod', 'modpack')),
    CONSTRAINT modpack_version_dependencies_position_check CHECK (position >= 0),
    CONSTRAINT modpack_version_dependencies_position_unique UNIQUE (version_id, relation_kind, position),
    INDEX modpack_version_dependencies_target_idx (target_kind, target_id)
) ENGINE=InnoDB;

CREATE TABLE web_sessions (
    token_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    account_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL,
    CONSTRAINT web_sessions_account_fk FOREIGN KEY (account_uuid)
        REFERENCES accounts(uuid) ON DELETE CASCADE,
    INDEX web_sessions_account_idx (account_uuid),
    INDEX web_sessions_expires_at_idx (expires_at)
) ENGINE=InnoDB;

CREATE TABLE oauth_authorization_codes (
    code_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    account_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    client_id VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    redirect_uri VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    code_challenge VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL,
    used_at DATETIME NULL,
    CONSTRAINT oauth_authorization_codes_account_fk FOREIGN KEY (account_uuid)
        REFERENCES accounts(uuid) ON DELETE CASCADE,
    CONSTRAINT oauth_authorization_codes_client_length
        CHECK (CHAR_LENGTH(client_id) BETWEEN 1 AND 128),
    CONSTRAINT oauth_authorization_codes_redirect_length
        CHECK (CHAR_LENGTH(redirect_uri) BETWEEN 1 AND 2048),
    CONSTRAINT oauth_authorization_codes_challenge_length
        CHECK (CHAR_LENGTH(code_challenge) BETWEEN 43 AND 128),
    INDEX oauth_authorization_codes_account_idx (account_uuid),
    INDEX oauth_authorization_codes_expires_at_idx (expires_at)
) ENGINE=InnoDB;

CREATE TABLE app_tokens (
    token_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    account_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    label VARCHAR(128) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL,
    last_used_at DATETIME NULL,
    CONSTRAINT app_tokens_account_fk FOREIGN KEY (account_uuid)
        REFERENCES accounts(uuid) ON DELETE CASCADE,
    INDEX app_tokens_account_idx (account_uuid),
    INDEX app_tokens_expires_at_idx (expires_at)
) ENGINE=InnoDB;

CREATE TABLE github_accounts (
    account_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    github_user_id BIGINT NOT NULL UNIQUE,
    github_login VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NOT NULL,
    github_avatar_url VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    linked_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT github_accounts_account_fk FOREIGN KEY (account_uuid)
        REFERENCES accounts(uuid) ON DELETE CASCADE,
    CONSTRAINT github_accounts_user_id_positive CHECK (github_user_id > 0),
    INDEX github_accounts_login_idx (github_login)
) ENGINE=InnoDB;

CREATE TABLE github_oauth_states (
    state_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY,
    account_uuid CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    completion_url VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL,
    used_at DATETIME NULL,
    CONSTRAINT github_oauth_states_account_fk FOREIGN KEY (account_uuid)
        REFERENCES accounts(uuid) ON DELETE CASCADE,
    INDEX github_oauth_states_account_idx (account_uuid),
    INDEX github_oauth_states_expires_at_idx (expires_at)
) ENGINE=InnoDB;
