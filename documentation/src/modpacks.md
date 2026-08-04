# Modpacks

A modpack is a TOML file that selects the mods to compose and can import other
modpacks.

```toml
name = "Client"
version = "0.1.0"
description = "Client-side modpack."
color = "#02a9a9"
modpacks = ["common"]
ignore = []

mods = [
    "main-client-mod",
    "network-mod-client",
]
```

Patchwork can receive either a direct path to a TOML file or an ID. If it
receives an ID like `client`, it looks for `modpacks/client.toml`.

```bash
patchwork compose \
  --mods-folder mods/ \
  --modpacks-folder modpacks/ \
  --modpack client \
  --cache build
```

Supported generic fields:

- `name`: human title of the modpack;
- `version`: required semantic version used by the registry;
- `description`: description shown by the launcher;
- `color`: optional color used by the launcher when the modpack is selected;
- `modpacks`: other modpacks to import;
- `mods`: explicitly selected mods;
- `ignore`: mods to remove from the imported tree;
- `options`: optional per-profile process environment and arguments used by the
  desktop launcher.

`name`, `description`, and `color` are descriptive metadata. They help the
launcher and browser, but they do not change graph resolution.

## Profile process options

A modpack can carry build and executable options. They become active when that
TOML file is installed or opened as a desktop profile:

```toml
[options.build]
args = ["--config /absolute/path/to/mods/.cargo/config.toml"]

[options.build.env]
RUST_LOG = "debug"

[options.run]
args = ["--connect", "127.0.0.1:25565"]

[options.run.env]
GAME_LOG = "trace"
```

Every item in `args` is a command-line fragment. Patchwork splits whitespace
and supports single quotes, double quotes, and backslash escapes, but does not
perform variable expansion or invoke a shell. Build arguments are appended to
`cargo build` after launcher-managed arguments such as `--release`; run
arguments are passed to the cached executable.

The `env` tables contain only custom values. Launcher-managed variables remain
visible as read-only rows in the profile's **Options** tab and cannot be
overridden. The registry rejects published modpacks that attempt such an
override. Empty `options` are omitted when the launcher writes the profile.

Options belong to the root profile being built or run. Imported modpacks do not
merge their process options into the parent profile. This keeps command-line
behavior deterministic when multiple modpacks are combined. A published
modpack still retains its options, so **Download as profile** preserves them;
adding it as a dependency of an existing profile does not activate them.

## Imported modpacks

The `modpacks` field imports other modpacks, identified relative to the folder
passed with `--modpacks-folder`.

```toml
name = "Client"
version = "0.1.0"
description = "Client-side modpack."
modpacks = ["common", "blocks"]
ignore = []

mods = [
    "main-client-mod",
]
```

This is the same as including all mods from the imported modpacks plus the mods
listed in `mods`. Patchwork removes duplicates and detects circular references
between modpacks, showing the cycle chain.

For compatibility, inside `mods` and inside `init`, `run`, and `ownership`
dependency lists, you can still reference another modpack with `modpack/id`:

```toml
mods = [
    "modpack/common",
    "main-client-mod",
]
```

This includes all mods from `common.toml`.

The same form can be used in mod metadata:

```toml
[package.metadata.mod.dependencies]
init = ["modpack/client-foundation"]
run = []
ownership = []
```

Patchwork expands these references, removes duplicates, and detects circular
references between modpacks.

## Ignore

The `ignore` field removes mods from the resolved tree. It is useful when an
imported modpack contains an API provider that you want to replace.

```toml
name = "Custom client"
version = "0.1.0"
description = "Client using a custom network provider."
modpacks = ["client"]
ignore = ["network-mod-client"]

mods = [
    "network-custom-client",
]
```

If a removed mod is still directly required by another mod, Patchwork fails with
a structured missing dependency error. If the dependency is expressed through an
API, another selected provider can satisfy it.

## Client and server

Client and server can share some mods and differ in others:

```text
client.toml:
  bevy-mod
  main-client-mod
  network-vanilla-mod
  network-mod-client

server.toml:
  bevy-mod
  main-server-mod
  network-vanilla-mod
  network-mod-server
```

The key point is that codegen runs on the composed modpack. If the client and
server select different mods, they can generate different crates or the same
crates depending on the metadata present in each modpack.

## Icons and favicons

The core treats `name`, `version`, `description`, `color`, `modpacks`, `mods`,
`ignore`, and `options` as modpack metadata. Process options do not affect graph
resolution or composition. The launcher also uses a filesystem convention to
show icons without putting binary data in TOML:

- a profile/modpack `client.toml` can have `client.png`, `client.jpg`,
  `client.jpeg`, `client.webp`, or `client.gif` next to it;
- a mod can have a favicon in its folder as `favicon.png`, `favicon.jpg`,
  `favicon.jpeg`, `favicon.webp`, or `favicon.gif`.

Favicons are decorative and do not take part in composition. If they are
missing, the launcher uses the default logo. Registry publication also accepts
`<id>.md` next to `<id>.toml` as that modpack's README. Registry images use
`png`, `webp`, `jpg`, then `jpeg`; SVG is not accepted.
