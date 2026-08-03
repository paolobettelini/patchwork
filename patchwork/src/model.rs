use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{PatchworkError, Result};

pub fn is_generated_mod_id(id: &str) -> bool {
    id.contains("generated")
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Modpack {
    pub version: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub modpacks: Vec<String>,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Dependencies {
    #[serde(default)]
    pub init: Vec<String>,
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub ownership: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModInfo {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub dependencies: Dependencies,
    #[serde(default)]
    pub provides: Option<String>,
    #[serde(default)]
    pub support: bool,
    #[serde(default)]
    pub api: bool,
    #[serde(default)]
    pub codegen: Vec<CodegenDeclaration>,
}

impl ModInfo {
    pub fn validate(&self, mod_name: &str, manifest_path: &Path) -> Result<()> {
        if self.support && self.api {
            return Err(PatchworkError::InvalidModMetadata {
                mod_name: mod_name.to_string(),
                manifest_path: manifest_path.to_path_buf(),
                reason: "support and api are mutually exclusive".to_string(),
            });
        }

        if self.support || self.api {
            let kind = if self.api { "API" } else { "support" };
            if self.entry.is_some() {
                return Err(PatchworkError::InvalidModMetadata {
                    mod_name: mod_name.to_string(),
                    manifest_path: manifest_path.to_path_buf(),
                    reason: format!("{kind} mods must not declare entry"),
                });
            }

            if self.provides.is_some() {
                return Err(PatchworkError::InvalidModMetadata {
                    mod_name: mod_name.to_string(),
                    manifest_path: manifest_path.to_path_buf(),
                    reason: format!(
                        "{kind} mods must not declare provides; use a normal lifecycle mod as the provider"
                    ),
                });
            }
        } else if self.entry.is_none() {
            return Err(PatchworkError::InvalidModMetadata {
                mod_name: mod_name.to_string(),
                manifest_path: manifest_path.to_path_buf(),
                reason: "normal mods must declare entry".to_string(),
            });
        }

        Ok(())
    }

    pub fn entry_type(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    pub fn has_lifecycle(&self) -> bool {
        !self.support && !self.api
    }
}

#[derive(Debug, Deserialize)]
pub struct CargoManifest {
    pub package: CargoPackage,
}

#[derive(Debug, Deserialize)]
pub struct CargoPackage {
    pub name: String,
    #[serde(default)]
    pub metadata: CargoPackageMetadata,
}

#[derive(Debug, Deserialize, Default)]
pub struct CargoPackageMetadata {
    #[serde(rename = "mod")]
    pub mod_info: Option<ModInfo>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CodegenDeclaration {
    #[serde(rename = "crate")]
    pub package: String,
    #[serde(default = "default_codegen_version")]
    pub version: String,
    pub generator: CodegenGenerator,
    #[serde(default)]
    pub dev_crate: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CodegenGenerator {
    #[serde(rename = "crate")]
    pub crate_name: String,
    #[serde(default = "default_codegen_command")]
    pub command: String,
}

fn default_codegen_version() -> String {
    "0.1.0".to_string()
}

fn default_codegen_command() -> String {
    "generate".to_string()
}
