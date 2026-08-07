use std::{
    env, fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
    thread,
    time::Duration,
};

const DEV_ADDR: &str = "127.0.0.1:1420";
const APP_ID: &str = "patchwork-app";
const APP_NAME: &str = "Patchwork";
const BUILD_BINARY_NAME: &str = "patchwork-app-tauri";

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("frontend-build") => build_frontend(true),
        Some("frontend-dev") => build_frontend(false).and_then(|()| serve_dev()),
        Some("dev") => run_tauri(&["dev"]),
        Some("build-debug") => run_tauri(&["build", "--debug", "--no-bundle", "--ci"]),
        Some("install") => install_app(),
        _ => Err(
            "usage: patchwork-desktop-tool <frontend-build|frontend-dev|dev|build-debug|install>"
                .to_owned(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn app_dir() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot determine patchwork-app directory".to_owned())
}

fn run_tauri(arguments: &[&str]) -> Result<(), String> {
    let app_dir = app_dir()?;
    let mut command = Command::new("cargo");
    command.arg("tauri").args(arguments).current_dir(&app_dir);
    run_checked(&mut command, "cargo tauri")
}

fn run_checked(command: &mut Command, label: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("failed to start {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} exited with {status}"))
    }
}

fn build_frontend(release: bool) -> Result<(), String> {
    let app_dir = app_dir()?;
    let mut command = Command::new("cargo");
    command.args(["leptos", "build"]);
    if release {
        command.arg("--release");
    }
    command.arg("--frontend-only").current_dir(&app_dir);
    run_checked(&mut command, "cargo leptos")?;

    let dist = app_dir.join("dist");
    fs::create_dir_all(&dist)
        .map_err(|error| format!("failed to create '{}': {error}", dist.display()))?;
    fs::copy(app_dir.join("index.html"), dist.join("index.html"))
        .map_err(|error| format!("failed to copy index.html into dist: {error}"))?;
    Ok(())
}

#[cfg(windows)]
fn install_app() -> Result<(), String> {
    run_tauri(&["build", "--ci", "--bundles", "nsis"])?;
    let app_dir = app_dir()?;
    let installer = newest_matching_file(&target_roots(&app_dir), |path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            && path
                .components()
                .any(|component| component.as_os_str() == std::ffi::OsStr::new("nsis"))
    })?
    .ok_or_else(|| "Tauri completed but the NSIS installer could not be found".to_owned())?;
    println!("Launching installer: {}", installer.display());
    let mut command = Command::new(&installer);
    run_checked(&mut command, "Patchwork NSIS installer")
}

#[cfg(target_os = "linux")]
fn install_app() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    run_tauri(&["build", "--no-bundle", "--ci"])?;
    let app_dir = app_dir()?;
    let source = release_binary(&app_dir)?;
    let prefix = if let Some(prefix) = env::var_os("PATCHWORK_INSTALL_PREFIX") {
        PathBuf::from(prefix)
    } else {
        let home = env::var_os("HOME")
            .ok_or_else(|| "HOME is required for a user installation".to_owned())?;
        PathBuf::from(home).join(".local")
    };
    if !prefix.is_absolute() {
        return Err("PATCHWORK_INSTALL_PREFIX must be an absolute path".to_owned());
    }

    let bin_dir = prefix.join("bin");
    let applications_dir = prefix.join("share/applications");
    let icon_root = prefix.join("share/icons/hicolor");
    let icon_dir = icon_root.join("256x256/apps");
    fs::create_dir_all(&bin_dir)
        .and_then(|()| fs::create_dir_all(&applications_dir))
        .and_then(|()| fs::create_dir_all(&icon_dir))
        .map_err(|error| format!("failed to create installation directories: {error}"))?;

    let binary = bin_dir.join(APP_ID);
    fs::copy(&source, &binary).map_err(|error| {
        format!(
            "failed to install binary '{}' to '{}': {error}",
            source.display(),
            binary.display()
        )
    })?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("failed to mark '{}' executable: {error}", binary.display()))?;

    let icon = icon_dir.join(format!("{APP_ID}.png"));
    fs::copy(app_dir.join("public/logo.png"), &icon)
        .map_err(|error| format!("failed to install icon '{}': {error}", icon.display()))?;

    let desktop_file = applications_dir.join(format!("{APP_ID}.desktop"));
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName={APP_NAME}\nComment=Patchwork desktop app\nExec={}\nIcon={APP_ID}\nTerminal=false\nCategories=Utility;\nStartupNotify=true\n",
        desktop_exec_path(&binary)
    );
    fs::write(&desktop_file, desktop).map_err(|error| {
        format!(
            "failed to install desktop entry '{}': {error}",
            desktop_file.display()
        )
    })?;

    refresh_linux_desktop(&applications_dir, &icon_root);
    println!("Installed Patchwork for the current user:");
    println!("  binary:  {}", binary.display());
    println!("  icon:    {}", icon.display());
    println!("  desktop: {}", desktop_file.display());
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn install_app() -> Result<(), String> {
    Err("desktop installation is currently supported on Linux and Windows".to_owned())
}

#[cfg(target_os = "linux")]
fn desktop_exec_path(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "linux")]
fn refresh_linux_desktop(applications_dir: &Path, icon_root: &Path) {
    let _ = Command::new("update-desktop-database")
        .arg(applications_dir)
        .status();
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(icon_root)
        .status();
}

fn release_binary(app_dir: &Path) -> Result<PathBuf, String> {
    let mut name = BUILD_BINARY_NAME.to_owned();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    for root in target_roots(app_dir) {
        let candidate = root.join("release").join(&name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Tauri completed but release binary '{name}' could not be found"
    ))
}

fn target_roots(app_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(configured) = env::var_os("CARGO_TARGET_DIR") {
        let configured = PathBuf::from(configured);
        if configured.is_absolute() {
            roots.push(configured);
        } else {
            roots.push(app_dir.join(&configured));
            roots.push(app_dir.join("src-tauri").join(&configured));
        }
    }
    roots.push(app_dir.join("src-tauri/target"));
    roots.push(app_dir.join("target"));
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(windows)]
fn newest_matching_file(
    roots: &[PathBuf],
    predicate: impl Fn(&Path) -> bool,
) -> Result<Option<PathBuf>, String> {
    let mut matches = Vec::new();
    for root in roots {
        collect_matching_files(root, &predicate, &mut matches)
            .map_err(|error| format!("failed to inspect '{}': {error}", root.display()))?;
    }
    matches.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::UNIX_EPOCH)
    });
    Ok(matches.pop())
}

#[cfg(windows)]
fn collect_matching_files(
    path: &Path,
    predicate: &impl Fn(&Path) -> bool,
    output: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_matching_files(&path, predicate, output)?;
        } else if file_type.is_file() && predicate(&path) {
            output.push(path);
        }
    }
    Ok(())
}

fn serve_dev() -> Result<(), String> {
    if TcpStream::connect_timeout(
        &DEV_ADDR.parse().expect("constant address is valid"),
        Duration::from_millis(150),
    )
    .is_ok()
    {
        println!("Patchwork frontend is already available at http://{DEV_ADDR}");
        return Ok(());
    }

    let dist = app_dir()?.join("dist");
    let listener = TcpListener::bind(DEV_ADDR)
        .map_err(|error| format!("failed to serve '{}' on {DEV_ADDR}: {error}", dist.display()))?;
    println!("Serving Patchwork frontend at http://{DEV_ADDR}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let dist = dist.clone();
                thread::spawn(move || {
                    if let Err(error) = serve_connection(stream, &dist) {
                        eprintln!("frontend server error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("frontend server accept error: {error}"),
        }
    }
    Ok(())
}

fn serve_connection(mut stream: TcpStream, dist: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = [0_u8; 16 * 1024];
    let read = stream.read(&mut request)?;
    if read == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&request[..read]);
    let Some(first_line) = request.lines().next() else {
        return write_response(&mut stream, "400 Bad Request", "text/plain", b"Bad Request", false);
    };
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let head_only = method == "HEAD";
    if method != "GET" && !head_only {
        return write_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"Method Not Allowed",
            head_only,
        );
    }

    let target = target.split(['?', '#']).next().unwrap_or("/");
    let relative = if target == "/" {
        PathBuf::from("index.html")
    } else {
        safe_relative_path(target.trim_start_matches('/'))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe request path"))?
    };
    let path = dist.join(relative);
    match fs::read(&path) {
        Ok(body) => write_response(
            &mut stream,
            "200 OK",
            content_type(&path),
            &body,
            head_only,
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => write_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"Not Found",
            head_only,
        ),
        Err(error) => Err(error),
    }
}

fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => safe.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!safe.as_os_str().is_empty()).then_some(safe)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_relative_static_asset_paths() {
        assert_eq!(
            safe_relative_path("assets/app.js"),
            Some(PathBuf::from("assets/app.js"))
        );
        assert_eq!(
            safe_relative_path("./index.html"),
            Some(PathBuf::from("index.html"))
        );
        assert!(safe_relative_path("../secret").is_none());
        assert!(safe_relative_path("").is_none());
    }

    #[test]
    fn target_roots_always_include_tauri_and_app_targets() {
        let app = Path::new("app");
        let roots = target_roots(app);
        assert!(roots.contains(&app.join("src-tauri/target")));
        assert!(roots.contains(&app.join("target")));
    }
}
