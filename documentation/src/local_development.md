# Local development with installed modpacks

A development profile can combine an installed modpack, downloaded mods from
multiple repositories, and explicitly linked local mods or libraries. Local
development is **per profile**, not a global replacement for the download cache.
The checkout remains the source of truth; Patchwork never downloads into it or
rewrites its Cargo manifests.

## Desktop workflow

1. Download a modpack as a profile, or use an existing profile.
2. Open the profile's **Local development** tab and enable it.
3. Use **Link checkout** to add one or more checkout directories. The path is
   saved, not a frozen list of projects. The suggested Git URL comes from
   `remote.origin.url`: change it if Cargo uses a different URL (e.g. HTTPS rather
   than SSH). An empty URL enables mod/modpack discovery without library patches.
4. Add optional manual mod/modpack overrides for individual IDs. Manual mappings
   have priority over discovered sources, which have priority over installed
   cache sources. Multiple checkouts are supported; conflicting IDs require an
   explicit override or exclusion, not an arbitrary first-match winner.
5. Libraries may live anywhere: `mods/`, `libs/`, or a separate checkout. A
   checkout with a Git URL discovers its libraries recursively. Additional
   library directories can also be linked using **Libraries** and the Git URL
   appearing in Cargo dependencies.
6. Save once, then Compose & Build. Download resolution and source preparation
   rediscover configured checkouts, including newly added mods/modpacks. Add a
   new mod to the local modpack, then compile: no Refresh or relinking is needed.
   Discovery makes projects available; it does not select every mod in a checkout.
   There is no background watcher or hot reload.

The root profile remains authoritative. Linking a checkout does not replace a
copied root profile with a same-named local modpack. To follow local root edits,
keep a symlink from the profile TOML to the development TOML, or use a separate
profile importing the local modpack (for example `client-dev` importing `client`).
The former is supported: changes to that profile from the launcher also modify
the linked source TOML. The latter keeps launcher-specific edits separate.

**Automatic source exclusions** contains one entry per line: `some-mod`,
`modpack/some-pack`, or `library/some-package`. Mod/modpack exclusions suppress
automatic local discovery and allow installed/downloaded sources to be used;
an explicit mod/modpack override still wins. Library exclusions suppress local
Git patches for that package, including additional library directories. They
do not disable a mod in the composition: use the Dependencies toggle for that.

Library discovery includes crates with a library target, not binary-only
composed projects such as `build-server/server`. Two distinct library crates
with the same package name and Git source remain an error; the diagnostic lists
both paths.

Adding a new mod override also adds that mod to the root profile selection.
Overriding an existing mod ID replaces its source, not its identity. The override
wins even if its local version is older than the installed version. Invalid or
missing linked sources fail explicitly rather than silently using downloaded
code. Removing an override keeps the profile selection: disable/remove the
selection too if that mod has no published version to fall back to.

Disable Local development to return to installed sources. Existing override
rows are retained. A second profile can keep using the published modpack while
the development profile uses local changes.

## Configuration and CLI

The launcher stores local connections separately:

```text
profiles/.patchwork/<profile-id>.development.toml
```

They are never loaded from metadata inside downloaded mods or modpacks. The CLI
uses the same format, activated explicitly with `--development`:

```toml
enabled = true
exclude = ["remote-only-mod", "modpack/published-content", "library/remote-helper"]

[[checkouts]]
path = "/work/game"
git = "https://github.com/example/game.git"

[[checkouts]]
path = "/work/extra-content"
# No git: discover mods/modpacks only.

[mods]
my-renderer = "/work/renderer/my-renderer"

[modpacks]
my-content = "/work/content/modpacks/my-content.toml"

[[libraries]]
git = "https://github.com/example/rendering.git"
path = "/work/rendering"

[[libraries]]
git = "https://github.com/another-author/physics.git"
path = "/work/physics"
```

Relative paths are resolved against the directory containing the development
configuration, not the current working directory. For desktop configuration the
base is therefore `profiles/.patchwork/`; folder/file pickers supply absolute paths.
Library overrides are associated with a Git source and Cargo package name.
Ambiguous duplicate packages for the same source are rejected.

```bash
patchwork compose \
  --modpack /path/to/profiles/client.toml \
  --mods-folder /path/to/patchwork/cache/mods \
  --modpacks-folder /path/to/patchwork/cache/modpacks \
  --cache /path/to/patchwork/cache/build \
  --name client \
  --development /path/to/development.toml

cd /path/to/patchwork/cache/build/client
CARGO_TARGET_DIR=/path/to/cargo-target cargo build
```

The CLI composes available inputs; it does not turn into the launcher's registry
downloader. Required remote mods/modpacks must already be installed. Explicit
mod mappings in CLI configuration replace sources; to select a new mod, also
add its ID to the root modpack's `mods` array. Explicit
development configuration is trusted local input, just like choosing a crate to
build. Do not enable configuration received from an untrusted party without
reviewing it. Cargo build scripts and codegen execute code.

## Prepared source tree

The core creates a profile-specific managed source tree from local overrides and
installed crates. This is necessary because a Rust dependency such as
`path = "../other-mod"` cannot follow a Patchwork ID override by itself.

Copies in that managed tree have their path dependencies adapted to the resolved
sources; originals are untouched. Local content is synchronized again before
composition and obsolete managed files are removed. Unchanged content is not
rewritten unnecessarily. This uses normal files, not symlinks, so Windows does
not need symlink privileges. Assets, contributor metadata and codegen read the
same prepared sources as Cargo.

The desktop tree is under `buildCache/.development/<profile-id>/`. It can be
discarded with the build cache. Never edit it as a development checkout.
Generated Cargo configuration is propagated to the composed project and to
codegen, so no manually generated `mods/.cargo/config.toml` is required.

Ordinary libraries remain ordinary Rust crates, not fake support mods. Keep
their distributable Git/version dependency declarations in source control.
Cargo version constraints still apply to local patches; linking a checkout does
not make an incompatible library version compatible. Different Git repositories
can provide different library packages; Patchwork mod IDs themselves remain
global identities, not repository-scoped names.

Cargo may still fetch Git source metadata during initial dependency resolution,
even when a local patch supplies the compiled package. This is not a guarantee
of an offline first build.

Generated mods remain codegen outputs, not downloadable registry entries. A
development override must not be used to publish or download generated IDs.
Codegen outputs used by the development build belong to the managed build tree.

Configured roots and profile TOMLs may be symlinks; their real paths are resolved
before overlap checks. Recursive discovery does not follow internal symlinks.
Source-tree preparation still rejects symlinks/special files inside copied
crate trees and the managed output, unsafe overlaps and ambiguous mappings.
Cargo workspace inheritance is materialized where supported;
unsupported or incomplete declarations produce errors instead of silently
changing their meaning. Arbitrary build scripts reading files outside their
crate are not automatically made portable by this feature: such inputs should
be explicit and available to the build.

## Nested dependencies and exclusions

The profile Dependencies view searches the full nested graph by ID, name and
required-by information. It shows mod/modpack kinds, missing entries and local
overrides, and filters All / Enabled / Disabled. Opening a nested entry preserves
the profile's development context.

Rows show the icon, name, ID and version. Expand a row to inspect its dependencies
and required-by list. Toggling a dependency updates the graph in place, preserving
scroll and expanded rows. A checked switch means the root profile does not
explicitly exclude that entry; a grey row can still be inactive because a parent
is disabled or an imported modpack excludes it. Its expanded status explains why.

An exclusion is stored in the root profile's `ignore` array:

```toml
ignore = ["old-renderer", "modpack/optional-content"]
```

Excluding a mod applies to all occurrences in that profile. Excluding a modpack
suppresses that import, but does not remove a mod still selected elsewhere.
Inactive entries remain inspectable so explicit exclusions can be reversed.
Exclusions inherited from active child modpacks or disabled ancestors are not
equivalent to an explicit root exclusion: the UI identifies those states.

A toggle does **not** remove hard Rust/lifecycle dependency edges. If another
selected mod requires the disabled mod, composition must fail. To replace an API
provider, disable the old implementation and select the new one; the exactly-one
provider rule still applies. API and support mods are not exempt from dependency
validation. Generated entries are not normal navigable/downloadable projects.

## Migrating from the old workaround

- Restore Installation's mod/modpack cache paths to dedicated cache directories,
  not your development checkout. Do not delete or move your source checkout.
- Add persistent **Checkouts** to the intended profile. Existing manual overrides
  remain valid and take priority. Remove redundant rows to rely on automatic
  discovery (especially before renaming/removing a previously linked crate).
  Each group has a remove-all button that only edits the draft configuration,
  never source files. Save to apply, or reload to discard the draft.
- Remove the old profile argument pointing to the generated
  `mods/.cargo/config.toml` once the equivalent library mappings are configured.
- The old generated patch file is no longer necessary for this launcher/CLI
  flow. Keep it only if you intentionally use it for standalone Cargo commands.
- `sync-modularis-deps.sh` may still be useful for normalizing distributable
  manifest declarations, but generating its patch file is not a build prerequisite.
- Local registry folders remain installation sources for Browse. They are not
  development overrides and do not implicitly activate checkout mappings in
  other profiles.
