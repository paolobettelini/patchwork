# Composer

Patchwork is the generic tool that turns a modpack into a buildable Cargo
project.

Typical command:

```bash
patchwork compose \
  --mods-folder mods/ \
  --modpacks-folder modpacks/ \
  --modpack client \
  --cache build \
  --name client
```

## Main steps

1. Resolve the modpack.
2. Exclude codegen-produced mod IDs containing `generated`.
3. Load mod metadata and transitively select lifecycle dependencies and
   `provides` API targets.
4. Expand any `modpack/id` references.
5. Build the provider map for APIs declared with `provides`.
6. Validate ownership and dependencies.
7. Sort mods topologically.
8. Generate the final crate with the project/modpack name.
9. Run codegen tasks declared by mods.
10. Patch the final `Cargo.toml` to use generated crates.
11. Copy template assets and assets from selected mods.

## What it reads

For each selected mod, Patchwork reads:

```text
mods/<mod-name>/Cargo.toml
```

and looks for:

```toml
[package.metadata.mod]
```

Mods with `support = true` or `api = true` are still selected mods: Patchwork
adds them as path dependencies, copies their assets, and honors their codegen
declarations. It skips lifecycle glue for both because they have no `entry`,
`init()` or `run()`. An API mod additionally requires exactly one selected
normal provider; a support mod does not.

Before reading manifests, the loader removes mod IDs containing `generated`
from both the modpack selection and lifecycle dependency lists. Those crates do
not exist in the mod cache: they are produced later by codegen and patched into
the composed Cargo project.

The old `mod.json` model is no longer part of the flow.

## What it generates

Inside the cache folder it produces:

```text
build/
  client/
    Cargo.toml
    src/main.rs
    src/generated/mod.rs
    assets/
  network-messages/
    Cargo.toml
    src/lib.rs
    src/generated/messages.rs
```

`client` is the final crate in this example. Other crates next to it are codegen
outputs, if the modpack declared any.

The CLI treats `--cache` as an arbitrary output root. The desktop launcher uses
its configured `cache/build` directory as that root, so launcher output does
not mix with cached downloaded mods or modpacks.

## Cargo source unification

Downloaded manifests can combine sibling path dependencies with Git
dependencies from the same repository. Without a root patch, Cargo treats a
cached API and the same API reached through Git as different crates, making
otherwise identical Rust types incompatible.

Patchwork collects Git source URLs from selected manifests and emits one
`[patch."<git-url>"]` table per source in the composed root manifest. Every
selected Patchwork mod/API is mapped to its cache path. Plain helper libraries
that are not Patchwork projects remain Git dependencies. This reproduces the
source unification normally provided by a development checkout's
`.cargo/config.toml` without depending on that local file.

## What it should not know

Patchwork should not contain networking, gameplay, or Bevy logic. If a feature
needs specific codegen, a mod declares a generator. The tool runs it as a process
and passes general context:

- composed project;
- mods folder;
- modpacks folder;
- resolved modpack;
- output crate;
- version;
- optional dev crate.

The generator interprets domain metadata and writes the generated crate.

The CLI, launcher, web catalogue, and database are clients or surrounding
services. They must not leak account, UI, or HTTP concerns into this core
composition pipeline.
