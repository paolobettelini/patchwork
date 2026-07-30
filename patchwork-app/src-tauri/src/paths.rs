use std::path::{Path, PathBuf};

pub(crate) fn default_patchwork_data_dir(tauri_data_dir: &Path) -> PathBuf {
    tauri_data_dir
        .parent()
        .map(|parent| parent.join("patchwork"))
        .unwrap_or_else(|| tauri_data_dir.join("patchwork"))
}

pub(crate) fn distinct_dependency_count(modpacks: &[String], mods: &[String]) -> usize {
    let mut dependencies = modpacks
        .iter()
        .chain(mods.iter())
        .map(|dependency| dependency.trim())
        .filter(|dependency| !dependency.is_empty())
        .collect::<Vec<_>>();
    dependencies.sort_unstable();
    dependencies.dedup();
    dependencies.len()
}

pub(crate) fn non_empty_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn is_valid_hex_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color
            .chars()
            .skip(1)
            .all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn slugify_modpack_id(name: &str) -> String {
    let mut id = String::new();
    let mut previous_dash = false;

    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            id.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if matches!(character, '-' | '_' | ' ' | '.') && !previous_dash && !id.is_empty() {
            id.push('-');
            previous_dash = true;
        }
    }

    while id.ends_with('-') {
        id.pop();
    }
    id
}

pub(crate) fn sanitize_existing_modpack_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("Modpack id cannot be empty".to_string());
    }
    if trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        Ok(trimmed.to_string())
    } else {
        Err(format!("Invalid modpack id '{trimmed}'"))
    }
}

pub(crate) fn sanitize_build_mode(mode: &str) -> Result<String, String> {
    let mode = mode.trim();
    if matches!(mode, "release" | "debug") {
        Ok(mode.to_string())
    } else {
        Err(format!("Unknown build mode '{mode}'"))
    }
}

pub(crate) fn expand_env_vars(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '$' {
            expanded.push(character);
            continue;
        }

        if chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                name.push(next);
            }
            if name.is_empty() {
                expanded.push_str("${}");
            } else if let Ok(value) = std::env::var(&name) {
                expanded.push_str(&value);
            } else {
                expanded.push_str("${");
                expanded.push_str(&name);
                expanded.push('}');
            }
            continue;
        }

        let mut name = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_ascii_alphanumeric() || next == '_' {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }

        if name.is_empty() {
            expanded.push('$');
        } else if let Ok(value) = std::env::var(&name) {
            expanded.push_str(&value);
        } else {
            expanded.push('$');
            expanded.push_str(&name);
        }
    }

    expanded
}

pub(crate) fn display_path(path: &Path) -> String {
    path.display().to_string()
}
