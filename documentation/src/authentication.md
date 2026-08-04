# Accounts and authentication

Patchwork accounts use a generated UUID as their stable identity. Email and
nickname are unique login/display fields, but the nickname may change without
changing ownership of published projects, sessions, or linked services.

Registration requires email, nickname, password, and password confirmation in
the UI. The frontend enforces the displayed password requirements before
submitting. Server-side validation remains authoritative for all fields.

## Email verification

Registration is a two-step operation:

1. `POST /api/auth/register` validates unique email/nickname and the client
   password digest, hashes that digest with Argon2, and creates a pending
   registration rather than an account.
2. The backend generates a cryptographically random verification identifier
   and a uniformly random six-digit code, then sends the code through Resend.
3. The browser copies the code into `POST /api/auth/register/verify`.
4. A successful verification atomically consumes the pending record, creates
   the stable account UUID, and starts the browser session.

The pending row stores only SHA-256 hashes of the verification identifier and
of `verification-id:code`; the plaintext code is never stored. Codes expire
after ten minutes, allow at most five failed attempts, and are single-use. A
new message for the same email or nickname is rate-limited to one per minute.
If delivery fails, the pending row is deleted so the user can retry.

The identifier returned to the browser has 256 bits of entropy. Including it
when hashing the six-digit code prevents an offline database-only attacker from
testing the one million possible codes without also knowing the secret
identifier. Online attempts remain bounded by the database counter.

## Password storage

The browser hashes the password with SHA-256 and sends the 64-character hex
digest as `passwordSha256`. The server validates that representation and feeds
it into Argon2 with a random salt before storing it.

```text
password -> browser SHA-256 -> TLS -> server Argon2 + random salt -> database
```

Client-side SHA-256 does not replace HTTPS and is not sufficient password
storage by itself. Its digest acts as the password-equivalent at the HTTP
boundary, so it must be protected by TLS. The database stores only the Argon2
encoded hash.

## Browser sessions

Successful verification or login creates a random session token. The browser
receives it in the HttpOnly `patchwork_session` cookie with `SameSite=Lax`; the
database stores only its SHA-256 hash. Sessions expire after 14 days.

JavaScript cannot read the HttpOnly cookie. Requests use normal same-origin
cookie handling, and logout deletes the server record and expires the cookie.
In production the cookie must also be Secure through `--secure-cookies true`.

## Desktop OAuth with PKCE

The desktop app is a public OAuth client and cannot safely contain a client
secret. It therefore uses Authorization Code Flow with PKCE:

1. The app creates a high-entropy verifier, its S256 challenge, and a random
   state value.
2. It binds a random loopback port and opens `/oauth/authorize` in the system
   browser with client ID `patchwork-app`.
3. The user signs in or creates and email-verifies an account on the styled
   server page, then explicitly authorizes the launcher.
4. The server creates a single-use authorization code valid for ten minutes,
   stores only its hash, and redirects to the exact loopback URI.
5. The app verifies state and exchanges the code plus original verifier at
   `/oauth/token`.
6. The server verifies the client, redirect URI, one-time code, and PKCE
   challenge, then returns a bearer token and profile.

Desktop access tokens last 90 days. Only their SHA-256 hashes are stored in the
database. The plaintext token and cached profile live in the Patchwork-owned
`config/auth.json`, not in Tauri storage or browser `localStorage`.

Protected backend routes accept either the browser session cookie or
`Authorization: Bearer <desktop-token>`. This lets the web and desktop clients
share profile and GitHub endpoints while keeping their persistence mechanisms
different.

## Future game authentication

The stable account UUID is suitable for passing an optional player identity to
a composed game. A future game login protocol should issue a short-lived,
audience-bound game ticket instead of exposing the long-lived desktop bearer
token. Game clients and servers can then validate that ticket with Patchwork's
backend. That protocol is not implemented yet.
