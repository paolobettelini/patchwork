# Desktop launcher

`patchwork-app` is a Leptos frontend hosted in a Tauri window. Rust compiled to
WebAssembly owns the UI; the native Tauri crate owns filesystem access,
processes, dialogs, browser launching, authentication callbacks, and PTYs.

The launcher currently provides Home, Browse, Upload, Settings, Profile, and
Console views. Browse and Upload reuse components from `patchwork-ui`, while
desktop-only actions are passed in at the application boundary.

## Local data and settings

The Linux default is:

```text
~/.local/share/patchwork/
  config/
    settings-path.json
    settings.json
    auth.json
  cache/
    mods/
    modpacks/
    build/
    bin/
  profiles/
    .patchwork/
      <profile-id>.origin.json
  target/
```

- `settings-path.json` is the bootstrap pointer to the currently selected
  `settings.json`. It is necessary because the settings file itself can be
  relocated from the Installation page.
- `settings.json` stores theme, the single backend URL, local registry folders,
  and configurable paths.
- `auth.json` stores the desktop bearer token and a cached profile. It is
  created with owner-only permissions on Unix.
- `cache/mods` holds installed Rust crate directories; `cache/modpacks` holds
  verified modpack TOML files and optional README/image sidecars.
- `cache/build` contains composed project output.
- `cache/bin` contains built profile executables, separated by profile and
  debug/release mode.
- `profiles` contains local profile/modpack TOML files and their icons.
- `profiles/.patchwork/*.origin.json` records whether a profile created through
  Browse came from a remote publication or a configured local registry. It is
  launcher metadata, not part of the modpack format.
- `target` is the launcher's default Cargo target directory for composed
  projects. The Installation settings may point Cargo elsewhere.

Installation shows the measured size of the Cargo, target, composed build, and
binary caches. The four red clear actions ask for confirmation and are rejected while
a Patchwork task is active:

- **Clear cargo cache** removes only `$CARGO_HOME/registry` and
  `$CARGO_HOME/git`. It deliberately preserves Cargo binaries, credentials,
  configuration, and the rest of Cargo home.
- **Clear target cache** removes and recreates the configured
  `cargoTargetDir`.
- **Clear build cache** removes and recreates the configured `buildCache`.
- **Clear binary cache** removes and recreates the configured `binCache`.
  Profiles are not runnable again until their selected mode is rebuilt.

Sizes use decimal units (`KB`, `MB`, `GB`) and are calculated off the UI thread.
Patchwork does not follow symbolic links while measuring and refuses to clear a
cache path that is relative, a filesystem root, the home directory, or a
symbolic link.

No launcher state is intentionally stored in Tauri's application data folder.
Tauri is only used to locate the operating system's local data root, from which
the sibling `patchwork` directory is selected.

## Download, compose, build, and stop

Compose calls the Patchwork core and writes the generated project below the
configured build cache. Build invokes `cargo build` with the configured
`CARGO_TARGET_DIR`. After a successful build, Patchwork moves the executable to
`binCache/<profile-id>/<debug|release>/<package-name>` and leaves an absolute
symbolic link at Cargo's original target path. Copy-then-remove semantics are
used so target and binary cache may reside on different filesystems.

Run never invokes Cargo. It starts the cached executable directly in the PTY,
using the composed project as its working directory when available. Only that
game process receives `BACKEND_ADDR`, whose value is the current **Backend**
setting. Compose, codegen and Cargo Build do not receive this variable. The Stop
command retains enough native process state to terminate either Cargo Build or
the running game. Patchwork also sets `BEVY_ASSET_ROOT` to the composed project,
so Bevy still finds its assembled `assets/` tree after the executable is moved
to the binary cache.

The gear beside the primary action configures an ordered checkbox pipeline:
**Download**, **Compose**, **Build**. All three default to enabled. Enabling a
stage enables every prerequisite before it; disabling an earlier stage disables
all later stages. Download resolves the complete profile graph and verifies
that every required mod/modpack is in its configured cache. Compose and Build
do not start if this prerequisite phase reports missing projects.

All native downloads emit `patchwork-download` events and retain the latest
status for a lightweight 120 ms frontend poll. The status contains phase,
current project, processed/total counts, and accumulated errors. The top bar
therefore renders one reliable global progress strip as
`<current-id> [processed/total]`, regardless of whether the operation started
from Browse, Download updates, or the compose pipeline. Individual project
failures do not cancel unrelated downloads, but a pipeline with any missing
dependency terminates before composition.

## Terminal console

Cargo Build and cached game executables run inside `portable-pty`, not through
separate piped stdout and stderr readers. The PTY merges both streams in their
real terminal order and preserves carriage returns, cursor movement, erase
commands, colours, and interactive terminal behaviour.

The native backend reads raw byte chunks from the PTY master, Base64-encodes
them, and emits Tauri events. The Leptos frontend decodes each chunk into a
`Uint8Array` and writes it directly to xterm.js with `convertEol: false`.
FitAddon observes the console panel, fits rows and columns, and sends the new
size back to the backend so the PTY can be resized. No ANSI parser or HTML
conversion exists in Patchwork.

```text
cargo build / cached game -> PTY -> raw bytes -> Base64 event -> Uint8Array -> xterm.js
```

## Desktop authentication

Sign in opens the system browser and uses OAuth 2.0 Authorization Code Flow
with PKCE S256. The launcher binds a random `127.0.0.1` port before opening the
browser, and the server accepts only loopback redirect URIs for the fixed
callback path. On success, the app exchanges the short-lived code for a bearer
token and persists it in `config/auth.json`.

The token is valid for 90 days unless revoked. Closing the window does not log
the user out: on the next start the launcher loads the token and cached profile
from `auth.json`, renders the cached identity immediately, and then refreshes
the complete profile from `/api/profile`. This synchronizes nickname, GitHub
connection, and published projects changed through the website. If the backend
is temporarily unavailable the cached profile remains visible. Logout revokes
the token server-side and clears the local token and profile while retaining
the configured server URL.

GitHub linking uses a separate short-lived loopback listener and is described
in [GitHub integration](./github_integration.md).

The configured Backend defaults to `http://127.0.0.1:8080` and is the single
network origin for auth, GitHub, registry browsing, Upload, Profile, and
published artifacts. Changing it signs out credentials issued by the previous
server. Registries stores only a dynamic list of local folders in addition to
this one URL; the old Remote database list has been removed.

## Browse and profile installation

Browse first queries the configured backend and then scans every configured
local folder with the Patchwork manifest parsers. Remote failures are displayed
as warnings so local browsing remains available. Results share one typed model
and can be filtered to mods, modpacks, or both.

The result area owns its vertical scroll. Remote titles open the same typed
Details/Dependencies page used by the website; local results remain filesystem
entries and show `-` for downloads. **Add to existing profile** is a themed
menu whose entries perform the add directly, rather than a native select plus
a second confirmation button.

Adding a result to an existing profile immediately edits its `mods` or
`modpacks` list and then installs that project and all transitive dependencies.
Downloading a modpack as a new profile first installs only the selected root
modpack, copies its verified manifest plus optional image and README under
`profiles/`, and immediately adds it to the launcher sidebar. The transitive
dependency download starts afterwards, so a later dependency failure never
prevents the already valid profile from being created. Local-folder projects
take precedence over remote GitHub downloads; see
[Registry browsing](./registry_browsing.md).

Each profile has a refresh icon. Refresh is also performed after the fast local
profile load at launcher boot. It re-reads the profile and compares all cached
projects with local/remote available versions. When one or more newer versions
exist, **Download updates (N)** appears beside Refresh and installs only newer
or missing candidates using the same resolver and progress events.

## Registry upload

The desktop Upload page reuses the same component and DTO crate as the website.
Its callbacks invoke native Tauri commands:

```text
Leptos Upload component
  -> registry_start_scan / registry_scan_progress
  -> registry_get_scan / registry_publish_scan
Tauri native backend
  -> bearer token from config/auth.json
  -> Actix registry route
```

The native commands use blocking HTTP in Tauri blocking tasks, keeping the
webview responsive. They proxy typed JSON only; GitHub authorization, tree
walking, Cargo parsing, checksums, status comparison, and publication all stay
in the shared server backend.

The desktop project-detail command calls the configured Backend's public
`GET /registry/projects/{kind}/{id}` route and absolutizes artifact URLs before
passing the typed response to Leptos. Home dependency pages read installed
content and dependency metadata from the cache, but cache location never
determines provenance. A project is local only when an exact typed ID is found
under one of the configured `localFolders`. Profiles created through Browse
retain the selected result's origin in `profiles/.patchwork`; pre-existing
profiles without that sidecar remain ordinary local profiles. A remote profile's
Home summary loads its current download count from Backend and falls back to the
count captured in its origin sidecar when Backend is unavailable; local profiles
display `-`. Details exposes ID, description, version, publisher, publication
timestamp, downloads, repository/path, exact commit, source tree OID, and
manifest SHA-256. Merely being installed in `cache/mods` or `cache/modpacks`
never makes a project local.

During scan, Leptos polls the temporary backend job every 250 ms and displays
the current phase, manifest/project counts, and newly validated entries. The job is
only progress state; publishing still uses the final persisted scan UUID and
entry UUIDs.

After publish the launcher reloads the persisted scan and refreshes
`/api/profile`, which updates the local cached profile in `auth.json`. Selecting
**Rescan** opens Upload without starting work. The canonical repository is
filled automatically; a mod uses its crate directory, while a modpack uses the
exact loose `<repository-path>/<id>.toml` manifest. The user can review or edit
both fields and then press **Scan**, limiting the operation to that project path.

The launcher never clones a repository for publication or installation. The
registry backend serves a mod archive from the exact published directory tree;
the launcher verifies its manifest and installs it atomically. Mutable default
branches are used only to discover a new commit during Scan/Rescan, never for
an existing published version.

## Build scripts

- `scripts/dev-frontend.sh`: build the development CSR frontend and serve
  `dist/` on port 1420, unless a server already responds there.
- `scripts/build-frontend.sh`: build release frontend assets and copy
  `index.html` into `dist/`.
- `scripts/tauri-dev.sh`: run `cargo tauri dev` from the app directory.
- `scripts/tauri-build-debug.sh`: create a debug native binary without a
  platform bundle.
- `scripts/install_arch.sh`: build a release binary and install the executable,
  desktop entry, and icon system-wide on Arch Linux.
