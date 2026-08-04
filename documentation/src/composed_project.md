# Composed project

The composed project is a generated Cargo crate. It is not an abstract model: it
is real Rust code that Cargo can compile.

## Final Cargo.toml

Patchwork takes the template and adds path dependencies to the selected mods:

```toml
[dependencies]
bevy-mod = { path = "/absolute/path/to/mods/bevy-mod" }
main-client-mod = { path = "/absolute/path/to/mods/main-client-mod" }
network-mod-client = { path = "/absolute/path/to/mods/network-mod-client" }
```

If generated crates exist, Patchwork also adds a patch:

```toml
[patch.crates-io]
# BEGIN COMPOSER CODEGEN PATCHES
network-messages = { path = "../network-messages" }
# END COMPOSER CODEGEN PATCHES
```

This patch lets mods declare a normal dependency:

```toml
[dependencies]
network-messages = "0.1.0"
```

During composition, that dependency resolves to the crate generated for that
specific modpack.

## Assets

The composed project always contains an `assets/` folder. Patchwork copies the
template assets and then, for each selected mod, copies `mods/<mod>/assets/` into
`assets/<mod>/`.

This lets different mods use the same file names without collisions:

```text
mods/block-dirt/assets/texture.png  -> assets/block-dirt/texture.png
mods/block-stone/assets/texture.png -> assets/block-stone/texture.png
```

## Final main.rs

Patchwork generates glue code that initializes mods in order, wraps shared
objects in `Arc`, and calls `run()`.

Conceptually:

```rust
let mut bevy_mod = bevy_mod::BevyMod::init();
let mut network_mod_client = network_mod_client::NetworkModClient::init(&mut bevy_mod);
let mut main_client_mod = main_client_mod::MainMod::init();

let network_mod_client = Arc::new(network_mod_client);

let mod_handles = main_client_mod.run(bevy_mod, network_mod_client.clone());
```

The real code depends on the modpack metadata.

## Why generate source code

Generating a final crate has some useful properties:

- the generated code can be inspected;
- Cargo and rust-analyzer can reason about normal crates;
- errors are normal Rust errors;
- concrete types can be used instead of forced dynamic dispatch;
- each build can be reproduced from the modpack and metadata.
