# Introduction

Patchwork is a source-level modding platform for Rust software. Instead of
loading binary plugins at runtime, its composer takes a set of Rust crates,
orders them, resolves dependencies between mods, generates a final Cargo
project, and builds it like a normal Rust program.

The main idea is to treat mods as composable source code. Each mod is a Cargo
crate, declares its metadata in its own `Cargo.toml`, and can depend on other
mods or on abstract APIs provided by other mods in the modpack.

This moves many problems from runtime to compile time:

- the compiler sees the whole final program;
- dependencies are normal Rust dependencies;
- types are real Rust types, not only strings or dynamic IDs;
- Patchwork can generate glue code, generated crates, and Cargo patches;
- each specific mechanic, like networking, stays in a mod or in a group of mods.

Around that core, the project includes a CLI, a Tauri desktop launcher, a
Leptos web application served by Actix, and a database crate for accounts and
the future public mod catalogue. Shared Leptos components keep Browse, Upload,
profiles, themes, and navigation consistent between desktop and web while
allowing each application to expose different actions.

The generic composer should not know what a network message, an item, a spell,
a quest, or a UI panel is. It only needs to know how to compose mods, read
metadata, resolve the graph, and run generators declared by mods.

## Goal of this book

This book explains:

- how to think about moddable features in this paradigm;
- how to write a mod;
- how to use the `patchwork` command;
- how the desktop launcher stores data and runs builds;
- how the web server, database, accounts, and OAuth flows fit together;
- how Patchwork links a stable Patchwork account to a GitHub identity;
- how to keep the generic framework separate from domain mechanics;
- how to design codegen without creating dependency cycles;
- how to explain the project to a person or to an LLM that needs to change it.

The principle to keep in mind is:

> Patchwork composes code, but it does not know the domain of the mods.

The catalogue and account services know about publishing and identity, but
they do not change this rule: the generated application remains a normal Cargo
project checked by Rust.
