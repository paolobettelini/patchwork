# Mods and Cargo metadata

A mod is a Cargo crate that declares metadata under `[package.metadata.mod]`.

Minimal example:

```toml
[package]
name = "logger-mod"
version = "0.1.0"
edition = "2024"

[package.metadata.mod]
entry = "LoggerMod"

[package.metadata.mod.dependencies]
init = []
run = []
ownership = []
```

The `entry` field is the Rust type that Patchwork uses in the final glue code.
That type must expose methods compatible with the current model:

```rust
pub struct LoggerMod;

impl LoggerMod {
    pub fn init() -> Self {
        Self
    }

    pub fn run(&self) -> Option<Vec<tokio::task::JoinHandle<()>>> {
        None
    }
}
```

Patchwork does not require a formal trait yet. It generates direct calls like:

```rust
let mut logger_mod = logger_mod::LoggerMod::init();
let logger_mod = std::sync::Arc::new(logger_mod);
let mod_handles = logger_mod.run();
```

## Mods, APIs, and providers

An API crate can be a Patchwork mod without implementing lifecycle glue:

```toml
[package]
name = "command-api"
version = "0.1.0"
edition = "2024"

[package.metadata.mod]
api = true
```

API mods are selected like normal mods and are included as Cargo dependencies.
Patchwork does not call `init()` or `run()` for them, and they must not declare
`entry` or `provides`. Every selected API mod requires exactly one selected
normal implementation.

A normal mod can provide an abstract API through `provides`:

```toml
[package.metadata.mod]
entry = "StdCommandProcessorMod"
provides = "command-api"

[package.metadata.mod.dependencies]
init = []
run = []
ownership = []
```

Other mods can depend on `"command-api"` instead of depending on the concrete
crate. Patchwork resolves the API to the mod that provides it. A modpack must
contain exactly one provider for each API.

`provides` is also an implicit source dependency. Selecting or downloading
`StdCommandProcessorMod` automatically selects/downloads `command-api`, even if
no consumer names the API in its lifecycle lists. This matters for providers
whose Cargo manifest uses a sibling path such as `../command-api`: a remote
cache must contain that API crate just like a local checkout does.

This gives compile-time dependency injection: consumer mods depend on the
concept, and the modpack decides the concrete implementation.

## Support mods and helper crates

Not every crate in `mods/` has to be a Patchwork mod. A private helper crate or
codegen library can still live in `mods/` without `[package.metadata.mod]`.
Use `support = true` when the crate should appear in Patchwork as an effective
mod without a lifecycle object, for example an asset-only or codegen-only
crate. Use `api = true`, not `support`, for replaceable API contracts. The two
flags are mutually exclusive.

Support mods are included as Cargo dependencies, their assets are copied, and
their codegen declarations run. They cannot appear in lifecycle dependency
lists because there is no object to pass to `init()` or `run()`.

Examples:

- `network-vanilla-types` contains only serializable structs;
- `codegen-utils` contains generic helpers;
- `network-codegen-utils` is a binary/library used as a generator.

Patchwork loads as mods only the crates selected by the modpack and containing
mod metadata. Normal, support, and API mods are all registry projects and are
all downloaded into the launcher cache when selected directly, through a
lifecycle dependency, or as the API target of `provides`.

The distinction also determines how repository-local Cargo dependencies should
be written:

- dependencies on Patchwork mods use sibling `path` sources;
- dependencies on plain helper libraries use a distributable Cargo source,
  normally `git` or a registry `version`;
- a development checkout may override those Git libraries with
  `[patch."<repository-url>"]` in `.cargo/config.toml`.

Patchwork must be able to inspect the directory of every selected normal, API,
or support mod. A local `path` is therefore part of the composition contract.
Plain libraries are different: they are Cargo implementation dependencies, not
registry projects. A user downloading only the selected mods must still be
able to obtain those libraries through Cargo.

Example inside a mod:

```toml
[dependencies]
some-selected-mod = { path = "../some-selected-mod" }
codegen-utils = { git = "https://github.com/example/project.git" }
```

Example local development override:

```toml
[patch."https://github.com/example/project.git"]
codegen-utils = { path = "codegen-utils" }
```

Cargo discovers `.cargo/config.toml` from the process working directory and
its parents, not from `--manifest-path`. Run local commands from the directory
whose `.cargo/` folder contains the patch, pass that config explicitly, or put
an equivalent patch in an ancestor of the composed project.

In the desktop launcher, pass it explicitly from the profile's **Options** tab:
add `--config` and the absolute config path as two compilation argument rows.
This is an opt-in profile setting; the composer does not search the mod cache
for Cargo configuration or import it automatically.

For composed projects, Patchwork generates equivalent root Git-source patches
for selected Patchwork mods automatically. This prevents a cached path crate
and the same package reached transitively through Git from becoming two
incompatible Cargo crate instances.

## Mod assets

A mod can contain an `assets/` folder. When the mod is selected, Patchwork copies
those files recursively into the composed project under:

```text
assets/<mod-name>/
```

Example:

```text
mods/block-dirt/assets/dirt.png
```

becomes:

```text
assets/block-dirt/dirt.png
```

This convention avoids collisions between different mods. A Bevy mod, for
example, can load the asset with the path `block-dirt/dirt.png` relative to the
composed project's `assets/` root.

## Mod favicon

For the launcher and browser, a mod can have a decorative `favicon.png`,
`favicon.jpg`, `favicon.jpeg`, `favicon.webp`, or `favicon.gif` file in its own
folder. The favicon is not part of the dependency graph and does not change the
build. It only represents the mod visually.
