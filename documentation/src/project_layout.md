# Project layout

A typical setup has two parts: the composition framework and one or more
projects that contain mods and modpacks.

```text
modding_system/
  patchwork/            generic composition library
  patchwork-cli/        command-line frontend
  patchwork-app/        Leptos + Tauri desktop launcher
  patchwork-ui/         shared Leptos UI components
  patchwork-registry-types/ shared registry API DTOs
  patchwork-web/        Leptos frontend + Actix backend
  patchwork-database/   Diesel data access and migrations
  template/             base composed Cargo project
  documentation/        this mdBook

minecraft_simple_demo/
  mods/
  modpacks/
```

## Framework

`modding_system/patchwork` contains the generic logic:

- metadata parsing;
- modpack loading;
- graph resolution;
- final project generation;
- abstract codegen handling.

`modding_system/patchwork-cli` exposes this logic from the command line through
the `patchwork` command.

`modding_system/patchwork-app` is the desktop launcher. It uses the core
library, but it should only contain UI logic, local settings, browsing for
profiles/modpacks/mods, and compose/build/run commands.

`modding_system/patchwork-ui` contains components shared by the desktop and web
Leptos frontends. Shared components own presentation and reusable state, not
Tauri commands or Actix handlers.

`modding_system/patchwork-registry-types` contains only serializable scan,
preview, dependency, status, and publish DTOs. It is shared by Actix, browser
WebAssembly, Tauri WebAssembly, and the native Tauri proxy without bringing
database models or server secrets into frontend builds.

`modding_system/patchwork-web` contains both the browser frontend and the Actix
server. The server owns HTTP authentication, GitHub secrets, static asset
serving, and access to `patchwork-database`.

`modding_system/patchwork-database` is the synchronous Diesel data layer. It
supports SQLite by default and MySQL behind a feature flag. Migrations are
embedded and run when `Database::connect` is called.

`modding_system/template` contains the base template of the final crate. The tool
copies it and injects dependencies and glue code. When the project is composed,
the final crate gets the name chosen for the modpack/profile. It does not have
to stay named `template`.

## Project with mods

`minecraft_simple_demo/mods` contains Rust crates. Some are real mods, while
others are API crates, support crates, or development copies of generated
crates:

- `bevy-mod`: wrapper mod around Bevy;
- `client-*`: mods that live only on the client side;
- `server-*`: mods that live only on the server side;
- `*-api`: API crates or abstract contracts;
- `*-impl`: concrete providers for an API;
- `*-events-mod`: shared ECS/event contracts;
- `*-network-messages-mod`: mods that export messages for network codegen;
- `*-registry-codegen`: mods/generators that aggregate typed registries.

`minecraft_simple_demo/modpacks` contains TOML modpacks. For example:

```toml
name = "Minecraft client"
version = "0.1.0"
description = "Local Bevy client."
modpacks = ["common"]
ignore = []

mods = [
    "client-game-bootstrap-mod",
    "client-network-udp-impl",
]
```

Build/cache folders are reproducible output. In the desktop launcher composed
projects live under `cache/build`; generated crates for the same composition
live alongside them. A direct CLI invocation may use any directory supplied to
`--cache`.

## Desktop data

On Linux the launcher defaults to:

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
  profiles/
  target/
```

This is Patchwork-owned application data, not Tauri's application directory.
The exact base directory follows the operating system's local data convention.
See [Desktop launcher](./desktop_launcher.md) for the role of each file.

## Practical rule

If something is generic composition logic, it belongs in
`modding_system/patchwork`.

If something talks about the game domain or a specific feature, it belongs in a
mod or in a support crate inside `mods/`.

HTTP transport and browser authentication belong in `patchwork-web`; reusable
persistence belongs in `patchwork-database`; host-specific desktop operations
belong in the Tauri backend of `patchwork-app`. Registry parsing belongs in the
Patchwork core, registry policy/orchestration in the Actix backend, and only
registry presentation belongs in `patchwork-ui`.
