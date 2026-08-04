# Registry publication

Patchwork publishes mod and modpack metadata that points to immutable GitHub
snapshots. It does not copy repositories, source files, README documents, or images into the
registry database. Both the website and desktop launcher use the same Actix
API and shared preview component.

The central invariant is:

```text
(project_kind, project_id, semantic_version) -> one immutable source tree
```

Once `example-mod 1.2.0` or `example-pack 1.2.0` refers to Git tree OID A, no
later request may replace it with tree OID B. Changed source requires a new
semantic version.

## Trust boundary

The browser and Tauri webview are presentation clients. They may submit:

- a GitHub repository URL;
- an optional repository-relative subdirectory or one loose modpack TOML path;
- the UUIDs of scan entries selected for publication.

They never submit authoritative titles, versions, dependencies, commits,
checksums, README paths, or image paths during publish. Those values are
derived by the backend and recovered from its persisted scan.

```text
website or desktop UI
  -> repository URL + base path
Actix backend
  -> GitHub authorization and immutable scan
  -> persisted registry scan
UI
  -> selected scan-entry UUIDs
Actix backend
  -> transactional publication from persisted data
```

The desktop commands are thin authenticated proxies. The Tauri backend reads
the bearer token from Patchwork's `config/auth.json`, calls the same HTTP
routes, and returns their typed response to Leptos. No GitHub token or GitHub
App secret enters WebAssembly.

## Eligibility

A scan requires all of the following:

1. an authenticated Patchwork account;
2. a linked GitHub account;
3. a GitHub App installation with access to the requested repository;
4. `write`, `maintain`, or `admin` permission for the linked numeric GitHub
   user ID.

The numeric GitHub user ID is authoritative. The backend refreshes cached
login/avatar display data after a successful repository authorization.

## Creating a scan

The synchronous API accepts:

```http
POST /registry/scans
Content-Type: application/json

{
  "repositoryUrl": "https://github.com/owner/repository",
  "basePath": "mods"
}
```

`basePath` is optional. Empty input and `.` mean repository root. It may name a
directory such as `mods` or one loose manifest such as
`modpacks/client.toml`. Absolute paths, `..`, control characters, and paths
longer than 1024 bytes are rejected.

The URL must be a plain HTTPS `github.com/owner/repository` URL. A trailing
`.git` is accepted. Credentials, query strings, fragments, and paths such as
`/tree/main` are rejected. GitHub's repository response supplies the canonical
owner, name, URL, default branch, and stable numeric repository ID.

## Authorization and snapshot resolution

The backend authenticates as the GitHub App, finds the installation for the
repository, and creates an installation access token. It resolves the linked
numeric GitHub user to the current login and asks GitHub for that collaborator's
repository permission. Permission is checked before any manifest is accepted.

The default branch is then resolved immediately through the commit API:

```text
default branch name -> exact commit SHA -> commit root tree OID
```

Every later GitHub request in that scan uses the exact commit/tree graph. The
scan never goes back to `main`, `master`, or another mutable ref.

## GitHub tree traversal and performance

Patchwork does not execute `git clone`. After resolving any requested base path,
it requests one recursive Git tree rooted at that exact tree OID. This avoids
the former one-HTTP-round-trip-per-directory bottleneck while still preserving
every directory's Git tree OID. If GitHub marks the recursive response as
truncated, Patchwork fails the scan and asks the caller to narrow the base path;
it never treats a partial tree as complete.

Only these blobs are fetched, with at most 12 GitHub blob requests in flight:

- `Cargo.toml` files below the base path;
- loose `.toml` files below the base path, tested as modpack manifests;
- ancestor `Cargo.toml` files needed for Cargo workspace inheritance.

When `basePath` points directly to `<id>.toml`, only that modpack candidate and
any ancestor Cargo workspace manifests are downloaded. Its parent Git tree is
still indexed so sibling README/image coordinates can be discovered.

Source code, assets, README content, and images are not downloaded during a
scan. Git submodules are not traversed and produce a warning.

Current safety limits are:

| Resource | Limit |
| --- | ---: |
| Git tree entries | 100,000 |
| Directories | 10,000 |
| Directory depth | 64 |
| Candidate manifests | 1,024 |
| One manifest blob | 1 MiB |
| Persisted scan lifetime | 20 minutes |

A recursive GitHub tree response marked `truncated` fails the scan rather than
producing an incomplete result. Network latency and GitHub API rate limits are
now the main variable costs: one tree response still has to enumerate every
tracked entry under the base path, and every required Cargo manifest remains a
separate blob API object even though those requests run concurrently.

## Manifest parsing

Raw Cargo TOML bytes are decoded as UTF-8 and passed to
`patchwork::parse_registry_mod_manifest`. This is the same Patchwork metadata
model used by composition, including support/API mods, lifecycle entry types,
providers, dependency groups, and codegen declarations.

The parser identifies a registry mod only when `[package.metadata.mod]` exists.
It reads:

- mod ID from `package.name`;
- title from `package.metadata.mod.title`, falling back to the ID;
- Cargo semantic version from `package.version`;
- inherited version from the nearest ancestor `[workspace.package].version`
  when `version.workspace = true`;
- `init`, `run`, and `ownership` dependencies;
- the implicit API source dependency declared by `provides`;
- the remaining Patchwork metadata stored as JSON for the immutable version.

IDs must be lowercase slugs containing only `a-z`, `0-9`, `-`, `_`, or `.`,
must start with a letter/digit, and may contain at most 128 bytes. Versions must
parse as semantic versions. Duplicate IDs in one scan are errors.

The lowercase substring `generated` is reserved for Compose-time generated mod
crates. If a parsed mod ID contains it, Scan persists and returns the candidate
as an `ERROR` entry with an explicit publication error; it can never be selected
for Publish. Dependencies pointing to such an ID remain version metadata but
are exempt from registry-availability warnings because they are intentionally
not published.

Every non-`Cargo.toml` candidate is passed to
`patchwork::parse_registry_modpack_manifest`. A valid loose modpack is named
`<id>.toml`, has the ordinary Patchwork modpack shape, and includes the required
top-level `version = "..."` SemVer string. Its title comes from `name` (falling
back to the ID); `description`, imports, selected mods, and ignores are retained
as version metadata. Unrelated TOML files are ignored. Malformed files that look
like modpacks become non-publishable error entries when an identity can be
recovered.

Dependency targets are typed as `mod` or `modpack`. The existing
`modpack/<id>` syntax is therefore preserved in mod dependency arrays without
colliding with a mod whose ID happens to be the same string. Modpack `mods`,
`modpacks`, and `ignore` entries are stored on that immutable modpack version.

## Snapshot coordinates

Each valid scan entry records at least:

| Value | Meaning |
| --- | --- |
| `project_kind` / `project_id` | Permanent typed registry identity. |
| `title` | Display title for this version. |
| `version` | Project semantic version. |
| `repository_path` | Directory containing the mod or loose modpack. |
| `manifest_path` | Exact Cargo or loose modpack TOML path. |
| `resolved_commit` | Exact commit shared by the scan. |
| `source_tree_oid` | Git tree OID for the project directory. |
| `manifest_blob_oid` | Git blob OID returned by GitHub. |
| `manifest_sha256` | SHA-256 calculated over fetched manifest bytes. |
| `readme_path` / `readme_blob_oid` | Optional immutable README coordinates. |
| `image_path` / `image_blob_oid` | Optional immutable image coordinates. |
| dependencies | Ordered per-version Patchwork dependency relations. |
| publisher | Stable Patchwork UUID. |
| GitHub publisher | Numeric linked GitHub user ID. |

For a mod, `source_tree_oid` is the authoritative checksum of all tracked
contents below the crate directory. Patchwork does not call it
`content_sha256`, because it does not download every file and hash the directory
itself. `manifest_sha256` is a real locally verified SHA-256 of the fetched
manifest.

For a loose modpack, `source_tree_oid` is the immutable parent-tree coordinate,
but unrelated sibling projects must not force a version bump. Its published
content identity is therefore the manifest blob plus the optional `<id>.md` and
image blob OIDs; `manifest_sha256` verifies the TOML bytes. A change to any of
those modpack-owned objects produces `VERSION_CONFLICT` for the same version,
while a change to an unrelated sibling does not.

## README and image discovery

For mods, README discovery examines direct files in the crate directory with
deterministic, case-insensitive priority:

1. `README.md`;
2. `README.markdown`;
3. `README`.

For a loose `<id>.toml` modpack, the README is the direct sibling `<id>.md`.
The identifying image must be a direct file named after the project ID. Extension
priority is `png`, `webp`, `jpg`, then `jpeg`. SVG is deliberately excluded.

Only path and Git blob OID are stored. The current published-artifact route can
fetch the raw README blob for Browse/profile transfer at `repository + exact
commit + readme_path`. A future detail page may render that Markdown with raw
HTML disabled, sanitize the HTML, and rewrite relative links/images against
the README directory at the same commit. It must never resolve relative content
against the current default branch.

## Status comparison

After parsing, the backend compares every entry with the registry:

### `NEW_MOD`

The typed project ID does not exist. Publish creates the permanent mod or
modpack and its first version.

### `NEW_VERSION`

The project belongs to the same Patchwork publisher and numeric GitHub
repository, but this semantic version does not exist. Publish adds a version.

### `UNCHANGED`

The same typed `(project_id, version)` exists with the same `source_tree_oid`.
The UI labels it **Already published** and disables selection.

### `VERSION_CONFLICT`

The same typed `(project_id, version)` exists with a different
`source_tree_oid`. The UI shows a red conflict and asks for a Cargo or modpack
version bump. Existing source coordinates are never updated.

### `ERROR`

Examples include malformed Patchwork metadata, duplicate IDs in the scan, or
an ID already owned by another Patchwork publisher/repository. Error entries
are visible but cannot be selected.

Dependencies present in the same valid scan or already present in the matching
`mods`/`modpacks` identity table are
marked available. Missing dependencies are warnings rather than publication
errors, because a dependency may be published separately later. Dependency
relations are stored on `mod_versions` or `modpack_versions`, not on permanent
identity rows.

## Persistent preview

`POST /registry/scans` returns HTTP `201` with a `scanId`, immutable repository
coordinates, ref/commit, expiration, scan warnings/errors, and all entries.
The backend stores the same authoritative values in `registry_scans` and
`registry_scan_entries`.

The website and desktop app normally start a progressive job instead:

```http
POST /registry/scan-jobs
GET /registry/scan-jobs/{job_id}
```

The POST returns `202` with a temporary job UUID. Both clients poll every 250
ms and render the current phase, `completed / total`, and each project after its
status has been validated. Manifest download progress is therefore visible as
`X / Y manifests`; validation is visible as `X / Y projects`. The final job payload
contains the same persisted `RegistryScan` returned by the synchronous route.

Jobs live only in backend memory and are scoped to the authenticated publisher;
they coordinate UI progress and expire after roughly 30 minutes. They are not
publication authority. Only the database-backed scan and entry UUIDs can be
published.

Clients select every publishable entry by default. Users may turn individual
checkboxes off. Unchanged, conflicting, and error entries remain disabled.

A scan can be reloaded while valid:

```http
GET /registry/scans/{scan_id}
```

The scan is private to its publisher. Another account receives `404`, avoiding
information leakage about a scan UUID.

## Publishing

The client sends only persisted entry UUIDs:

```http
POST /registry/scans/{scan_id}/publish
Content-Type: application/json

{
  "entryIds": [
    "9a5d9892-9897-47c7-9810-d331a9d9ebaf"
  ]
}
```

Before writing, the backend verifies:

- scan ownership;
- the same linked numeric GitHub user;
- expiration and single-use state;
- non-empty, unique entry UUIDs;
- membership of every entry in the scan;
- `NEW_MOD` or `NEW_VERSION` status and no entry errors;
- current registry ownership and version state.

Publication runs in one database transaction. It upserts repository display
coordinates by numeric GitHub repository ID, creates permanent mod/modpack
identities as needed, inserts immutable versions/dependencies, advances `latest_version_id` using
semantic-version ordering, and marks the scan published.

The database enforces both `UNIQUE(mod_id, version)` and
`UNIQUE(modpack_id, version)`. A concurrent publish that
wins after the scan therefore causes the losing transaction to fail and ask
the user to rescan. No GitHub ref needs to be resolved again because the stored
commit is immutable.

## Rescan

Every published mod or modpack on a profile has **Rescan**. The shared routes are:

```http
POST /registry/projects/{mods|modpacks}/{project_id}/rescan
POST /registry/projects/{mods|modpacks}/{project_id}/rescan-job
```

The legacy mod-only route remains available during development, but both
clients use the typed project route and then poll the ordinary scan-job endpoint.

The backend loads the project's numeric repository and original source base path,
verifies ownership, then calls the exact same scanner used by initial upload.
It resolves the repository's current default branch to a new commit and may
return new versions, unchanged entries, conflicts, or newly discovered projects.

The website redirects immediately to `/upload?job=<job_id>` and renders job
progress; the desktop app switches directly to its Upload view and does the
same. Removing a mod or modpack from the current branch never deletes old versions:
published versions remain reproducible through historical commits. Yanking or
archiving is a separate future operation.

## Installation contract

An installer must eventually receive at least:

```text
numeric/canonical repository identity
exact source_commit
repository_path
source_tree_oid
```

It must fetch/checkout `source_commit`, never the default branch. After fetch it
can compare the checked-out directory's Git tree OID with `source_tree_oid`.
The registry database remains metadata and identity storage, not a source-code
mirror.
