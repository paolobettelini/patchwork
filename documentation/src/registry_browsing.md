# Registry browsing

The website and desktop launcher render Browse through the same
`patchwork-ui` component and DTOs from `patchwork-registry-types`. Search text,
the Mods checkbox, and the Modpacks checkbox form a `RegistryBrowseRequest`.
Both project filters default to enabled.

## Remote search

The public endpoint is:

```http
GET /registry/search?q=inventory&mods=true&modpacks=true
```

The backend searches permanent IDs, latest-version titles, repository
owner/name, and modpack descriptions. Results always represent the current
`latest_version_id`; yanked/version-history browsing is a separate future
feature. The response combines typed mod and modpack results and orders them by
downloads, then title.

Each result includes project kind/ID, title, description, version, downloads,
repository coordinates, exact source commit, source tree OID, manifest SHA-256,
and optional artifact URLs. It never embeds source files or GitHub credentials.

Selecting a remote result opens its canonical project page. The page loads:

```http
GET /registry/projects/{mods|modpacks}/{project_id}
```

The response is built from `latest_version_id` and includes the Patchwork
publisher UUID/current nickname, publication timestamp, real database download
counter, exact repository snapshot, and dependencies belonging to that version.
Dependency targets that exist in the registry link to their own project page.

Published artifacts are fetched on demand through:

```http
GET /registry/projects/{mods|modpacks}/{project_id}/manifest
GET /registry/projects/{mods|modpacks}/{project_id}/readme
GET /registry/projects/{mods|modpacks}/{project_id}/image
GET /registry/projects/mods/{project_id}/source
```

The backend looks up the authoritative latest version and Git blob OID in the
database, authenticates as the GitHub App, and returns that exact blob. It does
not read a branch name. Manifest responses are TOML, README responses are raw
Markdown, and images are limited to PNG/WebP/JPEG. The routes are public
registry content even when the GitHub repository itself is private.

`source` is a bounded `tar.gz` assembled from the exact Git tree stored for the
published mod version. It contains only that crate directory, not the whole
repository. The server rejects truncated trees, symlinks, Git submodules, unsafe paths, too
many files, and excessive per-file/total sizes. It reuses one short-lived
GitHub App installation token while fetching blobs concurrently.

## Desktop backend setting

`settings.json` has one `backend` URL, defaulting to
`http://127.0.0.1:8080`. Browse, account/OAuth, GitHub, Upload, Profile, and
artifact requests all use this value. The former arbitrary Remote database
list no longer exists. When the web service is mounted below a base path, this
setting includes it, for example `https://mods.example.com/patchwork`; desktop
route and artifact resolution preserve that prefix.

Changing Backend normalizes and persists the URL. If it differs from the
previous server, the launcher clears the bearer token and cached profile:
tokens are server-specific and must never be sent to a newly selected backend.
The top bar receives the ordinary Tauri auth event and immediately becomes
signed out.

## Local registries

The desktop-only `localFolders` setting is an ordered list of filesystem roots.
Browse always performs the remote request and scans every configured local
folder. A backend/network failure becomes a warning rather than hiding valid
local results.

Local scanning recursively enumerates candidate TOML files, skips symlinks,
`.git`, and `target`, and applies defensive depth/file limits. `Cargo.toml`
files are passed to `patchwork::parse_registry_mod_manifest`, including parent
workspace manifests for inherited versions. Loose TOML files are passed to
`patchwork::parse_registry_modpack_manifest`. Therefore local and remote
results use the same Patchwork identity, SemVer, metadata, and dependency
rules; this is not a filename-only search.

Local results are labelled with their filesystem root and display `-` instead
of inventing a download count. Their
`manifest_sha256` is calculated from actual bytes. They deliberately have no
Git commit or `source_tree_oid`, because a plain folder supplies neither.

## Desktop profile actions

Mods expose **Add to existing profile**. Clicking it opens the profile list;
choosing one immediately adds the mod ID to that profile's `mods` array.

Modpacks expose two actions:

- **Add to existing profile** opens the profile list and adds the ID to the
  chosen profile's `modpacks` array.
- **Download as profile** creates `<profiles>/<id>.toml`, verifies its identity,
  version, and SHA-256, then copies the optional `<id>.md` and identifying
  image beside it. Local files are copied directly; remote files are obtained
  through the immutable artifact routes above.

Both operations sort and deduplicate the edited dependency list. They reject
invalid IDs, missing profiles, malformed manifests, checksum mismatches, and a
destination profile that already exists.

**Download as profile** also writes launcher-only provenance under
`profiles/.patchwork/<id>.origin.json`. This records the selected Browse source
without changing the public modpack TOML format. Home uses it to distinguish a
remote GitHub publication from a local-registry project after both have been
materialized on disk. For older profiles without a sidecar, Home checks the
configured local folders first and then resolves the modpack ID through the
backend. Cache presence is never used as a provenance signal.

Both operations invoke the native dependency resolver after editing/creating
the profile. For **Download as profile**, the selected root modpack is verified,
cached, and copied into `profiles/` first; the UI adds it to the sidebar before
starting the transitive pass. The selected project and every transitive
mod/modpack dependency are materialized into `cache/mods` and
`cache/modpacks`. A failure is attached to that project and resolution
continues, so the UI can report a partial download; Compile refuses to proceed
while required projects are still missing.

## Dependency resolution and cache installation

The resolver uses typed `(project_kind, project_id)` keys, deduplicates cycles,
and reads dependencies with the Patchwork core parsers. Lifecycle relations and
the API target declared by `provides` are both traversed. This guarantees that
a provider's API crate is materialized even when no lifecycle list names it.
For a missing cache entry the order is:

1. an already valid cache entry;
2. the first matching project from the ordered local registry folders;
3. the configured remote backend's latest published version.

Before scheduling either a root or transitive dependency, the resolver removes
mod IDs containing `generated`. Those crates are created by Compose-time
codegen, so they do not contribute to progress totals, do not produce download
errors, and are never requested from local folders or the backend. Dependency
views keep the reference visible but disabled as **Generated during compose**.

Local mods copy their crate directory while excluding `.git` and `target` and
rejecting symlinks. Remote mods fetch the exact source archive described above.
Both paths parse the installed `Cargo.toml`, verify ID/version, and verify the
manifest SHA-256 when available. Workspace-inherited package versions are
materialized to the published concrete version so the isolated cached crate
remains parseable without its former workspace.

Modpacks copy/fetch only `<id>.toml`, optional `<id>.md`, and the deterministic
PNG/WebP/JPG/JPEG image. The manifest is parsed and checksum-verified before it
becomes the visible cache file. Cache directory replacement uses staging and a
rollback backup for mods; malformed or incomplete content does not replace a
previous valid crate.

Refresh compares locally cached and available versions with SemVer. The
launcher keeps older immutable published versions available server-side, but
its current cache contains one selected/latest version per project ID.

Successful remote source downloads increment a mod's database counter;
successful remote modpack manifest downloads increment a modpack's counter.
README/image requests and local-folder copies do not increment counters.
Browse and Details format large values as `1.2K`/`1.2M` while local results
continue to display `-`.

## Trust boundary

Search results are display/input data, not publication authority. Registry
publishing still trusts only persisted scan entries. Install resolution reloads
authoritative project details from the backend, downloads only immutable Git
objects selected by the stored `source_tree_oid`, verifies the returned
manifest SHA-256, and parses it again before replacing cache content. It never
installs `main` or another mutable branch.
