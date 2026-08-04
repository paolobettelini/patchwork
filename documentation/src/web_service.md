# Web service

`patchwork-web` combines a client-side Leptos application with an Actix server.
The frontend provides Home, Browse, Upload, Profile, authentication dialogs,
and shared themes. Actix serves the built assets, handles the SPA fallback, and
owns all account, OAuth, GitHub, registry scan/publication, and database
operations.

## Configuration

The server requires a TOML file passed through `--config`:

```toml
[server]
address = "0.0.0.0"
port = 8080
db-connection = "./patchwork.sqlite"
frontend-url = "http://localhost:8080"

[email]
RESEND_API_KEY = "replace-with-resend-api-key"

[github]
app_id = 123456
client_id = "replace-with-github-client-id"
client_secret = "replace-with-github-client-secret"
private_key_path = "./github-app.pem"
callback_url = "http://localhost:8080/github/callback"
```

- `server.address` and `server.port` select the bind socket. They default to
  `0.0.0.0:8080` if omitted.
- `server.db-connection` is the only source of the database connection string. There
  is no database CLI flag or environment fallback.
- `server.frontend-url` is the public browser URL used for redirects back to the web
  profile. It can differ from the bind address.
- `email.RESEND_API_KEY` authenticates backend-only requests to Resend. Account
  creation cannot deliver verification codes without a valid key.
- `github.callback_url` must exactly match the GitHub App callback.
- a relative `github.private_key_path` is resolved from the directory that
  contains the configuration file.

`--address` and `--port` override the TOML values. `--site-root`, or
`PATCHWORK_WEB_SITE_ROOT`, selects the Leptos asset directory and defaults to
`dist`. `--secure-cookies`, or `PATCHWORK_SECURE_COOKIES`, marks browser session
cookies Secure and should be enabled behind production HTTPS.

The real configuration, Resend API key, GitHub client secret, and private key
must not be committed. They belong only on the backend host and are never
serialized into the Leptos frontend or its WebAssembly bundle. The current
development sender is `Patchwork <onboarding@resend.dev>`; production must use
a Resend-verified sending domain.

In the GitHub App settings, configure `github.callback_url` as the Callback URL,
set the Setup URL to
`http://localhost:8080/github/installation-complete` (using the production
origin in production), and turn **Request user authorization during
installation** off. User linking already has its own state-protected flow.

## Build and run

```bash
cd patchwork-web
cp patchwork.example.toml patchwork.toml
cargo leptos build
cargo run --features server -- --config patchwork.toml
```

`cargo leptos build` creates the browser JavaScript and a valid WebAssembly
module in `dist/pkg`. Serving a Cargo-produced native `.wasm` file in its place
causes a browser magic-number or WebAssembly validation error.

Actix serves `/styles.css`, `/logo.png`, and `/pkg/*` directly. Other non-API
GET paths fall back to the Leptos `index.html`, allowing routes such as
`/browse`, `/upload`, `/profile`, `/mods/<id>`, and `/modpacks/<id>` to be
handled client-side.

## Themes

The site and desktop launcher expose the same theme set. The browser stores its
selection in `localStorage`; it is intentionally independent from the desktop
`settings.json` because the two applications may run on different devices.

## Browse

Browse performs a public `/registry/search` request with text and typed
mod/modpack filters. It renders the same result component as the desktop
launcher, including latest version, downloads, source, and identifying image,
but deliberately exposes no profile/download actions. Images are retrieved by
the backend from the exact Git blob recorded for the published version. See
[Registry browsing](./registry_browsing.md).

Browse cards, published projects on Profile, Upload preview entries, and
available dependencies are navigable. Published pages expose **Details** and
**Dependencies**. Details shows the real version, downloads, publisher,
publication time, repository path, commit, source tree OID, and manifest hash.
An unpublished Upload entry uses its owned persisted scan as the temporary
source of truth and shows unavailable publication/download values as `-`; it
does not masquerade as a published registry version.

## Upload and profile

Upload is a shared `patchwork-ui` component backed by browser HTTP callbacks.
It is available only when the browser session is authenticated and the profile
has a linked GitHub account. The page accepts a repository URL and optional
subdirectory or loose modpack TOML path, then starts a backend scan job. It polls phase/count updates and
renders validated entries incrementally before switching to the persisted
backend preview.

Publishable entries start selected. Already-published, conflicting, and invalid
entries are disabled. The publish action sends only selected entry UUIDs. On
success the site reloads both the scan and `/api/auth/me`, so the profile list
and latest versions update without trusting client-side metadata.

Each published mod and modpack exposes **Rescan**. The site starts the same backend job,
redirects to `/upload?job=<UUID>` for live progress, and ends on the private
persisted preview. A completed scan can still be reloaded through
`/upload?scan=<UUID>` for its 20-minute lifetime.

## Database startup

`Database::connect` creates a pool and automatically applies embedded
migrations. For local SQLite development the configured path is created when
the server first starts. See [Database](./database.md) for the schema and MySQL
build mode. The current registry schema changes the single development
baseline, so an older local SQLite file must be deleted before first use.
