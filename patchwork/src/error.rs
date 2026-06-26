use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, PatchworkError>;

#[derive(Debug)]
pub enum PatchworkError {
    Io {
        action: &'static str,
        path: Option<PathBuf>,
        source: io::Error,
    },
    Toml {
        document: &'static str,
        path: PathBuf,
        source: toml::de::Error,
    },
    InvalidUtf8Path {
        context: &'static str,
        path: PathBuf,
    },
    InvalidModpackId {
        id: String,
        reason: &'static str,
    },
    ModpackNotFound {
        id: String,
        folder: PathBuf,
    },
    ModpackCycle {
        cycle: Vec<PathBuf>,
    },
    InvalidModName {
        name: String,
        context: &'static str,
    },
    MissingModManifest {
        mod_name: String,
        manifest_path: PathBuf,
    },
    MissingModMetadata {
        mod_name: String,
        manifest_path: PathBuf,
    },
    DuplicateProvider {
        api: String,
        first_provider: String,
        second_provider: String,
    },
    OwnershipConflict {
        message: String,
    },
    MissingDependency {
        dependent_mod: String,
        dependency: String,
    },
    SelfDependency {
        mod_name: String,
    },
    ModDependencyCycle {
        unresolved_mods: Vec<String>,
    },
    InvalidCrateName {
        name: String,
        reason: &'static str,
    },
    DuplicateGeneratedCrate {
        package: String,
    },
    MissingCodegenGenerator {
        generator_crate: String,
        manifest_path: PathBuf,
    },
    CodegenFailed {
        package: String,
        status: String,
    },
    MissingProjectName {
        modpack_path: PathBuf,
    },
    UnsupportedAssetEntry {
        path: PathBuf,
    },
}

impl PatchworkError {
    pub fn kind(&self) -> &'static str {
        match self {
            PatchworkError::Io { .. } => "io",
            PatchworkError::Toml { .. } => "toml",
            PatchworkError::InvalidUtf8Path { .. } => "invalid_utf8_path",
            PatchworkError::InvalidModpackId { .. } => "invalid_modpack_id",
            PatchworkError::ModpackNotFound { .. } => "modpack_not_found",
            PatchworkError::ModpackCycle { .. } => "modpack_cycle",
            PatchworkError::InvalidModName { .. } => "invalid_mod_name",
            PatchworkError::MissingModManifest { .. } => "missing_mod_manifest",
            PatchworkError::MissingModMetadata { .. } => "missing_mod_metadata",
            PatchworkError::DuplicateProvider { .. } => "duplicate_provider",
            PatchworkError::OwnershipConflict { .. } => "ownership_conflict",
            PatchworkError::MissingDependency { .. } => "missing_dependency",
            PatchworkError::SelfDependency { .. } => "self_dependency",
            PatchworkError::ModDependencyCycle { .. } => "mod_dependency_cycle",
            PatchworkError::InvalidCrateName { .. } => "invalid_crate_name",
            PatchworkError::DuplicateGeneratedCrate { .. } => "duplicate_generated_crate",
            PatchworkError::MissingCodegenGenerator { .. } => "missing_codegen_generator",
            PatchworkError::CodegenFailed { .. } => "codegen_failed",
            PatchworkError::MissingProjectName { .. } => "missing_project_name",
            PatchworkError::UnsupportedAssetEntry { .. } => "unsupported_asset_entry",
        }
    }

    pub fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        PatchworkError::Io {
            action,
            path: Some(path.into()),
            source,
        }
    }

    pub fn io_without_path(action: &'static str, source: io::Error) -> Self {
        PatchworkError::Io {
            action,
            path: None,
            source,
        }
    }

    pub fn parse_toml(
        document: &'static str,
        path: impl Into<PathBuf>,
        source: toml::de::Error,
    ) -> Self {
        PatchworkError::Toml {
            document,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for PatchworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchworkError::Io {
                action,
                path,
                source,
            } => {
                if let Some(path) = path {
                    write!(f, "{action} '{}': {source}", path.display())
                } else {
                    write!(f, "{action}: {source}")
                }
            }
            PatchworkError::Toml {
                document,
                path,
                source,
            } => write!(f, "invalid {document} TOML '{}': {source}", path.display()),
            PatchworkError::InvalidUtf8Path { context, path } => write!(
                f,
                "{context} path must be valid UTF-8: '{}'",
                path.display()
            ),
            PatchworkError::InvalidModpackId { id, reason } => {
                write!(f, "invalid modpack id '{id}': {reason}")
            }
            PatchworkError::ModpackNotFound { id, folder } => {
                write!(f, "modpack '{id}' not found in {}", folder.display())
            }
            PatchworkError::ModpackCycle { cycle } => {
                let cycle = cycle
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                write!(f, "circular modpack import detected: {cycle}")
            }
            PatchworkError::InvalidModName { name, context } => {
                write!(
                    f,
                    "invalid mod name '{name}' in {context}: path separators are not allowed"
                )
            }
            PatchworkError::MissingModManifest {
                mod_name,
                manifest_path,
            } => write!(
                f,
                "Cargo.toml not found for mod '{mod_name}': {}",
                manifest_path.display()
            ),
            PatchworkError::MissingModMetadata {
                mod_name,
                manifest_path,
            } => write!(
                f,
                "missing [package.metadata.mod] for mod '{mod_name}': {}",
                manifest_path.display()
            ),
            PatchworkError::DuplicateProvider {
                api,
                first_provider,
                second_provider,
            } => write!(
                f,
                "multiple providers for API '{api}': '{first_provider}' and '{second_provider}'"
            ),
            PatchworkError::OwnershipConflict { message } => write!(f, "{message}"),
            PatchworkError::MissingDependency {
                dependent_mod,
                dependency,
            } => write!(
                f,
                "dependency '{dependency}' required by mod '{dependent_mod}' is not satisfied (no selected mod and no selected provider)"
            ),
            PatchworkError::SelfDependency { mod_name } => {
                write!(f, "mod '{mod_name}' depends on itself")
            }
            PatchworkError::ModDependencyCycle { unresolved_mods } => write!(
                f,
                "circular dependency detected among mods; unresolved mods: {}",
                unresolved_mods.join(", ")
            ),
            PatchworkError::InvalidCrateName { name, reason } => {
                write!(f, "invalid crate name '{name}': {reason}")
            }
            PatchworkError::DuplicateGeneratedCrate { package } => {
                write!(f, "generated crate '{package}' is declared more than once")
            }
            PatchworkError::MissingCodegenGenerator {
                generator_crate,
                manifest_path,
            } => write!(
                f,
                "codegen generator crate '{generator_crate}' not found at {}",
                manifest_path.display()
            ),
            PatchworkError::CodegenFailed { package, status } => {
                write!(
                    f,
                    "codegen for generated crate '{package}' failed with status {status}"
                )
            }
            PatchworkError::MissingProjectName { modpack_path } => write!(
                f,
                "could not derive project name from modpack path '{}'",
                modpack_path.display()
            ),
            PatchworkError::UnsupportedAssetEntry { path } => write!(
                f,
                "unsupported asset entry '{}': only files and directories are supported",
                path.display()
            ),
        }
    }
}

impl Error for PatchworkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PatchworkError::Io { source, .. } => Some(source),
            PatchworkError::Toml { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for PatchworkError {
    fn from(source: io::Error) -> Self {
        PatchworkError::io_without_path("filesystem operation failed", source)
    }
}
