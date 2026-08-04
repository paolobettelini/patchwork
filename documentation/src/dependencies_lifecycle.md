# Dependencies and lifecycle

Each mod declares three dependency lists:

```toml
[package.metadata.mod.dependencies]
init = ["some-mod"]
run = ["some-api"]
ownership = ["owned-mod"]
```

These lists do more than define initialization order. They also tell Patchwork
how to pass objects to the final code.

## `init`

`init` dependencies are passed to `init()` as mutable references:

```rust
let mut some_mod = some_mod::SomeMod::init();
let mut consumer = consumer::Consumer::init(&mut some_mod);
```

Use this for early registration, configuration, wiring, and setup that must
happen before objects are shared.

## `run`

`run` dependencies are shared through `Arc`:

```rust
let some_mod = Arc::new(some_mod);
consumer.run(some_mod.clone());
```

This is a good fit for shared services, thread-safe APIs, managers, registries,
and async tasks.

## `ownership`

`ownership` dependencies are moved by value:

```rust
consumer.run(owned_mod);
```

Use this when a mod must take exclusive ownership of an object. With Bevy, for
example, the main mod can take ownership of the Bevy wrapper and call `.run()` on
the app.

Rules:

- only one mod can take ownership of the same object;
- an owned object is not wrapped in `Arc`;
- no other mod can request it in `run`;
- Patchwork validates these conflicts before generating the project.

## Topological order

Patchwork builds a graph using `init`, `run`, and `ownership`. Dependencies are
selected transitively and initialized before consumers. A dependency does not
need to be repeated in the root modpack's `mods` list. If Patchwork finds a
cycle, composition fails.

The mental rule is:

> if a mod names another mod or API in its dependencies, that thing must be
> available first.

API mods declared with `api = true` participate in selection but do not create
lifecycle objects. If a dependency names a selected API mod, Patchwork resolves
the runtime object to the exactly one selected normal mod that declares
`provides = "<api-id>"`. Selecting a provider also selects its provided API as
a source dependency, ensuring the API crate is present in local and downloaded
compositions.

Support mods declared with `support = true` also participate in selection, but
represent content with no runtime object. Select them through the modpack; do
not place them in `init`, `run`, or `ownership`.
