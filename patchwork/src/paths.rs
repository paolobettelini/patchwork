use crate::error::{PatchworkError, Result};
use std::env;
use std::path::{Path, PathBuf};

pub fn absolutize(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()
            .map_err(|source| PatchworkError::io_without_path("read current directory", source))?
            .join(path))
    }
}

pub fn relative_path_for_manifest(project_dir: &Path, path: &Path) -> PathBuf {
    let Ok(path) = path.canonicalize() else {
        return path.to_path_buf();
    };

    if let Some(parent) = project_dir.parent() {
        if let Ok(relative_to_parent) = path.strip_prefix(parent) {
            return PathBuf::from("..").join(relative_to_parent);
        }
    }

    path
}

pub fn path_to_toml_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn crate_dir(base: &Path, name: &str) -> Result<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        return Err(PatchworkError::InvalidCrateName {
            name: name.to_string(),
            reason: "path separators are not allowed",
        });
    }

    Ok(base.join(name))
}
