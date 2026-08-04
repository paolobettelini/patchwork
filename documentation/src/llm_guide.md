# Guide for LLMs and contributors

This page is a checklist for anyone changing the project without breaking the
paradigm.

## Before changing code

Read:

1. the `Cargo.toml` of the mod involved;
2. the modpack that includes it;
3. the generated code in `<cache>/<project-name>`;
4. any generated crates next to the composed project;
5. the domain generator, if the feature uses codegen.

For launcher, website, account, or catalogue changes, also read the ownership
boundary before editing:

- reusable Leptos presentation belongs in `patchwork-ui`;
- desktop filesystem/process/browser work belongs in `patchwork-app/src-tauri`;
- HTTP handlers and secrets belong in the server feature of `patchwork-web`;
- persistence operations and migrations belong in `patchwork-database`.

Do not assume that a specific feature belongs in the framework. If the feature
talks about networking, items, ECS, gameplay, or serializers, it probably
belongs in a mod.

## Questions to ask

- Is this logic generic for all mods, or is it domain-specific?
- Does Patchwork need to know it, or is a metadata-declared generator enough?
- Am I using crate names and Rust types instead of hardcoded relative paths?
- Am I introducing a cycle between a mod, a types crate, and a generated crate?
- Can mods that use the generated crate also see it during development?
- Does the selected modpack include all needed mods?
- Should client and server share this codegen or have separate generated crates?
- Is this feature a replaceable provider? If yes, it needs an API.
- Is this feature a contribution to a registry? If yes, prefer codegen.
- Is an identifier mutable display metadata or a stable identity? Patchwork
  accounts use UUIDs; GitHub users and repositories use numeric IDs.
- Does a browser action need a cookie, while a desktop action needs a bearer
  token and loopback callback?
- Could this value expose a password equivalent, GitHub secret, private key,
  app JWT, or access token to WebAssembly?
- Is a publication value derived from the persisted authoritative scan, or am I
  accidentally trusting metadata sent again by a client?
- Does a source coordinate use the exact commit and numeric repository ID, or
  a mutable branch/owner/name?

## Things not to do

- Do not add domain metadata to Patchwork's generic model.
- Do not put network messages, items, or gameplay registries in `modding_system`.
- Do not bring back `mod.json`.
- Do not use relative paths in codegen metadata when a crate name is enough.
- Do not put aggregated payloads in the same crate that depends on the generated
  crate.
- Do not use string-based dynamic dispatch if you can generate a typed enum.
- Do not edit generated output as a permanent fix.
- Do not expose backend configuration or GitHub credentials through shared
  frontend types, HTML, JavaScript, WebAssembly, logs, or redirects.
- Do not store desktop auth in Tauri's app directory or browser storage; use
  the Patchwork-owned `config/auth.json` path.
- Do not identify accounts by nickname or GitHub users by login when a stable
  UUID or numeric ID exists.
- Do not identify GitHub repositories only by `owner/name`; persist the numeric
  repository ID.
- Do not clone repositories in the registry scanner or download every source
  file to invent a directory SHA-256. Use Git trees and call the directory
  checksum `source_tree_oid`.
- Do not accept title, version, dependency, path, commit, or checksum fields in
  a publish request. Accept persisted scan-entry UUIDs only.
- Do not update an existing `(mod_id, version)` or `(modpack_id, version)` to
  different source. Require a semantic-version bump.
- Do not parse ANSI output manually or split PTY output into stdout/stderr
  lines; preserve the ordered raw byte stream for xterm.js.

## Things to do

- Use `[package.metadata.mod]` for generic mod metadata.
- Use `[package.metadata.<domain>]` for domain-specific metadata.
- Put payloads in leaf crates.
- Generate standalone crates.
- Use `dev_crate` to keep rust-analyzer useful.
- Use `mods/<mod>/assets/` so Patchwork copies files to `assets/<mod>/`.
- Use filesystem favicons for the launcher/browser, not binary TOML fields.
- Verify with `patchwork compose` and `cargo check --manifest-path
  <cache>/<project-name>/Cargo.toml`.
- Add backend routes before Actix's SPA fallback and document them in
  [Backend routes](./backend_routes.md).
- Keep OAuth codes, states, session tokens, and app tokens short-lived or
  revocable as appropriate, and store only hashes server-side.
- Resolve mutable GitHub refs once, then scan and publish from the exact commit.
- Keep dependency rows on immutable versions, not permanent mod/modpack identities.
- Revalidate scan ownership, expiry, single use, selected entry membership, and
  current database state inside the publish transaction.

## Correct mental model

Patchwork is an orchestrator.

Mods are source code.

The modpack is the selection.

Codegen is domain-specific.

The final result is a normal Rust program.

The desktop launcher is a trusted native host around an untrusted webview.

The web frontend is a public client; server secrets remain in Actix.

The database is persistence plus final transactional invariants: HTTP handlers
authenticate and authorize operations, while unique constraints and publish
transactions defend immutable registry state under concurrency.
