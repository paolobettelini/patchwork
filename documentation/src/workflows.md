# Practical workflows

## Compose client and server

From the project that contains `mods/` and `modpacks/`:

```bash
patchwork compose \
  --mods-folder mods/ \
  --modpacks-folder modpacks/ \
  --modpack client \
  --cache build

patchwork compose \
  --mods-folder mods/ \
  --modpacks-folder modpacks/ \
  --modpack server \
  --cache build
```

Then build or check the generated projects:

```bash
cargo check --manifest-path build/client/Cargo.toml
cargo check --manifest-path build/server/Cargo.toml
```

## Add a simple mod

1. Create a new crate in `mods/my-mod`.
2. Add `[package.metadata.mod]` to its `Cargo.toml`.
3. Implement `EntryType::init()` and `EntryType::run()`.
4. Add the mod to a modpack.
5. Run `patchwork compose`.
6. Run `cargo check` on the generated project.

## Add a feature with a replaceable API

1. Define an API crate and add `[package.metadata.mod] api = true`.
2. Consumer mods depend on the API name in metadata.
3. A concrete mod declares `provides = "api-name"`.
4. The modpack selects exactly one provider.

Example:

```toml
[package.metadata.mod]
entry = "StdInventoryMod"
provides = "inventory-api"
```

## Add codegen

1. Create a generic helper crate if needed.
2. Create a generator crate, for example `items-codegen-utils`.
3. Choose the domain metadata, for example
   `[package.metadata.items.definitions]`.
4. Make the generator read the composed project and its path dependencies.
5. Generate a standalone crate, for example `items-generated`.
6. Add `[[package.metadata.mod.codegen]]` to a mod in that domain.
7. Create a dev crate with the same name in `mods/` if you want language server
   support.
8. Consumer mods depend on `items-generated = "0.1.0"`.
9. Patchwork patches that dependency to the generated output.

If generated code names types from ordinary helper libraries, keep those
libraries as Git or registry dependencies in the contributor manifest. The
generator should preserve that source in its generated `Cargo.toml`; only the
selected contributor mods themselves must be discovered through local paths.

## Maintain local mod and library dependencies

Use sibling paths for Patchwork mods and distributable Git/version sources for
plain Rust libraries. A local checkout can patch the Git source back to its
adjacent library directories through `.cargo/config.toml`.

This matters for registry installations: Patchwork downloads normal, API, and
support mods, but Cargo must be able to fetch helper libraries that are not
Patchwork projects.

For a desktop development profile, a practical setup is:

1. Replace the default mod and modpack cache directories with symbolic links to
   the development checkout. For example, preserving the existing downloaded
   caches as backups:

   ```bash
   PATCHWORK_DATA="${XDG_DATA_HOME:-$HOME/.local/share}/patchwork"

   rm -rf "$PATCHWORK_DATA/cache/mods" && ln -s /some/local/mods "$PATCHWORK_DATA/cache"
   rm -rf "$PATCHWORK_DATA/cache/modpacks" && ln -s /some/local/modpacks "$PATCHWORK_DATA/cache"
   ```

   The launcher's
   cache clear actions intentionally refuse to delete symbolic-link targets.
2. Add `/some/local`, or the appropriate checkout root, to **Registries / Local
   folders** so Browse resolves development projects locally before the remote
   registry.
3. Open the profile's **Options** tab and add one compilation argument row:
   `--config /some/local/mods/.cargo/config.toml`.

Patchwork mods are downloaded and composed as complete crate directories, but
ordinary Rust libraries are not Patchwork registry projects and are not
downloaded together with the mods. Their distributable `Cargo.toml`
dependencies should therefore use a Git address. During local development it
is useful to keep using the adjacent library checkout instead.

For a Modularis checkout, copy
[`scripts/sync-modularis-deps.sh`](../../scripts/sync-modularis-deps.sh) into
`mods/.cargo/` and run it there:

```bash
mkdir -p /some/local/mods/.cargo
cp /path/to/modding_system/scripts/sync-modularis-deps.sh /some/local/mods/.cargo/
chmod +x /some/local/mods/.cargo/sync-modularis-deps.sh

/some/local/mods/.cargo/sync-modularis-deps.sh --dry-run
/some/local/mods/.cargo/sync-modularis-deps.sh
```

The script is specific to the Modularis Git repository. It scans the crates
directly below `mods/`, keeps dependencies on Patchwork mods as sibling `path`
dependencies, rewrites dependencies on plain libraries to the remote Git
address, and generates `mods/.cargo/config.toml` with local `[patch]` entries
for those libraries. Because it can rewrite the child `Cargo.toml` files, review
the `--dry-run` output before applying it.

The explicit config argument matters because Cargo Build runs with the composed
project as its working directory. Cargo would not discover a `.cargo` directory
under the separate mod cache by walking that directory. Keep repository-local
plain-library overrides in the selected config:

```toml
[patch."https://github.com/example/project.git"]
codegen-utils = { path = "/absolute/path/to/mods/codegen-utils" }
```

Using an absolute config path makes the profile independent of the composed
build cache location. The launcher splits that row into separate `argv` values
without invoking a shell. Quote the path when it contains spaces. This solution
is profile-specific: ordinary downloaded profiles continue to use their
published Git dependencies, while a development profile can opt into local
libraries.

## Debug

When something does not work, check in this order:

1. Does the modpack really include the mod?
2. Does the mod have `[package.metadata.mod]`?
3. Do dependency names match crate names or API providers?
4. Is there exactly one provider for each API?
5. Does any mod take ownership of something that is also used in `run`?
6. Was the generated crate written in `build-*/`?
7. Does the composed project's `Cargo.toml` contain the patch to the generated
   crate?
8. Do payloads used by codegen live in leaf crates?

## Run the desktop launcher

```bash
cd patchwork-app
cargo tauri dev
```

If port 1420 is already occupied, check whether it is the existing Patchwork
frontend before terminating anything. `scripts/dev-frontend.sh` deliberately
reuses a responding server on that address.

For a debug native build without packaging:

```bash
cd patchwork-app
scripts/tauri-build-debug.sh
```

## Run the web service

```bash
cd patchwork-web
cp patchwork.example.toml patchwork.toml
# Add real server-only GitHub App credentials to patchwork.toml.
cargo leptos build
cargo run --features server -- --config patchwork.toml
```

Override only the bind socket when needed:

```bash
cargo run --features server -- \
  --config patchwork.toml \
  --address 127.0.0.1 \
  --port 3000 \
  --base-path /patchwork
```

Address, port, and base path override their TOML values. The database
connection still comes only from the TOML file. Set
`--secure-cookies true` when the public site is served over HTTPS.

## Inspect the local database

```bash
sqlite3 patchwork-web/patchwork.sqlite ".tables"
sqlite3 patchwork-web/patchwork.sqlite \
  "select uuid, nickname, email from accounts;"
```

Manual migration commands are not needed during normal startup because the
database crate embeds and applies migrations automatically.

## Publish mods and modpacks from GitHub

1. Sign in to Patchwork on the website or desktop launcher.
2. Connect the GitHub identity that has repository write access.
3. Install the Patchwork GitHub App for the repository/account or organization.
4. Open **Upload** and enter `https://github.com/owner/repository`.
5. Optionally enter a subtree containing projects, such as `mods`, or one loose
   manifest such as `modpacks/client.toml`.
6. Select **Scan** and inspect the exact commit, statuses, dependencies, and
   warnings returned by the backend.
7. Leave only desired `NEW MOD` and `NEW VERSION` entries enabled.
8. Select **Publish / Update** before the 20-minute scan expires.

For the GitHub App itself, set the OAuth Callback URL to
`http://localhost:8080/github/callback`, the Setup URL to
`http://localhost:8080/github/installation-complete`, and disable **Request user
authorization during installation**. Installing the App and connecting a
GitHub identity are separate operations.

If an existing version reports `VERSION CONFLICT`, change `package.version` in
the mod's `Cargo.toml` or top-level `version` in `<id>.toml`, commit, and scan
again. Never replace the tree/checksum of a published version.

Use **Rescan** on either profile section after pushing a new version. It opens
Upload with the canonical repository and the individual crate directory or
loose modpack TOML already filled in; press **Scan** after reviewing them.
Rescan does not remove versions that disappeared from the current branch.

## Browse remote and local registries

1. In desktop Settings / Registries, set Backend and add any local registry
   roots. The final empty local-folder row is intentional and creates another
   row as it is filled.
2. Open Browse, enter a keyword or project ID, and select Mods and/or Modpacks.
3. On desktop, select a profile and add a mod or modpack dependency. For a
   modpack, **Download as profile** instead creates a standalone profile and
   carries its optional icon and README.
4. Follow the top-bar progress while the selected project and its complete
   transitive graph are resolved from cache, local folders, then the backend.
   Partial failures are reported without discarding successful downloads.
5. Use Refresh on a profile to check available SemVer versions. If
   **Download updates** appears, install them through the same verified flow.
6. Configure Download/Compose/Build from the gear beside the primary action.
   The default pipeline executes all three in order.

The website exposes the same search and filters but no download controls.

## Test account and GitHub flows

For browser auth, register from the site, reload it to verify the HttpOnly
session persists, change the nickname, then logout from the profile menu.

For desktop auth, keep the web backend running, use Sign in from the launcher,
approve access in the browser, and verify the launcher profile changes without
a restart. Connect GitHub from both clients separately: the web flow returns to
`/profile`, while the desktop flow returns to its random one-shot loopback
listener and refreshes through a Tauri event.

For publication testing, use a repository covered by the GitHub App
installation and linked-user permission. Confirm that read-only users are
rejected, `UNCHANGED` entries cannot be selected, a changed tree with the same
version reports a conflict, and a version bump produces `NEW VERSION`.

## Build the documentation

```bash
mdbook build documentation
```

Use `mdbook serve documentation` while editing to get automatic rebuilds.
