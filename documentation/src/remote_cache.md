# Remote compilation cache

The desktop launcher's **Settings / Remote cache** optionally enables embedded
sccache 0.18.0 for Cargo compilation, including compilation performed by codegen.
It is disabled by default. No separately installed sccache is required, and the
game executable does not inherit the cache configuration.

## Configuration

Configure an S3-compatible endpoint URL and bucket, then save settings.
Region is available only under Advanced and defaults to `auto`.
HTTP versus HTTPS determines TLS. Endpoint URLs must not contain credentials,
query parameters or fragments. Advanced settings include an object key prefix
(default `patchwork/`) and an optional local disk tier.

- **Read only / None** supports public anonymous reads without distributing
  write credentials. Anonymous requests do not use ambient AWS credentials.
- **Read only / Access key / Secret key** supports authenticated reads.
- **Read write / Access key / Secret key** permits populating the remote cache.
  Patchwork rejects anonymous read-write configuration.
- **Use local cache in addition to remote cache** enables the upstream
  `disk,s3` multilevel chain. The local tier remains writable even when remote
  access is read-only. With this option off, only S3 is configured.

The bucket must already exist. Configure permissions on the server: selecting
read-only in the launcher is not a substitute for bucket access control. Anyone
allowed to write cache objects is trusted to supply compiled artifacts; restrict
write credentials to trusted publishers and use HTTPS outside local development.

**Test connection** tests the current form without saving it. Read-only mode
reads `.sccache_check` under the configured prefix; a missing object is accepted,
so this is an endpoint/read-request check, not proof of a successful cache hit or
of bucket contents. Read-write mode additionally writes a random temporary
object, reads it back and deletes it. That test needs delete permission; a cleanup
error reports the object key. A timeout or interrupted process may leave a probe
object behind. The test has a 20-second timeout and does not require bucket listing.

## Execution and storage

Public settings are saved in launcher `settings.json`. Credentials are stored
separately beside `auth.json`, in `remote-cache-credentials.json`, not in profile
TOML or remote registry metadata. This is a plaintext credential file, not an OS
keychain: newly created files use mode `0600` on Unix; Windows uses inherited
directory ACLs. Protect this configuration directory and backups. Saved secrets
are never returned to the frontend; blank credential inputs preserve them.
Selecting authentication None and saving removes the stored credential file.

Cache files live in `sccache/` next to the configured build-cache directory
(normally `patchwork/cache/sccache/`). A named alias `wrapper/.../sccache[.exe]`
is a **hard link to the Patchwork executable**, or a copy when hard links are
unavailable. This is necessary because upstream interprets an executable with a
different basename as a compiler alias. No third-party executable is downloaded.
Updated binaries get a new alias; old aliases may be removed when no build is
running.

The alias enters wrapper mode before initializing Tauri when
`PATCHWORK_SCCACHE_WRAPPER` is present, then calls `sccache::main()`.
Each compose/build operation starts its own sccache daemon on an automatically
selected loopback port and stops it on completion. Changing settings therefore
applies to the next operation without restarting the launcher. The upstream idle
timeout is also configured as a fallback for interrupted operations.

Credentials are supplied to the daemon, not Cargo or its build scripts. If a
credentialed daemon dies mid-build, retry the operation to initialize a fresh
authenticated session. Cache startup/configuration errors fail the operation;
upstream server-I/O fallback is enabled, and multilevel cache-write errors are
configured to be ignored rather than prevent compilation.

The launcher manages these environment variables for the compilation processes:

| Variable | Value |
| --- | --- |
| `RUSTC_WRAPPER` | Alias of the embedded Patchwork executable |
| `PATCHWORK_SCCACHE_WRAPPER` | `1` |
| `RUSTC_WORKSPACE_WRAPPER` | Empty (avoid stacking wrappers) |
| `CARGO_INCREMENTAL` | `0` (required for Rust caching) |
| `SCCACHE_BUCKET`, `SCCACHE_ENDPOINT`, `SCCACHE_REGION` | Form configuration |
| `SCCACHE_S3_USE_SSL` | Deduced from endpoint scheme |
| `SCCACHE_S3_KEY_PREFIX` | Advanced key prefix |
| `SCCACHE_S3_RW_MODE` | `READ_ONLY` or `READ_WRITE` |
| `SCCACHE_S3_NO_CREDENTIALS` | `true` for anonymous access |
| `SCCACHE_MULTILEVEL_CHAIN` | `disk,s3` or `s3` |
| `SCCACHE_LOCAL_RW_MODE` | `READ_WRITE` |
| `SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY` | `ignore` |
| `SCCACHE_BASEDIRS` | Automatically detected existing source/cache roots |
| `SCCACHE_DIR` | Local `sccache/objects` directory |
| `SCCACHE_CONF`, `SCCACHE_CACHED_CONF` | Launcher-owned cache configuration files |
| `SCCACHE_SERVER_PORT` | Per-operation loopback port |
| `SCCACHE_IDLE_TIMEOUT` | `300` seconds |
| `SCCACHE_IGNORE_SERVER_IO_ERROR` | `1` |

Inherited `AWS_*` and `SCCACHE_*` configuration is removed for these managed
commands. Access-key authentication adds `AWS_ACCESS_KEY_ID` and
`AWS_SECRET_ACCESS_KEY` only to daemon startup and the isolated connection probe.
Disabled remote cache leaves the normal Cargo compilation path unchanged.

## Portability and expected speedups

**This integration uses unmodified upstream sccache. It does not guarantee cache
hits between different users' Rust source directories.** Automatic base
directories include existing mod cache, build cache, Cargo target, local registry
folders and Cargo home. They are passed using the platform-specific path-list
separator. However, upstream 0.18 Rust hashing still includes absolute inputs,
notably the compilation working directory; `SCCACHE_BASEDIRS` does not completely
normalize Rust cache keys. No custom Rust hashing fork or forced path remapping
is applied.

A first installation only benefits when the remote already has compatible
artifacts. Toolchain, target, flags, features, dependencies and relevant paths
must match; different OS/architectures cannot generally share compiled objects.
Final executable linking and other non-cacheable work still execute locally.
An empty cache does not make the first compilation faster. For shared-cache
experiments, first use consistent toolchains, layouts and build settings across
the trusted cache producer and consumers.

See upstream [S3 configuration](https://github.com/mozilla/sccache/blob/main/docs/S3.md)
and [multilevel caching](https://github.com/mozilla/sccache/blob/main/docs/MultiLevel.md).
