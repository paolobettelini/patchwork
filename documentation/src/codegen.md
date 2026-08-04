# Generic codegen

Codegen extends the paradigm: some information exists only after a modpack has
been selected. For example, the final enum of network messages must contain the
messages declared by all selected mods.

Patchwork should not know how to build that enum. It only needs a generic way to
declare and run a generator.

## Declaring a generated crate

A mod declares a codegen task in its metadata:

```toml
[[package.metadata.mod.codegen]]
crate = "network-messages"
version = "0.1.0"
dev_crate = "network-messages"

[package.metadata.mod.codegen.generator]
crate = "network-codegen-utils"
command = "generate"
```

Fields:

- `crate`: name of the generated crate;
- `version`: version written in the generated crate;
- `dev_crate`: name of a crate inside `mods/` that the generator updates for
  language server support;
- `generator.crate`: crate that contains the generator binary;
- `generator.command`: subcommand passed to the generator.

Patchwork resolves `generator.crate` as:

```text
mods/<generator.crate>/Cargo.toml
```

and runs:

```bash
cargo run --manifest-path mods/network-codegen-utils/Cargo.toml -- generate ...
```

## Generator contract

A generator should:

1. read the `Cargo.toml` of the composed project;
2. discover selected mods through path dependencies;
3. read domain metadata from those mods;
4. preserve the Cargo source of ordinary library dependencies used by emitted
   types (`git`, registry `version`, or a rebased `path`);
5. generate a standalone crate;
6. optionally update the dev crate in `mods/`;
7. avoid changing the generic framework.

For networking, the generator reads:

```toml
[package.metadata.network.messages]
clientbound = ["some_types::MessageA"]
serverbound = ["some_types::MessageB"]
```

This table is not known by Patchwork. It is known only by the network generator.

Contributor discovery and generated dependencies have different rules. The
composed project contains selected Patchwork mods as paths, so a generator can
open their manifests and inspect their metadata. A contributor may, however,
refer to a type from a plain Cargo library:

```toml
[dependencies]
block-api = { git = "https://github.com/example/project.git" }
```

The generator does not need to find that library in the downloaded mod cache.
It should copy the dependency declaration into the generated crate. Local paths
must be canonicalized relative to the contributor and then rebased relative to
the generated crate; Git/version sources should remain distributable. Requiring
all such dependencies to be paths incorrectly turns ordinary libraries into
Patchwork registry artifacts.

## Dev crate

A generated crate can exist in two places:

```text
build/network-messages/
mods/network-messages/
```

The first one is the real composition output. The second one is a development
copy that helps rust-analyzer and other mods while working.

Mods can depend on:

```toml
network-messages = "0.1.0"
```

and use a local patch during development:

```toml
[patch.crates-io]
network-messages = { path = "../network-messages" }
```

In the composed project, Patchwork inserts a patch to the generated crate inside
the build/cache folder.

## Registry boundary

The substring `generated` is reserved in Patchwork mod IDs for crates produced
by codegen during Compose. A Cargo crate whose Patchwork mod ID contains that
substring is not a registry artifact:

- Upload reports the crate as a non-publishable error entry;
- Browse and profile/project navigation do not expose a project page for it;
- the desktop dependency resolver skips it instead of looking in local
  registries, the backend, or the download cache;
- Compose removes it from modpack selection and lifecycle dependency graphs
  before attempting to read mod manifests from the cache;
- a dependency reference may still be stored on the publishing mod version,
  but clients render it as **Generated during compose**.

This rule applies to Patchwork mod IDs, not to arbitrary Cargo dependency names
or output directory names. The generated crate is supplied by the codegen task
and patched into the composed Cargo project; it is never downloaded as source
from the registry.

## Helper libraries

To avoid rewriting crate-generation boilerplate, a library like `codegen-utils`
can provide generic primitives:

- `GeneratedCrate`;
- `GeneratedDependency`;
- `GeneratedFile`;
- `write_generated_crate`;
- crate-name normalization;
- type-path to variant-name conversion.

This library stays generic. It should not know what networking is.
