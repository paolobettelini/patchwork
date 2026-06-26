use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Modpack {
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

#[derive(Debug, Deserialize, Default)]
pub struct Dependencies {
    #[serde(default)]
    pub init: Vec<String>,
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub ownership: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModInfo {
    pub entry: String,
    pub dependencies: Dependencies,
    #[serde(default)]
    pub provides: Option<String>,
    #[serde(default)]
    pub codegen: Vec<CodegenDeclaration>,
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

#[derive(Debug, Deserialize, Clone)]
pub struct CodegenDeclaration {
    #[serde(rename = "crate")]
    pub package: String,
    #[serde(default = "default_codegen_version")]
    pub version: String,
    pub generator: CodegenGenerator,
    #[serde(default)]
    pub dev_crate: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
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
