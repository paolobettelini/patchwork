use base64::{Engine, engine::general_purpose};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub(crate) const ICON_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

const COLOR_PALETTE: [&str; 8] = [
    "#02a9a9", "#fd614e", "#6268c8", "#fdb22c", "#7df9ff", "#77ff8a", "#ff6bd6", "#ff8a1c",
];

pub(crate) fn read_icon_data_url(path: &Path) -> Result<String, io::Error> {
    let bytes = fs::read(path)?;
    let mime = mime_for_icon_path(path);
    Ok(format!(
        "data:{mime};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

pub(crate) fn matching_icon_for_modpack_file(path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(parent) = path.parent() else {
        return Ok(None);
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(None);
    };

    matching_icon_named(parent, stem)
}

pub(crate) fn matching_icon_named(parent: &Path, stem: &str) -> Result<Option<PathBuf>, String> {
    let mut matches = fs::read_dir(parent)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| {
            candidate
                .file_stem()
                .and_then(|file_stem| file_stem.to_str())
                == Some(stem)
                && supported_icon_extension(candidate).is_some()
        })
        .collect::<Vec<_>>();

    if matches.len() > 1 {
        matches.sort();
        let names = matches
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Multiple favicon files found for '{stem}': {names}. Keep exactly one."
        ));
    }

    Ok(matches.pop())
}

pub(crate) fn copy_icon_to_profile(
    source: &Path,
    profiles_dir: &Path,
    id: &str,
) -> Result<(), String> {
    let extension = supported_icon_extension(source)
        .ok_or_else(|| "Selected favicon must be png, jpg, jpeg, webp, or gif".to_string())?;
    remove_existing_icons(profiles_dir, id).map_err(|error| error.to_string())?;
    let destination = profiles_dir.join(format!("{id}.{extension}"));
    fs::copy(source, &destination).map_err(|error| {
        format!(
            "Failed to copy favicon '{}' to '{}': {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

pub(crate) fn remove_existing_icons(profiles_dir: &Path, id: &str) -> Result<(), io::Error> {
    for extension in ICON_EXTENSIONS {
        let path = profiles_dir.join(format!("{id}.{extension}"));
        if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub(crate) fn icon_version_for(path: &Path) -> Result<String, io::Error> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    Ok(format!("{}:{modified}", metadata.len()))
}

pub(crate) fn deterministic_color_for(id: &str) -> &'static str {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let index = hasher.finish() as usize % COLOR_PALETTE.len();
    COLOR_PALETTE[index]
}

pub(crate) fn fake_downloads_for(id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let value = 1.0 + (hasher.finish() % 24_900) as f32 / 1_000.0;
    format!("{value:.1}K")
}

fn supported_icon_extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    ICON_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}

fn mime_for_icon_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/png",
    }
}
