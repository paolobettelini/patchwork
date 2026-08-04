# Avoiding dependency cycles

Codegen is the most delicate part of the paradigm. If a mod declares types that
must be included in a generated crate, that same mod must not depend on the
generated crate in a circular way.

## Bad cycle

This is a problematic design:

```text
network-vanilla-mod
  defines ServerMessage1
  declares ServerMessage1 in metadata
  depends on network-messages

network-messages
  generates enum with network_vanilla_mod::ServerMessage1
  depends on network-vanilla-mod
```

The cycle is:

```text
network-vanilla-mod -> network-messages -> network-vanilla-mod
```

Cargo cannot compile it.

## Correct separation

The solution is to introduce a leaf crate for payloads:

```text
network-vanilla-types
  defines ServerMessage1
  does not depend on network-messages

network-vanilla-mod
  depends on network-vanilla-types
  declares network_vanilla_types::ServerMessage1 in metadata
  can depend on network-messages if it needs the final enums

network-messages
  depends on network-vanilla-types
  generates enum with network_vanilla_types::ServerMessage1
```

The graph becomes:

```text
network-vanilla-types <- network-messages
network-vanilla-types <- network-vanilla-mod
network-messages     <- network-vanilla-mod
```

There is no cycle.

## General rule

When you generate an aggregated crate:

1. aggregated types should live in leaf crates;
2. the generated crate depends on the leaf crates;
3. operational mods depend on the generated crate;
4. the leaf crate never depends on the generated crate;
5. the generator can validate this and fail with a clear error.

For networking, the generator rejects payloads declared in the same mod crate
that declares them in metadata. The error suggests moving payloads into a leaf
types crate.

## Pattern for other domains

Items:

```text
my-items-types       -> struct SwordDefinition
items-generated      -> enum ItemDefinition
my-items-mod         -> uses items-generated
```

Spells:

```text
fire-spells-types    -> struct Fireball
spells-generated     -> enum SpellCommand
fire-spells-mod      -> registers/uses SpellCommand
```

UI:

```text
hud-types            -> struct HudPanelSpec
ui-generated         -> enum UiPanel
hud-mod              -> dispatch/render
```

The shape changes, but the healthy graph stays the same.
