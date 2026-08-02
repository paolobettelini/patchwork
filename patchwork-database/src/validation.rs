use std::path::{Component, Path};

use url::Url;

use crate::error::{DatabaseError, Result};

pub(crate) fn normalize_nickname(value: &str) -> Result<String> {
    let value = value.trim();
    let length = value.chars().count();
    if !(1..=16).contains(&length) {
        return validation("nickname", "must contain between 1 and 16 characters");
    }
    if value.chars().any(char::is_control) {
        return validation("nickname", "must not contain control characters");
    }
    Ok(value.to_owned())
}

pub(crate) fn normalize_email(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() || value.len() > 254 {
        return validation("email", "must contain between 1 and 254 bytes");
    }
    if value.chars().any(char::is_whitespace) {
        return validation("email", "must not contain whitespace");
    }
    let mut parts = value.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if local.is_empty() || domain.is_empty() || parts.next().is_some() || !domain.contains('.') {
        return validation("email", "must look like a valid email address");
    }
    Ok(value)
}

pub(crate) fn normalize_password_hash(value: &str) -> Result<String> {
    let value = value.trim();
    if !(60..=255).contains(&value.len()) {
        return validation("password_hash", "must contain between 60 and 255 bytes");
    }
    if value.chars().any(char::is_control) {
        return validation("password_hash", "must not contain control characters");
    }
    Ok(value.to_owned())
}

pub(crate) fn normalize_sha256_hex(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        validation(field, "must be a SHA-256 hex digest")
    }
}

pub(crate) fn normalize_package_id(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return validation(field, "must contain between 1 and 128 ASCII characters");
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
    }) {
        return validation(
            field,
            "must be a lowercase slug containing only a-z, 0-9, '-', '_' or '.'",
        );
    }
    if !value.as_bytes()[0].is_ascii_alphanumeric() {
        return validation(field, "must start with a letter or digit");
    }
    Ok(value.to_owned())
}

pub(crate) fn normalize_title(value: &str) -> Result<String> {
    normalize_bounded_text("title", value, 200, false, false)
}

pub(crate) fn normalize_description(value: &str) -> Result<String> {
    normalize_bounded_text("description", value, 4_000, true, true)
}

pub(crate) fn normalize_source_ref(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 255 {
        return validation("source_ref", "must contain between 1 and 255 characters");
    }
    if value.chars().any(char::is_control) {
        return validation("source_ref", "must not contain control characters");
    }
    Ok(value.to_owned())
}

pub(crate) fn normalize_repository_url(value: &str) -> Result<String> {
    let mut parsed = Url::parse(value.trim()).map_err(|error| DatabaseError::Validation {
        field: "repository_url",
        message: error.to_string(),
    })?;

    if parsed.scheme() != "https" {
        return validation("repository_url", "must use HTTPS");
    }
    if !matches!(
        parsed.host_str(),
        Some("github.com") | Some("www.github.com")
    ) {
        return validation("repository_url", "must point to github.com");
    }
    parsed.set_query(None);
    parsed.set_fragment(None);

    let segments = parsed
        .path_segments()
        .map(|segments| {
            segments
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if segments.len() != 2 {
        return validation(
            "repository_url",
            "must identify a repository, for example https://github.com/owner/repository",
        );
    }

    let normalized_path = format!(
        "/{}/{}",
        segments[0],
        segments[1].strip_suffix(".git").unwrap_or(&segments[1])
    );
    parsed.set_path(&normalized_path);
    Ok(parsed.to_string().trim_end_matches('/').to_owned())
}

pub(crate) fn normalize_https_url(field: &'static str, value: &str) -> Result<String> {
    let mut parsed = Url::parse(value.trim()).map_err(|error| DatabaseError::Validation {
        field,
        message: error.to_string(),
    })?;
    if parsed.scheme() != "https" {
        return validation(field, "must use HTTPS");
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

pub(crate) fn normalize_repo_path(
    field: &'static str,
    value: &str,
    allow_repository_root: bool,
) -> Result<String> {
    let value = value.trim().replace('\\', "/");
    if value.is_empty() || value == "." {
        if allow_repository_root {
            return Ok(".".to_owned());
        }
        return validation(field, "must point to a file inside the repository");
    }
    if value.len() > 1_024 || value.starts_with('/') {
        return validation(
            field,
            "must be a relative repository path of at most 1024 bytes",
        );
    }

    let path = Path::new(&value);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return validation(field, "must not contain '..' or an absolute path");
    }

    let normalized = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");

    if normalized.is_empty() {
        if allow_repository_root {
            Ok(".".to_owned())
        } else {
            validation(field, "must point to a file inside the repository")
        }
    } else {
        Ok(normalized)
    }
}

fn normalize_bounded_text(
    field: &'static str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
    allow_multiline: bool,
) -> Result<String> {
    let value = value.trim();
    let length = value.chars().count();
    if (!allow_empty && length == 0) || length > max_chars {
        let minimum = if allow_empty { 0 } else { 1 };
        return validation(
            field,
            format!("must contain between {minimum} and {max_chars} characters"),
        );
    }
    let has_disallowed_control = value.chars().any(|character| {
        character.is_control() && !(allow_multiline && matches!(character, '\n' | '\r' | '\t'))
    });
    if has_disallowed_control {
        let message = if allow_multiline {
            "must not contain control characters other than newlines or tabs"
        } else {
            "must not contain control characters"
        };
        return validation(field, message);
    }
    Ok(value.to_owned())
}

fn validation<T>(field: &'static str, message: impl Into<String>) -> Result<T> {
    Err(DatabaseError::Validation {
        field,
        message: message.into(),
    })
}
