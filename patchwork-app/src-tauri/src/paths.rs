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
    expand_env_vars_with(value, |name| std::env::var(name).ok())
}

fn expand_env_vars_with(
    value: &str,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> String {
    let mut dollar_expanded = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '$' {
            dollar_expanded.push(character);
            continue;
        }

        if chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                name.push(next);
            }
            if !closed {
                dollar_expanded.push_str("${");
                dollar_expanded.push_str(&name);
            } else if name.is_empty() {
                dollar_expanded.push_str("${}");
            } else if let Some(value) = lookup(&name) {
                dollar_expanded.push_str(&value);
            } else {
                dollar_expanded.push_str("${");
                dollar_expanded.push_str(&name);
                dollar_expanded.push('}');
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
            dollar_expanded.push('$');
        } else if let Some(value) = lookup(&name) {
            dollar_expanded.push_str(&value);
        } else {
            dollar_expanded.push('$');
            dollar_expanded.push_str(&name);
        }
    }

    let mut expanded = String::with_capacity(dollar_expanded.len());
    let mut remaining = dollar_expanded.as_str();
    while let Some(open) = remaining.find('%') {
        expanded.push_str(&remaining[..open]);
        let after_open = &remaining[open + 1..];
        let Some(close) = after_open.find('%') else {
            expanded.push_str(&remaining[open..]);
            remaining = "";
            break;
        };
        let name = &after_open[..close];
        let valid_name = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if valid_name {
            if let Some(value) = lookup(name) {
                expanded.push_str(&value);
            } else {
                expanded.push('%');
                expanded.push_str(name);
                expanded.push('%');
            }
        } else {
            expanded.push('%');
            expanded.push_str(name);
            expanded.push('%');
        }
        remaining = &after_open[close + 1..];
    }
    expanded.push_str(remaining);
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/example".to_owned()),
            "USER" => Some("example".to_owned()),
            "USERPROFILE" => Some(r"C:\Users\example".to_owned()),
            _ => None,
        }
    }

    #[test]
    fn expands_unix_and_windows_environment_syntax() {
        assert_eq!(
            expand_env_vars_with("$HOME/${USER}/%USERPROFILE%", lookup),
            r"/home/example/example/C:\Users\example"
        );
    }

    #[test]
    fn preserves_unknown_or_unclosed_variables() {
        assert_eq!(
            expand_env_vars_with("$UNKNOWN/${UNKNOWN}/%UNKNOWN%/${UNCLOSED", lookup),
            "$UNKNOWN/${UNKNOWN}/%UNKNOWN%/${UNCLOSED"
        );
    }
}

pub(crate) fn display_path(path: &Path) -> String {
    path.display().to_string()
}
