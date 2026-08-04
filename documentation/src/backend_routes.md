# Backend routes

The Actix backend serves JSON APIs, OAuth HTML/form endpoints, GitHub redirects,
compiled Leptos assets, and the SPA entry point. Protected routes accept either
the HttpOnly browser session cookie or a desktop bearer token unless stated
otherwise.

## Account API

| Method | Route | Authentication | Purpose |
| --- | --- | --- | --- |
| `POST` | `/api/auth/register` | Public | Validate registration, send a code, and return a pending verification challenge. |
| `POST` | `/api/auth/register/verify` | Public challenge | Consume a six-digit code, create the account, and start a browser session. |
| `POST` | `/api/auth/login` | Public | Sign in by email or nickname and create a browser session. |
| `GET` | `/api/auth/me` | Required | Return account, GitHub link, mods, and modpacks. |
| `POST` | `/api/auth/logout` | Optional | Revoke the supplied cookie session or bearer token. |
| `POST` | `/api/account/nickname` | Required | Change the mutable nickname. |
| `GET` | `/api/profile` | Required | Profile alias shared with the desktop app. |

Registration JSON uses `email`, `nickname`, and `passwordSha256`, and returns
HTTP `202` with `verificationId`, normalized `email`, and `expiresIn` seconds.
Verification JSON uses `verificationId` and six-digit `code`; success returns
the profile and sets the session cookie. Login uses `identifier` and
`passwordSha256`; identifier may be email or nickname. Profile JSON includes
the stable Patchwork UUID, an optional linked GitHub account, and published
projects. Published mods and modpacks include their latest version, repository
URL/path, and `canRescan: true`.

## Desktop OAuth

| Method | Route | Authentication | Purpose |
| --- | --- | --- | --- |
| `GET` | `/oauth/authorize` | Session-aware | Validate the PKCE request and show login or consent HTML. |
| `POST` | `/oauth/login` | Public form | Sign in within the authorization flow. |
| `POST` | `/oauth/register` | Public form | Validate registration, email a code, and show the verification form. |
| `POST` | `/oauth/register/verify` | Public challenge form | Verify email, create the account/session, and show consent. |
| `POST` | `/oauth/consent` | Browser session | Approve the launcher and issue a short-lived code. |
| `POST` | `/oauth/token` | Code + verifier | Consume the code and issue a desktop bearer token. |

The authorize endpoint accepts only `response_type=code`, client ID
`patchwork-app`, PKCE method `S256`, and a loopback redirect URI. Token exchange
requires `grant_type=authorization_code`, the same redirect URI, and the
original verifier.

## GitHub API

| Method | Route | Authentication | Purpose |
| --- | --- | --- | --- |
| `GET` | `/github/connect` | Required | Begin browser linking and redirect to GitHub. |
| `POST` | `/github/connect` | Bearer token | Begin desktop linking and return `authorizationUrl`. |
| `GET` | `/github/callback` | One-time state | Exchange GitHub code, persist identity, and redirect to completion URL. |
| `GET` | `/github/installation-complete` | Public | Show the non-authoritative completion page after a GitHub App installation. |
| `GET` | `/github/account` | Required | Return the linked GitHub identity or `404`. |
| `DELETE` | `/github/account` | Required | Remove the local GitHub association. |

Desktop connect JSON contains `completionUrl`. Only the strict Patchwork
loopback callback form is accepted. The callback identifies the Patchwork
account from the one-time state record, not from a browser cookie.

## Registry API

Browse and published-artifact routes are public. Scan, publish, and Rescan
routes require a browser session cookie or desktop bearer token; scan and
publish additionally require a linked GitHub account. Scans are private to
their publisher.

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/registry/search?q=...&mods=true&modpacks=true` | Search latest published mod and/or modpack versions. |
| `GET` | `/registry/projects/{mods\|modpacks}/{project_id}` | Return public details for the authoritative latest version, including publisher, downloads, publication date, immutable Git coordinates, and version dependencies. |
| `GET` | `/registry/projects/mods/{project_id}/source` | Build and return a `tar.gz` of the exact published mod directory tree. Successful responses increment the mod download counter. |
| `GET` | `/registry/projects/{mods\|modpacks}/{project_id}/manifest` | Return the exact published manifest blob. |
| `GET` | `/registry/projects/{mods\|modpacks}/{project_id}/readme` | Return the optional exact published Markdown blob. |
| `GET` | `/registry/projects/{mods\|modpacks}/{project_id}/image` | Return the optional exact published PNG/WebP/JPEG blob. |
| `POST` | `/registry/scans` | Authorize a GitHub repository, resolve its default branch, scan mod and modpack manifests, persist and return a preview. |
| `POST` | `/registry/scan-jobs` | Start the same scan asynchronously and return a temporary job UUID. |
| `GET` | `/registry/scan-jobs/{job_id}` | Return owned scan phase, counts, validated entries, final preview, or error. |
| `GET` | `/registry/scans/{scan_id}` | Reload one owned persisted preview. |
| `POST` | `/registry/scans/{scan_id}/publish` | Publish selected entry UUIDs transactionally from authoritative scan data. |
| `POST` | `/registry/mods/{mod_id}/rescan` | Scan the current default branch of an owned mod's repository/base path. |
| `POST` | `/registry/mods/{mod_id}/rescan-job` | Start that same rescan as a progressive job. |
| `POST` | `/registry/projects/{mods\|modpacks}/{project_id}/rescan` | Run the shared typed Rescan flow for an owned mod or modpack. |
| `POST` | `/registry/projects/{mods\|modpacks}/{project_id}/rescan-job` | Start the typed Rescan as a progressive job; this is what current clients use. |

`source` is intentionally available only for mods. The backend obtains one
GitHub App installation token, enumerates the recursive tree identified by the
published `source_tree_oid`, fetches its blobs concurrently, rejects symlinks
and unsafe/archive-too-large paths, and never reads a mutable branch. Modpacks
are downloaded as their manifest plus optional README/image artifacts instead.
Fetching a mod source archive increments the mod counter after the archive is
successfully assembled; fetching a modpack manifest increments its counter.
Mod IDs containing `generated` are reserved build outputs: search omits them
and the public project-details route returns not found for them.

Create-scan request:

```json
{
  "repositoryUrl": "https://github.com/owner/repository",
  "basePath": "optional/subdirectory-or-modpack.toml"
}
```

Create/rescan returns HTTP `201`. The response includes `scanId`, numeric and
display repository coordinates, base path, requested ref, exact resolved
commit, timestamps, scan warnings/errors, and entries. Each entry includes:

```json
{
  "entryId": "UUID",
  "projectKind": "MOD",
  "projectId": "example-mod",
  "title": "Example Mod",
  "description": "",
  "version": "1.2.0",
  "repositoryPath": "mods/example-mod",
  "manifestPath": "mods/example-mod/Cargo.toml",
  "sourceTreeOid": "git-tree-oid",
  "manifestBlobOid": "git-blob-oid",
  "manifestSha256": "64-hex-characters",
  "readmePath": "mods/example-mod/README.md",
  "readmeBlobOid": "git-blob-oid",
  "imagePath": "mods/example-mod/example-mod.png",
  "imageBlobOid": "git-blob-oid",
  "status": "NEW_VERSION",
  "dependencies": [
    { "kind": "run", "targetKind": "MOD", "targetId": "inventory-api", "available": true }
  ],
  "warnings": [],
  "errors": []
}
```

Optional paths/OIDs are `null` when absent. Status is one of `NEW_MOD`,
`NEW_VERSION`, `UNCHANGED`, `VERSION_CONFLICT`, or `ERROR`.

The website and launcher use the job variants so the UI does not remain blank
during a large scan. Starting a job returns HTTP `202`:

```json
{ "jobId": "UUID" }
```

The client polls the owned job every 250 ms. Progress phases are `QUEUED`,
`AUTHORIZING`, `INDEXING_REPOSITORY`, `FETCHING_MANIFESTS`, `VALIDATING_PROJECTS`,
`PERSISTING`, `COMPLETE`, and `FAILED`. `completed` and optional `total` describe
the current phase; `entries` grows as mods and modpacks receive their final classification.
`COMPLETE` includes the ordinary persisted `scan`, while `FAILED` includes an
error string. Jobs are in-memory UI coordination records retained for roughly
30 minutes; the authoritative completed scan remains in the database for its
normal 20-minute lifetime.

Publish request deliberately contains no metadata:

```json
{
  "entryIds": ["UUID", "UUID"]
}
```

Success returns the scan UUID and each created internal version UUID. A publish
returns `400` for malformed/non-publishable selections, `404` for a missing or
foreign scan, and `409` for expired/already-published/stale state. Repository
permission or a missing GitHub link returns `403`. GitHub API failures during a
tree/blob scan are reported as gateway errors.

See [Registry publication](./registry_publication.md) for authorization,
immutability, tree traversal, limits, and client behavior.
See [Registry browsing](./registry_browsing.md) for search DTOs, local
aggregation, artifacts, and desktop profile actions.

## Frontend assets

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/styles.css` | Shared site stylesheet. |
| `GET` | `/logo.png` | Patchwork logo. |
| `GET` | `/pkg/*` | JavaScript and WebAssembly produced by `cargo leptos build`. |
| `GET` | `/`, `/browse`, `/upload`, `/profile`, other non-API paths | Serve the SPA entry point. `/upload?job=<UUID>` follows progress and `/upload?scan=<UUID>` reloads a persisted preview. |

Unknown API-like paths currently reach the general SPA fallback unless a
configured route matches first. New APIs should therefore always be registered
explicitly before the fallback.
