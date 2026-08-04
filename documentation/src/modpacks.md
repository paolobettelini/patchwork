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
- `ignore`: mods to remove from the imported tree.

`name`, `description`, and `color` are descriptive metadata. They help the
launcher and browser, but they do not change graph resolution.

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

The core treats `name`, `version`, `description`, `color`, `modpacks`, `mods`, and `ignore`
as modpack metadata. The launcher also uses a filesystem convention to show
icons without putting binary data in TOML:

- a profile/modpack `client.toml` can have `client.png`, `client.jpg`,
  `client.jpeg`, `client.webp`, or `client.gif` next to it;
- a mod can have a favicon in its folder as `favicon.png`, `favicon.jpg`,
  `favicon.jpeg`, `favicon.webp`, or `favicon.gif`.

Favicons are decorative and do not take part in composition. If they are
missing, the launcher uses the default logo. Registry publication also accepts
`<id>.md` next to `<id>.toml` as that modpack's README. Registry images use
`png`, `webp`, `jpg`, then `jpeg`; SVG is not accepted.
