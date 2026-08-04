# Getting started

Patchwork is split into independent crates, so only install the tooling needed
for the component you are working on. A complete development environment uses
Rust, Cargo, `cargo-leptos`, the Tauri CLI, a WebAssembly target, and mdBook.

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-leptos
cargo install tauri-cli
cargo install mdbook
```

Tauri also needs the platform packages documented by Tauri for the current
operating system, including a webview implementation.

## Compose from the CLI

From a project containing `mods/` and `modpacks/`:

```bash
patchwork compose \
  --mods-folder mods \
  --modpacks-folder modpacks \
  --modpack client \
  --cache build \
  --name client
```

The result is a regular Cargo project at `build/client`.

## Start the desktop launcher

```bash
cd patchwork-app
cargo tauri dev
```

Tauri runs `scripts/dev-frontend.sh` as its development frontend command. The
script builds only the Leptos frontend, copies the HTML entry point into
`dist/`, and serves it on `127.0.0.1:1420`. It reuses an already running server
on that address instead of starting a second one.

## Start the website

Create a private server configuration from the checked-in example:

```bash
cd patchwork-web
cp patchwork.example.toml patchwork.toml
cargo leptos build
cargo run --features server -- --config patchwork.toml
```

The default bind address is `0.0.0.0:8080`. The SQLite file and all migrations
are created automatically on first connection. Account registration requires a
Resend API key, while GitHub linking requires valid GitHub App credentials in
the TOML file; details are in
[Web service](./web_service.md) and [GitHub integration](./github_integration.md).

## Build this book

```bash
mdbook serve documentation --open
```

Without `--open`, mdBook prints the local URL and leaves browser choice to the
developer.
