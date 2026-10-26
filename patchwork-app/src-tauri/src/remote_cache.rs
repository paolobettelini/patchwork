use crate::{
    cache_settings::{CacheAuthentication, CacheCredentials, CacheMode, RemoteCacheSettings},
    model::{AppState, LauncherSettings},
};
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use tauri::{Manager, State};

const WRAPPER: &str = "PATCHWORK_SCCACHE_WRAPPER";
const PROBE: &str = "PATCHWORK_SCCACHE_PROBE";
pub(crate) type CargoEnvironment = BTreeMap<OsString, Option<OsString>>;

pub fn wrapper_requested() -> bool {
    env::var_os(WRAPPER).is_some()
}

pub fn wrapper_main() {
    if env::var_os(PROBE).is_some() {
        let result = tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())
            .and_then(|rt| {
                rt.block_on(async {
                    tokio::time::timeout(Duration::from_secs(20), probe())
                        .await
                        .map_err(|_| "Connection test timed out".to_owned())?
                })
            });
        match result {
            Ok(message) => println!("{message}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
    } else {
        sccache::main();
    }
}

async fn probe() -> Result<String, String> {
    let operator = sccache::cache::s3::S3Cache::new(
        env::var("SCCACHE_BUCKET").unwrap_or_default(),
        env::var("SCCACHE_S3_KEY_PREFIX").unwrap_or_default(),
        env::var("SCCACHE_S3_NO_CREDENTIALS").as_deref() == Ok("true"),
    )
    .with_endpoint(env::var("SCCACHE_ENDPOINT").ok())
    .with_region(env::var("SCCACHE_REGION").ok())
    .build()
    .map_err(|e| e.to_string())?;
    // Use the same object read as sccache, without requiring ListBucket permission.
    match operator.read(".sccache_check").await {
        Ok(_) => (),
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => (),
        Err(error) => return Err(format!("S3 read failed: {error}")),
    }
    if env::var("SCCACHE_S3_RW_MODE").as_deref() == Ok("READ_ONLY") {
        return Ok("S3 read request accepted (an absent check object is allowed). Write access was not tested.".into());
    }
    let key = format!(".patchwork-probe-{:032x}", rand::random::<u128>());
    let payload = format!("patchwork-cache-probe-{:032x}", rand::random::<u128>());
    operator
        .write(&key, payload.clone())
        .await
        .map_err(|e| format!("S3 write failed: {e}"))?;
    let read = operator.read(&key).await;
    let cleanup = operator.delete(&key).await;
    let bytes = read
        .map_err(|e| format!("S3 read-back failed: {e}"))?
        .to_vec();
    if bytes != payload.as_bytes() {
        return Err("S3 read-back did not match the written object".into());
    }
    cleanup.map_err(|e| {
        format!("Read/write succeeded, but deleting probe object {key} failed: {e}")
    })?;
    Ok("S3 write, read-back and cleanup succeeded.".into())
}

fn validate(config: &RemoteCacheSettings) -> Result<(), String> {
    let url = url::Url::parse(config.endpoint.trim())
        .map_err(|_| "Enter an HTTP or HTTPS S3 endpoint")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Endpoint must be an HTTP(S) URL without credentials, query or fragment".into(),
        );
    }
    if config.bucket.trim().is_empty()
        || config.bucket.contains('/')
        || config.region.trim().is_empty()
    {
        return Err("Bucket and region are required; bucket must be a name, not a path".into());
    }
    if config.authentication == CacheAuthentication::None && config.mode == CacheMode::ReadWrite {
        return Err("Read-write cache requires access key authentication".into());
    }
    Ok(())
}

fn credentials_path(state: &AppState) -> PathBuf {
    state
        .auth_path
        .parent()
        .expect("auth config directory")
        .join("remote-cache-credentials.json")
}

fn read_credentials(path: &Path) -> Result<CacheCredentials, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid remote cache credentials file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CacheCredentials::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn require_credentials(
    config: &RemoteCacheSettings,
    credentials: &CacheCredentials,
) -> Result<(), String> {
    if config.authentication == CacheAuthentication::AccessKey
        && (credentials.access_key.is_empty() || credentials.secret_key.is_empty())
    {
        return Err("Access key and secret key are required".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn remote_cache_credentials_status(state: State<AppState>) -> Result<bool, String> {
    let credentials = read_credentials(&credentials_path(&state))?;
    Ok(!credentials.access_key.is_empty() && !credentials.secret_key.is_empty())
}

#[tauri::command]
pub(crate) fn save_remote_cache(
    state: State<AppState>,
    config: RemoteCacheSettings,
    credentials: Option<CacheCredentials>,
) -> Result<LauncherSettings, String> {
    let path = credentials_path(&state);
    let stored = credentials
        .as_ref()
        .cloned()
        .unwrap_or(read_credentials(&path)?);
    if config.enabled {
        validate(&config)?;
        require_credentials(&config, &stored)?;
    }
    if config.authentication == CacheAuthentication::None {
        match fs::remove_file(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    } else if credentials.is_some() {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&stored).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned")?;
    let mut updated = settings.clone();
    updated.remote_cache = config;
    let settings_path = state
        .settings_path
        .lock()
        .map_err(|_| "settings path lock poisoned")?;
    crate::save_settings(&settings_path, &updated).map_err(|e| e.to_string())?;
    *settings = updated.clone();
    Ok(updated)
}

#[tauri::command]
pub(crate) async fn test_remote_cache(
    state: State<'_, AppState>,
    config: RemoteCacheSettings,
    credentials: Option<CacheCredentials>,
) -> Result<String, String> {
    validate(&config)?;
    let credentials = credentials.unwrap_or(read_credentials(&credentials_path(&state))?);
    require_credentials(&config, &credentials)?;
    let settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let environment = cache_environment(&settings, &config)?;
        let mut command = cache_command(&environment)?;
        add_credentials(&mut command, &config, &credentials);
        command.env(PROBE, "1");
        let output = command.output().map_err(|e| e.to_string())?;
        let text = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).into_owned()
        } else {
            String::from_utf8_lossy(&output.stderr).into_owned()
        };
        let text = redact(text, &credentials);
        if output.status.success() {
            Ok(text.trim().to_owned())
        } else {
            Err(text.trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn redact(mut message: String, credentials: &CacheCredentials) -> String {
    for secret in [&credentials.access_key, &credentials.secret_key] {
        if !secret.is_empty() {
            message = message.replace(secret, "[redacted]");
        }
    }
    message
}

fn cache_environment(
    settings: &LauncherSettings,
    config: &RemoteCacheSettings,
) -> Result<CargoEnvironment, String> {
    let cache = Path::new(&settings.build_cache)
        .parent()
        .ok_or("Build cache has no parent")?
        .join("sccache");
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let cache = cache.canonicalize().map_err(|e| e.to_string())?;
    let config_path = cache.join("config.toml");
    fs::write(&config_path, "").map_err(|e| e.to_string())?;
    let mut values = CargoEnvironment::new();
    for name in [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_PROFILE",
    ] {
        values.insert(name.into(), None);
    }
    for (key, _) in env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("SCCACHE_") || name.starts_with("AWS_") || name == PROBE {
            values.insert(key, None);
        }
    }
    let mut set = |key: &str, value: OsString| {
        values.insert(key.into(), Some(value));
    };
    set(WRAPPER, "1".into());
    set("RUSTC_WRAPPER", wrapper_executable(&cache)?.into());
    set("RUSTC_WORKSPACE_WRAPPER", "".into());
    set("CARGO_INCREMENTAL", "0".into());
    set("SCCACHE_CONF", config_path.into());
    set(
        "SCCACHE_CACHED_CONF",
        cache.join("cached-config.toml").into(),
    );
    set("SCCACHE_DIR", cache.join("objects").into());
    set("SCCACHE_BUCKET", config.bucket.trim().into());
    set("SCCACHE_ENDPOINT", config.endpoint.trim().into());
    set("SCCACHE_REGION", config.region.trim().into());
    set(
        "SCCACHE_S3_USE_SSL",
        if url::Url::parse(config.endpoint.trim())
            .map_err(|e| e.to_string())?
            .scheme()
            == "https"
        {
            "true"
        } else {
            "false"
        }
        .into(),
    );
    set("SCCACHE_S3_KEY_PREFIX", config.key_prefix.clone().into());
    set(
        "SCCACHE_S3_RW_MODE",
        if config.mode == CacheMode::ReadOnly {
            "READ_ONLY"
        } else {
            "READ_WRITE"
        }
        .into(),
    );
    set(
        "SCCACHE_S3_NO_CREDENTIALS",
        if config.authentication == CacheAuthentication::None {
            "true"
        } else {
            "false"
        }
        .into(),
    );
    set(
        "SCCACHE_MULTILEVEL_CHAIN",
        if config.local_cache { "disk,s3" } else { "s3" }.into(),
    );
    set("SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY", "ignore".into());
    set("SCCACHE_LOCAL_RW_MODE", "READ_WRITE".into());
    set("SCCACHE_IGNORE_SERVER_IO_ERROR", "1".into());
    set("SCCACHE_IDLE_TIMEOUT", "300".into());
    // Upstream only normalizes supported hash inputs; Rust still hashes its cwd.
    let mut roots = vec![
        PathBuf::from(&settings.mod_cache),
        PathBuf::from(&settings.build_cache),
        PathBuf::from(&settings.cargo_target_dir),
    ];
    roots.extend(settings.local_folders.iter().map(PathBuf::from));
    if let Some(home) = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::user_home_dir().map(|p| p.join(".cargo")))
    {
        roots.push(home);
    }
    let mut roots: Vec<_> = roots
        .into_iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    roots.sort();
    roots.dedup();
    set(
        "SCCACHE_BASEDIRS",
        env::join_paths(roots).map_err(|e| e.to_string())?,
    );
    Ok(values)
}

fn cache_command(environment: &CargoEnvironment) -> Result<Command, String> {
    let executable = environment
        .get(&OsString::from("RUSTC_WRAPPER"))
        .and_then(Option::as_ref)
        .ok_or("Missing embedded wrapper executable")?;
    let mut command = Command::new(executable);
    for (key, value) in environment {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok(command)
}

fn wrapper_executable(cache: &Path) -> Result<PathBuf, String> {
    // Upstream treats non-sccache executable names as compiler aliases.
    // This is the launcher binary itself, not an external sccache installation.
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let metadata = fs::metadata(&executable).map_err(|e| e.to_string())?;
    let modified = metadata
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = cache
        .join("wrapper")
        .join(format!("{modified}-{}", metadata.len()));
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let destination = directory.join(format!("sccache{}", env::consts::EXE_SUFFIX));
    if !destination.exists() && fs::hard_link(&executable, &destination).is_err() {
        fs::copy(&executable, &destination).map_err(|e| e.to_string())?;
    }
    Ok(destination)
}

fn add_credentials(
    command: &mut Command,
    config: &RemoteCacheSettings,
    credentials: &CacheCredentials,
) {
    if config.authentication == CacheAuthentication::AccessKey {
        command
            .env("AWS_ACCESS_KEY_ID", &credentials.access_key)
            .env("AWS_SECRET_ACCESS_KEY", &credentials.secret_key);
    }
}

pub(crate) struct CacheSession {
    pub environment: CargoEnvironment,
}

impl CacheSession {
    pub(crate) fn start(
        app: &tauri::AppHandle,
        settings: &LauncherSettings,
    ) -> Result<Option<Self>, String> {
        if !settings.remote_cache.enabled {
            return Ok(None);
        }
        validate(&settings.remote_cache)?;
        let credentials = read_credentials(&credentials_path(&app.state::<AppState>()))?;
        require_credentials(&settings.remote_cache, &credentials)?;
        let mut environment = cache_environment(settings, &settings.remote_cache)?;
        let socket = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let port = socket.local_addr().map_err(|e| e.to_string())?.port();
        environment.insert("SCCACHE_SERVER_PORT".into(), Some(port.to_string().into()));
        drop(socket);
        let mut command = cache_command(&environment)?;
        add_credentials(&mut command, &settings.remote_cache, &credentials);
        let output = command
            .arg("--start-server")
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(redact(
                format!(
                    "sccache startup failed: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ),
                &credentials,
            ));
        }
        Ok(Some(Self { environment }))
    }
}

impl Drop for CacheSession {
    fn drop(&mut self) {
        if let Ok(mut command) = cache_command(&self.environment) {
            let _ = command
                .arg("--stop-server")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
