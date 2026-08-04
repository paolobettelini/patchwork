# Metadata reference

## Mod metadata

```toml
[package.metadata.mod]
entry = "EntryType"
provides = "optional-api-name"
support = false
api = false

[package.metadata.mod.dependencies]
init = []
run = []
ownership = []
```

Fields:

- `entry`: Rust type used by Patchwork. Required for normal mods and forbidden
  for `support` and `api` mods;
- `provides`: optional API implemented by a normal lifecycle mod. When the
  provider is selected, the target is selected transitively and must be a mod
  with `api = true`;
- `support`: marks a selected support mod with no lifecycle object. Support
  mods are included in the composed Cargo project and can contribute assets or
  codegen, but Patchwork does not generate `init()` or `run()` calls for them;
- `api`: marks a selected API contract with no lifecycle object. Exactly one
  selected normal mod must declare `provides = "<api-id>"`;
- `dependencies.init`: dependencies passed to `init()` as `&mut`;
- `dependencies.run`: dependencies passed to `run()` as `Arc`;
- `dependencies.ownership`: dependencies passed to `run()` by value.

`support` and `api` are mutually exclusive. Neither kind may declare `entry` or
`provides`. Support mods are useful for selected asset-only crates, codegen
inputs, or other content that belongs to a modpack without exposing a runtime
object. They are selected directly by a modpack and cannot be used in the
`init`, `run`, or `ownership` lists, because those lists pass lifecycle objects.

API mods are real selected mods and Cargo dependencies. A dependency may name
their ID; Patchwork then passes the runtime object created by the single
selected provider. The API crate itself is still included in composition and
in registry/cache downloads. Conversely, selecting a provider automatically
adds its `provides` target to composition and dependency downloads.

Mod IDs containing the lowercase substring `generated` are reserved for
Compose-time codegen outputs. They cannot be published and are not downloadable
or navigable registry projects. See [Generic codegen](./codegen.md).

## Codegen metadata

```toml
[[package.metadata.mod.codegen]]
crate = "generated-crate"
version = "0.1.0"
dev_crate = "generated-crate"

[package.metadata.mod.codegen.generator]
crate = "domain-codegen-utils"
command = "generate"
```

Fields:

- `crate`: name of the generated crate;
- `version`: version written in the generated `Cargo.toml`;
- `dev_crate`: crate in `mods/` to update for development;
- `generator.crate`: generator crate;
- `generator.command`: subcommand passed to the generator.

## Modpack

```toml
name = "Human name"
version = "0.1.0"
description = "Shown in the launcher"
color = "#02a9a9"
modpacks = ["common"]
ignore = ["mod-to-remove"]

mods = [
    "some-mod",
    "modpack/common",
]

[options.build]
args = ["--config /absolute/path/to/.cargo/config.toml"]

[options.build.env]
RUST_LOG = "debug"

[options.run]
args = ["--server", "example.test:25565"]

[options.run.env]
GAME_LOG = "trace"
```

`modpack/common` expands `modpacks/common.toml`.

Fields:

- `name`: optional human title;
- `version`: required semantic version string;
- `description`: optional description;
- `color`: optional hex color for the launcher;
- `modpacks`: imported modpacks;
- `ignore`: mods excluded after import;
- `mods`: explicitly selected mods;
- `options.build.args`: custom arguments appended to `cargo build`;
- `options.build.env`: custom environment variables for Cargo Build;
- `options.run.args`: custom arguments passed to the cached executable;
- `options.run.env`: custom environment variables for the cached executable.

Each array item is split into `argv` values using shell-style quotes and
escapes, without invoking a shell or expanding variables. Options are used only
when this file is the desktop root profile.
Imported modpack options are not merged. Patchwork's read-only defaults are not
serialized under `options` and their environment names are reserved.

## Assets and favicons

These are filesystem conventions, not TOML metadata:

- `mods/<mod>/assets/**` is copied to `assets/<mod>/` in the composed project;
- `mods/<mod>/favicon.{png,jpg,jpeg,webp,gif}` is the mod favicon in the
  launcher;
- `modpacks/<id>.{png,jpg,jpeg,webp,gif}` is the favicon for
  `modpacks/<id>.toml`.
- registry publication recognizes `<id>.{png,webp,jpg,jpeg}` and `<id>.md`
  beside a loose `<id>.toml`; it stores immutable Git coordinates, not bytes.

## Network metadata

This is not generic framework metadata. It is metadata for the network domain:

```toml
[package.metadata.network.messages]
clientbound = [
    "some_types::ServerToClient",
]
serverbound = [
    "some_types::ClientToServer",
]
```

Patchwork ignores it. The network domain generator reads it.
