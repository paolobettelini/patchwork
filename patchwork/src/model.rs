use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    #[serde(default, skip_serializing_if = "ProfileOptions::is_empty")]
    pub options: ProfileOptions,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct ProfileOptions {
    #[serde(default, skip_serializing_if = "ProcessOptions::is_empty")]
    pub build: ProcessOptions,
    #[serde(default, skip_serializing_if = "ProcessOptions::is_empty")]
    pub run: ProcessOptions,
}

impl ProfileOptions {
    pub fn is_empty(&self) -> bool {
        self.build.is_empty() && self.run.is_empty()
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        validate_process_options("build", &self.build, BUILD_RESERVED_ENV)?;
        validate_process_options("run", &self.run, RUN_RESERVED_ENV)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct ProcessOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl ProcessOptions {
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.env.is_empty()
    }

    pub fn expanded_args(&self) -> std::result::Result<Vec<String>, String> {
        let mut expanded = Vec::new();
        for fragment in &self.args {
            expanded.extend(parse_argument_fragment(fragment)?);
        }
        Ok(expanded)
    }
}

const BUILD_RESERVED_ENV: &[&str] = &["TERM", "COLORTERM", "CARGO_TERM_COLOR", "CARGO_TARGET_DIR"];
const RUN_RESERVED_ENV: &[&str] = &[
    "TERM",
    "COLORTERM",
    "BACKEND_ADDR",
    "PATCHWORK_AUTH_FD",
    "PATCHWORK_AUTH_PIPE_VERSION",
];

fn validate_process_options(
    label: &str,
    options: &ProcessOptions,
    reserved_env: &[&str],
) -> std::result::Result<(), String> {
    if options.args.len() > 128 || options.env.len() > 128 {
        return Err(format!(
            "profile {label} options may contain at most 128 arguments and 128 environment variables"
        ));
    }
    for argument in &options.args {
        if argument.len() > 4096 || argument.contains('\0') {
            return Err(format!("a profile {label} argument is invalid or too long"));
        }
    }
    options
        .expanded_args()
        .map_err(|reason| format!("invalid profile {label} arguments: {reason}"))?;
    for (name, value) in &options.env {
        let valid_name = !name.is_empty()
            && name.len() <= 256
            && name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_'
                    || byte.is_ascii_alphanumeric() && (index > 0 || !byte.is_ascii_digit())
            });
        if !valid_name {
            return Err(format!("'{name}' is not a valid environment variable name"));
        }
        if value.len() > 8192 || value.contains('\0') {
            return Err(format!(
                "the value of profile {label} variable '{name}' is invalid or too long"
            ));
        }
        if reserved_env
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(name))
        {
            return Err(format!(
                "profile {label} variable '{name}' is managed by Patchwork and cannot be overridden"
            ));
        }
    }
    Ok(())
}

fn parse_argument_fragment(fragment: &str) -> std::result::Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut token_started = false;

    for character in fragment.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            token_started = true;
            continue;
        }

        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    current.push(character);
                }
            }
            Some('"') => match character {
                '"' => quote = None,
                '\\' => escaped = true,
                _ => current.push(character),
            },
            _ => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    token_started = true;
                }
                '\\' => {
                    escaped = true;
                    token_started = true;
                }
                character if character.is_whitespace() => {
                    if token_started {
                        arguments.push(std::mem::take(&mut current));
                        token_started = false;
                    }
                }
                _ => {
                    current.push(character);
                    token_started = true;
                }
            },
        }
    }

    if escaped {
        return Err("argument ends with an incomplete escape".to_owned());
    }
    if let Some(quote) = quote {
        return Err(format!("argument contains an unclosed {quote} quote"));
    }
    if token_started {
        arguments.push(current);
    }
    Ok(arguments)
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
