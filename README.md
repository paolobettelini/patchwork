# Patchwork

Patchwork is an experimental source-level modding platform for Rust software.
Instead of loading binary plugins at runtime, it composes the selected mods into
a normal Cargo project and then compiles the complete application.

<div align="center">
  <img src="./media/app_preview.png" alt="Patchwork desktop app" width="600">
</div>

The core idea is that every mechanic or object can be a distinct Rust crate.
Mods depend on other mods or abstract APIs, modpacks choose the concrete
implementations, and Rust type-checks the final generated program.

## Components

```text
modding_system/
  patchwork/            core composition library
  patchwork-cli/        `patchwork compose` command
  patchwork-app/        Leptos + Tauri desktop launcher
  patchwork-ui/         shared Leptos components
  patchwork-registry-types/ shared registry API DTOs
  patchwork-web/        Leptos website + Actix backend
  patchwork-database/   Diesel models and migrations
  template/             base generated Cargo project
  documentation/        mdBook documentation
```

## Documentation

The [mdBook summary](documentation/src/SUMMARY.md) links the complete
documentation for the composition model, metadata, desktop launcher, web
configuration, accounts, OAuth, GitHub integration, database, and backend API.

Build or serve it locally with:

```bash
mdbook build documentation
mdbook serve documentation
```

## Compose from the CLI

```bash
patchwork compose \
  --mods-folder mods \
  --modpacks-folder modpacks \
  --modpack client \
  --cache build \
  --name client
```

The output is a regular Cargo project:

```bash
cargo check --manifest-path build/client/Cargo.toml
```

## Start the desktop app

```bash
cd patchwork-app
cargo tauri dev
```

## Start the website

Create a private server configuration, fill in the Resend and GitHub App
credentials, then build the Leptos assets and run Actix:

```bash
cd patchwork-web
cp patchwork.example.toml patchwork.toml
cargo leptos build
cargo run --features server -- --config patchwork.toml
```

The default bind address is `0.0.0.0:8080`. Server, email, database, and GitHub
backend configuration are read from the TOML file. Embedded migrations run
automatically when the server connects.

During early development the database uses a single baseline migration. Delete
the local SQLite file and sign in again after schema changes; the mdBook's
[database chapter](documentation/src/database.md) documents this policy.

Authenticated users with a linked GitHub account can scan and publish mods and
loose versioned modpack TOMLs from Upload on either client. Patchwork pins the default branch to an exact commit,
walks GitHub trees without cloning, previews immutable versions, and publishes
only selected server-side scan entries. The complete contract is in
[Registry publication](documentation/src/registry_publication.md).

## Mod metadata

A lifecycle mod declares an entry type and its dependencies:

```toml
[package.metadata.mod]
entry = "EntryType"
provides = "optional-api-name"

[package.metadata.mod.dependencies]
init = []
run = []
ownership = []
```

An API contract with no lifecycle object uses:

```toml
[package.metadata.mod]
api = true
```

An asset-only, codegen-only, or other selected mod with no lifecycle object
instead uses `support = true`. Both are real selected mods and Cargo
dependencies, but Patchwork does not generate `init()` or `run()` calls for
them. The flags are mutually exclusive, and every API mod requires exactly one
selected normal provider declared with `provides = "<api-id>"`. See the
[metadata reference](documentation/src/metadata_reference.md) for the complete
format, providers, modpacks, codegen, assets, and favicons.

Dependencies between selected Patchwork mods should use sibling Cargo paths so
Compose and domain codegen can inspect their manifests. Plain helper libraries
should use a distributable Git or registry source; local development can patch
that source back to the checkout with `.cargo/config.toml`. Generated crates
preserve these library sources instead of requiring every dependency to be a
downloaded Patchwork mod. See [Mods and Cargo metadata](documentation/src/mods.md)
and [Generic codegen](documentation/src/codegen.md).
