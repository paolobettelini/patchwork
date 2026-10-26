use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteCacheSettings {
    pub enabled: bool,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub mode: CacheMode,
    pub authentication: CacheAuthentication,
    pub key_prefix: String,
    pub local_cache: bool,
}

impl Default for RemoteCacheSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            bucket: String::new(),
            region: "auto".into(),
            mode: CacheMode::ReadOnly,
            authentication: CacheAuthentication::None,
            key_prefix: "patchwork/".into(),
            local_cache: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CacheMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CacheAuthentication {
    None,
    AccessKey,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCredentials {
    pub access_key: String,
    pub secret_key: String,
}
