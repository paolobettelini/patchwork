# GitHub integration

GitHub linking associates an already authenticated Patchwork UUID with a
GitHub user. The authoritative external identifier is GitHub's numeric user ID;
login and avatar URL are cached display metadata because a GitHub login can
change.

The database enforces a unique constraint on `github_user_id`, so the same
GitHub identity cannot be linked to two Patchwork accounts.

## Web flow

1. An authenticated user selects **Connect GitHub** on the profile page.
2. `GET /github/connect` creates a random state associated with the Patchwork
   UUID and the configured web profile completion URL.
3. The backend redirects the browser to GitHub's authorization page.
4. GitHub redirects to `/github/callback?code=...&state=...`.
5. The backend consumes the state, exchanges the code using the server-only
   client ID and client secret, calls GitHub's authenticated-user API, and
   stores numeric ID, login, and avatar URL.
6. The backend returns the browser to `<frontend-url>/profile`.

## Desktop flow

GitHub authorization still happens in the browser, but the result must update
the running Tauri application immediately:

```text
desktop app
  -> bind http://127.0.0.1:<random-port>/github-connected
  -> POST /github/connect with completionUrl and bearer token
backend
  -> return authorizationUrl
desktop app
  -> open authorizationUrl in the system browser
GitHub
  -> redirect to backend /github/callback
backend
  -> verify state and link account
  -> HTTP 302 to the stored loopback completion URL
Tauri listener
  -> accept the callback and close
  -> GET /github/account
  -> refresh /api/profile and emit a UI auth event
```

The POST endpoint accepts only `http` completion URLs with host exactly
`127.0.0.1`, an explicit port, path exactly `/github-connected`, and no query,
fragment, username, or password. This prevents the backend from becoming an
open redirect. The random state is 256 bits, stored only as a SHA-256 hash,
valid for ten minutes, tied to both the Patchwork UUID and completion URL, and
consumed atomically once.

The app's loopback server is a one-shot notification channel, not an
authentication authority. The callback contains only the result; after it
arrives the desktop app fetches `/github/account` using its existing Patchwork
bearer token. The UI is then updated through the existing Tauri authentication
event without restarting the app. While authorization is pending, the frontend
also polls the native authentication state as a fallback, so a delayed or lost
window event cannot leave the button stuck on `Waiting for GitHub`.

## Disconnect

`DELETE /github/account` removes the local association for the authenticated
Patchwork account. It does not revoke GitHub authorization and does not
uninstall the GitHub App. The profile immediately refreshes in both clients.

## User authorization and installation

These are deliberately separate concepts:

- user authorization proves which GitHub user is linked to a Patchwork UUID;
- GitHub App installation grants Patchwork access to repositories owned by a
  user or organization.

The user access token returned by the callback is currently used only to call
GitHub's `/user` endpoint and is discarded afterwards. It is not stored in the
database.

The backend authenticates as the GitHub App. It signs an RS256 JWT
from `app_id` and the PEM private key, using an issue time 60 seconds in the
past and an expiry nine minutes in the future. It can exchange that JWT for an
installation access token scoped to a list of numeric repository IDs. No
installation token is persisted or exposed to either frontend.

The registry publication flow now verifies all of the following:

- the Patchwork account has a linked numeric GitHub user ID;
- the GitHub App installation can access the numeric repository ID;
- GitHub reports at least `write` permission for the linked user (`maintain`
  maps to write-level access; `admin` is also accepted);
- repository and user identity comparisons use numeric GitHub IDs, not only
  mutable `owner/repo` and login strings.

For a scan the backend first creates an installation token covering repositories
available to that installation, resolves the requested repository's numeric ID,
checks the linked user's collaborator permission, and then creates a second
installation token scoped to that one numeric repository ID. That scoped token
reads commits, trees, and selected Cargo blobs. It is dropped when the request
finishes.

Repository user authorization and installation remain separate even if the
GitHub App UI can combine them. Patchwork deliberately does not combine them:
connecting a GitHub identity does not prove that the App is installed for every
repository. Conversely, an organization installation does not prove that the
linked user may publish from every repository it covers.

If a scan cannot find an installation for the requested repository, both
clients show a dedicated tip asking the user to install the Patchwork GitHub
App there and retry. Installation grants the App read access to repository
metadata and Git objects. It does not replace the separate collaborator check:
the scan still verifies that the linked user has `write`, `maintain`, or
`admin` permission.

See [Registry publication](./registry_publication.md) for the no-clone tree
scan and immutable commit model.

## GitHub App configuration

Configure the development GitHub App with expiring user tokens enabled, Device
Flow and webhooks disabled, Contents and Metadata read-only repository
permissions, and installation allowed on any account. Use these two distinct
URLs:

```text
Callback URL: http://localhost:8080/github/callback
Setup URL:    http://localhost:8080/github/installation-complete
```

**Request user authorization during installation must be OFF.** Patchwork starts
user authorization explicitly through `/github/connect`; enabling the GitHub
option sends installation OAuth codes to the callback without Patchwork's
one-time state and mixes two unrelated flows.

`GET /github/installation-complete` returns a styled informational page. It
does not mutate the database and does not trust the optional
`installation_id` query parameter. The next repository scan is the authoritative
installation/access check. For development configurations that still redirect
an installation OAuth code to `/github/callback`, that callback recognizes the
missing-state installation shape and shows the same page without exchanging or
storing the code. This is only a compatibility convenience; the GitHub App
setting should still use the dedicated Setup URL.

`client_secret`, the PEM private key, app JWTs, user tokens, and installation
tokens are backend-only secrets. None may be returned to or embedded in the
Leptos frontend.

The current registry calls require only the configured GitHub App repository
permissions `Contents: read-only` and `Metadata: read-only`: Contents covers
commit/tree/blob reads, while Metadata covers repository and collaborator
permission metadata. Publishing does not write to the GitHub repository.

GitHub documents the post-install redirect separately as the
[Setup URL](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
and explains the optional combined install/OAuth behavior in
[Generating a user access token for a GitHub App](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app).
