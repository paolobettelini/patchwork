use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::models::{DependencyInput, DependencyKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModpackManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub modpacks: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub mods: Vec<String>,
}

impl ModpackManifest {
    pub fn parse(input: &str) -> Result<Self> {
        Ok(toml::from_str(input)?)
    }

    pub fn dependencies(&self) -> Vec<DependencyInput> {
        let mut dependencies =
            Vec::with_capacity(self.mods.len() + self.modpacks.len() + self.ignore.len());

        dependencies.extend(self.mods.iter().cloned().map(|target_id| DependencyInput {
            kind: DependencyKind::Mod,
            target_id,
        }));
        dependencies.extend(
            self.modpacks
                .iter()
                .cloned()
                .map(|target_id| DependencyInput {
                    kind: DependencyKind::Modpack,
                    target_id,
                }),
        );
        dependencies.extend(
            self.ignore
                .iter()
                .cloned()
                .map(|target_id| DependencyInput {
                    kind: DependencyKind::Ignore,
                    target_id,
                }),
        );

        dependencies
    }
}
