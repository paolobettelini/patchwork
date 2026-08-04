# Database

`patchwork-database` is the shared Diesel persistence layer for accounts,
browser/app/game sessions, GitHub links, registry scans, and immutable mod/modpack versions.
SQLite is the default backend; MySQL is an alternative compile-time feature.
Exactly one backend feature must be enabled.

`Database::connect` creates a connection pool and applies embedded migrations.
SQLite connections enable foreign keys and a five-second busy timeout.

## Development baseline

Patchwork currently keeps the complete schema in one baseline migration per
backend. Schema edits intentionally have no upgrade compatibility. After this
game-authentication schema change, remove the development SQLite database and
start the server so the baseline can create the new tables:

```bash
rm patchwork-web/patchwork.sqlite
```

The deletion removes accounts, pending registrations, sessions, GitHub links,
scans, mods, versions, and modpacks. The launcher's cached bearer token will no
longer identify a server-side token; sign out/in or remove Patchwork's local
`config/auth.json`. Do not delete the entire Patchwork data directory.

Forward-only migrations must replace this policy before preserving real user
data matters.

## Identity model

- Patchwork accounts use generated UUIDs. Nicknames and email are mutable or
  display/login data, not foreign keys.
- GitHub users use numeric GitHub user IDs. Login is cached display data.
- GitHub repositories use numeric repository IDs together with provider
  `github`. Owner/name and canonical URL may change.
- Mods and modpacks use permanent, typed registry slug IDs.
- Version rows use internal UUIDs plus the unique public pairs
  `(mod_id, version)` or `(modpack_id, version)`.

## Authentication tables

| Table | Purpose |
| --- | --- |
| `accounts` | UUID, unique nickname/email, optional Argon2 password hash. |
| `pending_registrations` | Hashed email code/challenge, proposed fields, expiry, attempt count. |
| `web_sessions` | SHA-256 hashes of browser session tokens and expiry. |
| `oauth_authorization_codes` | Hashed one-time desktop OAuth codes and PKCE challenge. |
| `app_tokens` | SHA-256 hashes of desktop bearer tokens, expiry, and use metadata. |
| `github_accounts` | One Patchwork UUID to one globally unique numeric GitHub user ID. |
| `github_oauth_states` | Hashed one-time GitHub state, Patchwork UUID, completion URL, expiry. |

Raw session tokens, app tokens, authorization codes, email codes, and GitHub
states are never stored. GitHub OAuth user tokens and GitHub App installation
tokens are not persisted at all.

## Game authentication tables

| Table | Purpose |
| --- | --- |
| `game_server_instances` | Ephemeral random server UUID, SHA-256 secret hash, active/expired/closed state, heartbeat timestamp, and renewable lease expiry. |
| `game_launch_tickets` | SHA-256 one-use launcher ticket hash, account UUID, 60-second expiry and consumption time. |
| `game_process_sessions` | UUID, SHA-256 process-token hash, account UUID, expiry/use/revocation state. |
| `game_handshakes` | Registered server key/nonce, optional authorized client/account/process binding, direct/transfer kind, expiry and state. |
| `game_player_sessions` | Persistent admission UUID, current authoritative server, and active/disconnected state for an account/process. |
| `game_transfer_tickets` | SHA-256 ticket hash, source binding, nullable target server/handshake assigned at authorization, total/reservation expiry, and created/reserved/consumed/expired state. |

Launch consumption, handshake authorization/redemption, transfer reservation,
and player-server movement use transactions plus status predicates. The
database therefore enforces one-use semantics even when duplicate requests are
concurrent. X25519 private keys, shared secrets, HKDF output, AES keys, and game
packets never reach this database.

Server IDs and secrets are not configured records. A running third-party
server registers a fresh instance and keeps the plaintext secret in RAM. The
periodic cleanup marks missed leases expired, disconnects their player
sessions, and eventually removes retained terminal records. The transfer
target remains null until the client authorizes a real handshake registered by
server B, preserving make-before-break semantics on server A.

## Registry tables

### `repositories`

One stable external source repository:

- internal UUID primary key;
- provider, currently constrained to `github`;
- numeric `provider_repository_id`;
- cached owner, repository name, and canonical URL;
- created/updated timestamps;
- unique `(provider, provider_repository_id)` constraint.

### `mods`

Permanent mod identity:

- string ID primary key;
- owning Patchwork publisher UUID;
- repository UUID;
- source base path used by Rescan;
- optional latest version UUID;
- non-negative download count;
- creation timestamp.

Title, dependencies, paths, and source checksums are deliberately not stored on
`mods`, because they may differ by version.

The download count is incremented with one atomic SQL update after a remote mod
source archive has been assembled successfully. The equivalent `modpacks`
counter is incremented after its published manifest blob has been fetched.
Failed responses, README/image views, and local-folder installs do not count.

### `mod_versions`

One immutable release:

- internal UUID;
- mod ID and semantic version;
- version-specific title and metadata JSON;
- repository directory and exact source commit;
- directory Git tree OID;
- Cargo manifest path, Git blob OID, and verified SHA-256;
- optional README path/blob OID;
- optional image path/blob OID;
- publishing Patchwork UUID and numeric GitHub user ID;
- publication and optional yank timestamps.

`UNIQUE(mod_id, version)` enforces version immutability under concurrent
publishes. Source fields are insert-only in registry operations.

Browse queries join `mods.latest_version_id` to the immutable version and
repository rows. Search matches permanent ID, latest title, and repository
owner/name; modpack search additionally matches the version description. This
means Browse never guesses a latest release from publication timestamps.

### `mod_version_dependencies`

Ordered `init`, `run`, and `ownership` relations for one version. Each row also
stores `target_kind = mod|modpack`; the primary
key prevents duplicate relation/target pairs, while a separate unique index
preserves one position within each dependency kind. Targets are strings and may
be temporarily absent from the registry.

The `provides` target remains in the immutable version `metadata_json` rather
than this lifecycle table. Registry project responses materialize it as a typed
`provides` dependency so launchers install the API crate together with its
provider. This also lets existing development databases expose the relation
without a schema migration.

### `registry_scans`

Private, temporary publication snapshots:

- scan UUID and publisher UUID;
- numeric GitHub user and repository IDs;
- cached repository coordinates;
- base path and requested default branch;
- exact resolved commit and selected base tree OID;
- JSON scan warnings/errors;
- creation, expiration, and optional publication timestamps.

Expired unpublished scans are opportunistically removed when a new scan is
created. Entry rows cascade with their parent scan.

### `registry_scan_entries`

Authoritative preview rows. Each stores its UUID, typed project ID,
version/title/description, paths, tree/blob/checksum coordinates, optional README/image coordinates, status,
metadata, dependencies, warnings, and errors. Publish accepts only these UUIDs
and reads all other values from the row.

## Modpack tables

`modpacks` is the permanent identity table: slug, publisher, repository,
Rescan base path, latest-version pointer, downloads, and creation time.
`modpack_versions` stores immutable SemVer releases with title, description,
exact commit/tree/manifest coordinates, optional `<id>.md` and image
coordinates, metadata, publisher identities, and yank state.
`modpack_version_dependencies` stores ordered `mod`, `modpack`, and `ignore`
relations plus a typed `mod|modpack` target. `UNIQUE(modpack_id, version)` gives
modpacks the same concurrent immutability guarantee as mods.

## Transactional publication

`Database::publish_registry_scan` opens one transaction and validates scan
ownership, linked GitHub identity, expiry, single use, entry membership/status,
and current registry state. It then:

1. upserts repository display coordinates by numeric ID;
2. inserts each new permanent mod or modpack when required;
3. inserts immutable typed version rows;
4. expands dependency JSON into normalized rows;
5. advances the latest version by semantic-version comparison;
6. marks the scan published.

Any conflict rolls back the complete operation. The exact commit and tree OID
come from the persisted scan, not from the publish request.

## SQLite

The server TOML normally contains:

```toml
[server]
db-connection = "./patchwork.sqlite"
```

Useful inspection commands:

```bash
sqlite3 patchwork-web/patchwork.sqlite ".tables"
sqlite3 patchwork-web/patchwork.sqlite \
  "select id, publisher_uuid, latest_version_id from mods;"
sqlite3 patchwork-web/patchwork.sqlite \
  "select mod_id, version, source_commit, source_tree_oid from mod_versions;"
sqlite3 patchwork-web/patchwork.sqlite \
  "select modpack_id, version, source_commit, source_tree_oid from modpack_versions;"
```

The Diesel CLI is optional because the server applies embedded migrations:

```bash
cargo install diesel_cli --no-default-features --features sqlite
DATABASE_URL=patchwork.sqlite diesel migration run \
  --migration-dir patchwork-database/migrations/sqlite
```

## MySQL

Build without default features:

```bash
cargo build --manifest-path patchwork-database/Cargo.toml \
  --no-default-features --features mysql
```

Configure a normal URL:

```toml
[server]
db-connection = "mysql://patchwork:password@127.0.0.1/patchwork"
```

SQLite and MySQL migrations expose the same Rust schema and operations.

## Synchronous Diesel in Actix

Diesel's current connections are synchronous. Registry scans spend most of
their time awaiting GitHub and perform short database operations, but higher
load should move Diesel calls to Actix blocking workers or a dedicated database
executor. This performance concern must not move authorization or authoritative
scan state into clients.
