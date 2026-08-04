# The paradigm

In traditional modding, a mod is often a plugin loaded at runtime. The main
program exposes dynamic registries, string IDs, callbacks, reflection, or a
script engine. This works, but many guarantees are moved outside the compiler.

In source-level modding, mods are normal Rust crates. Patchwork builds a new
final crate that depends on the selected mods and generates the code needed to
initialize and run them.

## Separation of roles

The system has three layers.

The generic framework:

- reads Cargo metadata;
- reads modpacks;
- resolves dependencies;
- generates the final project;
- runs codegen tasks declared by mods.

Domain mods:

- implement concrete features;
- declare dependencies and metadata;
- can declare codegen tasks;
- can read or produce generated types.

Generated crates:

- contain code derived from the selected modpack;
- exist in the final build;
- can also have a development copy inside `mods/` to help the language server;
- are normal dependencies for other mods.

## The most important boundary

The framework should not contain concepts like:

- `ClientBoundMessage`;
- `ServerBoundMessage`;
- the network serializer;
- the item registry;
- spell enums;
- domain-specific ECS dispatch.

These things belong to mods. The framework only provides a generic way to say:

```toml
[[package.metadata.mod.codegen]]
crate = "some-generated-crate"
version = "0.1.0"
dev_crate = "some-generated-crate"

[package.metadata.mod.codegen.generator]
crate = "some-codegen-mod"
command = "generate"
```

The meaning of the generated code is domain-specific. Patchwork only knows
which generator to run, where to write the generated crate, and how to patch the
composed project so it can use that crate.

## Types, not only string IDs

When possible, a moddable feature should expose real Rust types instead of only
string IDs. For example, a network mod should not force other mods to send
`"vanilla:login"` or `"my_mod:spawn"` as strings. It should generate a final
enum:

```rust
pub enum ClientBoundMessage {
    SpawnPlayer(my_types::SpawnPlayer),
    DespawnEntity(other_types::DespawnEntity),
}
```

This enum can be built, serialized, matched, and dispatched with normal Rust
code.

The point is not to remove every dynamic registry. The point is to move into the
type system everything that can be known after the modpack is composed.
