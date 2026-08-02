// This schema is intentionally limited to SQL types shared by SQLite and MySQL.

diesel::table! {
    accounts (uuid) {
        uuid -> Text,
        nickname -> Text,
        email -> Text,
        password_hash -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    pending_registrations (verification_id_hash) {
        verification_id_hash -> Text,
        code_hash -> Text,
        email -> Text,
        nickname -> Text,
        password_hash -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        attempts -> Integer,
    }
}

diesel::table! {
    repositories (id) {
        id -> Text,
        provider -> Text,
        provider_repository_id -> BigInt,
        owner -> Text,
        name -> Text,
        canonical_url -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    mods (id) {
        id -> Text,
        publisher_uuid -> Text,
        repository_id -> Text,
        source_base_path -> Text,
        latest_version_id -> Nullable<Text>,
        downloads -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    mod_versions (id) {
        id -> Text,
        mod_id -> Text,
        version -> Text,
        title -> Text,
        repository_path -> Text,
        source_commit -> Text,
        source_tree_oid -> Text,
        manifest_path -> Text,
        manifest_blob_oid -> Text,
        manifest_sha256 -> Text,
        readme_path -> Nullable<Text>,
        readme_blob_oid -> Nullable<Text>,
        image_path -> Nullable<Text>,
        image_blob_oid -> Nullable<Text>,
        metadata_json -> Text,
        published_by -> Text,
        published_github_user_id -> BigInt,
        published_at -> Timestamp,
        yanked_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    mod_version_dependencies (version_id, relation_kind, target_id) {
        version_id -> Text,
        relation_kind -> Text,
        target_id -> Text,
        position -> Integer,
    }
}

diesel::table! {
    registry_scans (id) {
        id -> Text,
        publisher_uuid -> Text,
        github_user_id -> BigInt,
        github_repository_id -> BigInt,
        repository_owner -> Text,
        repository_name -> Text,
        repository_url -> Text,
        base_path -> Text,
        requested_ref -> Text,
        resolved_commit -> Text,
        root_tree_oid -> Text,
        warnings_json -> Text,
        errors_json -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        published_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    registry_scan_entries (id) {
        id -> Text,
        scan_id -> Text,
        mod_id -> Text,
        version -> Text,
        title -> Text,
        repository_path -> Text,
        source_tree_oid -> Text,
        manifest_path -> Text,
        manifest_blob_oid -> Text,
        manifest_sha256 -> Text,
        readme_path -> Nullable<Text>,
        readme_blob_oid -> Nullable<Text>,
        image_path -> Nullable<Text>,
        image_blob_oid -> Nullable<Text>,
        status -> Text,
        metadata_json -> Text,
        dependencies_json -> Text,
        warnings_json -> Text,
        errors_json -> Text,
    }
}

diesel::table! {
    modpacks (id) {
        id -> Text,
        title -> Text,
        description -> Text,
        published_at -> Timestamp,
        downloads -> BigInt,
        publisher_uuid -> Text,
        repository_url -> Text,
        manifest_path -> Text,
        source_ref -> Text,
        logo_url -> Nullable<Text>,
    }
}

diesel::table! {
    modpack_dependencies (modpack_id, relation_kind, target_id) {
        modpack_id -> Text,
        relation_kind -> Text,
        target_id -> Text,
        position -> Integer,
    }
}

diesel::table! {
    web_sessions (token_hash) {
        token_hash -> Text,
        account_uuid -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
    }
}

diesel::table! {
    oauth_authorization_codes (code_hash) {
        code_hash -> Text,
        account_uuid -> Text,
        client_id -> Text,
        redirect_uri -> Text,
        code_challenge -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        used_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    app_tokens (token_hash) {
        token_hash -> Text,
        account_uuid -> Text,
        label -> Nullable<Text>,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        last_used_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    github_accounts (account_uuid) {
        account_uuid -> Text,
        github_user_id -> BigInt,
        github_login -> Text,
        github_avatar_url -> Text,
        linked_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    github_oauth_states (state_hash) {
        state_hash -> Text,
        account_uuid -> Text,
        completion_url -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        used_at -> Nullable<Timestamp>,
    }
}

diesel::joinable!(mods -> accounts (publisher_uuid));
diesel::joinable!(mods -> repositories (repository_id));
diesel::joinable!(mod_versions -> mods (mod_id));
diesel::joinable!(mod_version_dependencies -> mod_versions (version_id));
diesel::joinable!(registry_scans -> accounts (publisher_uuid));
diesel::joinable!(registry_scan_entries -> registry_scans (scan_id));
diesel::joinable!(modpacks -> accounts (publisher_uuid));
diesel::joinable!(modpack_dependencies -> modpacks (modpack_id));
diesel::joinable!(web_sessions -> accounts (account_uuid));
diesel::joinable!(oauth_authorization_codes -> accounts (account_uuid));
diesel::joinable!(app_tokens -> accounts (account_uuid));
diesel::joinable!(github_accounts -> accounts (account_uuid));
diesel::joinable!(github_oauth_states -> accounts (account_uuid));

diesel::allow_tables_to_appear_in_same_query!(
    accounts,
    repositories,
    mods,
    mod_versions,
    mod_version_dependencies,
    registry_scans,
    registry_scan_entries,
    modpacks,
    modpack_dependencies,
    web_sessions,
    oauth_authorization_codes,
    app_tokens,
    github_accounts,
    github_oauth_states,
    pending_registrations,
);
