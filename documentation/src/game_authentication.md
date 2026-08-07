# Game authentication and server transfer

Patchwork game authentication lets an optional game client prove its stable
Patchwork account UUID to an arbitrary third-party game server. Servers do not
need to be known or configured by Patchwork in advance. Each running server
registers an ephemeral instance and receives its own backend credential.

Patchwork is the identity and admission authority. The game protocol remains
responsible for TCP/UDP transport, framing, X25519, key derivation, encrypted
packets, gameplay state, addresses, ports, transfer cookies, and disconnects.
The protocol version documented here is `1`.

## Trust boundaries and credentials

| Credential | Holder | Accepted by | Lifetime | Database representation |
| --- | --- | --- | --- | --- |
| Desktop app token | Launcher | `/game/launch-ticket` and normal app APIs | 90 days | SHA-256 hash |
| Launch ticket | Launcher, then local game | `/game/process-sessions` | 60 seconds, one use | SHA-256 hash |
| Process token | Game client memory | `/game/handshakes/authorize` | About 8 hours | SHA-256 hash |
| Server secret | One running game server | Authenticated `/server/*` routes | Renewable 10-minute lease | SHA-256 hash |
| Transfer ticket | Source server, then client | Transfer handshake authorization | 60 seconds | SHA-256 hash |

The launcher never gives its long-lived app token to a composed executable. A
game server never receives a desktop token, launch ticket, or process token. A
client never receives a server secret. Raw backend credentials are not stored
in the database.

HTTPS is mandatory outside local development. TLS protects backend requests;
the independent X25519/AES channel protects the game connection.

## Dynamic server instances

A game server registers every time its process starts:

```http
POST /server/instances
```

No existing identity or credential is required. Patchwork generates a random
UUID and a random 32-byte secret:

```json
{
  "server_id": "7e63a4e8-9c65-4ec6-9bca-25ca5e303065",
  "server_secret": "unpadded-base64url-32-random-bytes",
  "expires_in": 600
}
```

The server keeps both values only in RAM. `server_id` is public and identifies
the current process instance. `server_secret` authenticates that instance and
must never be sent to game clients, committed, or logged. Patchwork stores only
`SHA-256(server_secret)`, `last_seen_at`, and `expires_at`.

The instance renews its lease about every two minutes:

```http
POST /server/instances/<server-id>/heartbeat
Authorization: Bearer <server-secret>
```

```json
{
  "alive": true,
  "server_id": "7e63a4e8-9c65-4ec6-9bca-25ca5e303065",
  "expires_in": 600
}
```

Each successful heartbeat sets `last_seen_at = now` and `expires_at = now + 10
minutes`. A heartbeat received after expiry cannot revive an instance. The
server must register a new instance instead. A clean shutdown should call:

```http
DELETE /server/instances/<server-id>
Authorization: Bearer <server-secret>
```

Success is `204 No Content`. Expired or closed instances cannot create or
redeem handshakes and cannot create transfers. Their active player sessions are
marked `disconnected` by cleanup. That database transition does not close a
network socket; the game still owns its connections.

There is deliberately no server address or port in Patchwork. A source server
communicates the destination address directly to its client using the game's
own protocol.

## Launcher bootstrap

Immediately before starting a cached executable, an authenticated launcher
calls:

```http
POST /game/launch-ticket
Authorization: Bearer <desktop-app-token>
```

```json
{
  "launch_ticket": "unpadded-base64url-32-random-bytes",
  "expires_in": 60
}
```

The launcher creates a one-shot local auth transport before starting the game.
Unix uses an inherited anonymous pipe:

```text
BACKEND_ADDR=https://patchwork.example.com
PATCHWORK_AUTH_FD=3
PATCHWORK_AUTH_PIPE_VERSION=1
```

Windows uses a unique local named pipe instead:

```text
BACKEND_ADDR=https://patchwork.example.com
PATCHWORK_AUTH_PIPE=\\.\pipe\patchwork-auth-<pid>-<random>
PATCHWORK_AUTH_PIPE_VERSION=1
```

`PATCHWORK_AUTH_FD` is only a descriptor number and `PATCHWORK_AUTH_PIPE` is
only a local endpoint name; neither contains the credential. Windows clients
open the named pipe for reading after process start. The launcher rejects remote
named-pipe clients and waits up to ten seconds for the local game to connect.
The Windows named-pipe transport described here is implemented by the desktop
launcher; implementing the corresponding game-side reader is a separate step.
Both transports contain exactly:

```text
u32 big-endian byte length || UTF-8 launch-ticket bytes
```

The write end is closed after this one message. The ticket is not placed in
arguments, environment variables, terminal input/output, or a file. The auth
transport is separate from the console PTY, so stdin remains usable.

When the launcher has no signed-in account it omits the auth endpoint and
ticket, allowing an anonymous game launch. If an account exists but ticket
issuance or local delivery fails, the launcher aborts instead of silently
downgrading identity.

The game consumes the ticket without an app token:

```http
POST /game/process-sessions
Content-Type: application/json

{
  "launch_ticket": "..."
}
```

```json
{
  "process_token": "...",
  "process_session_id": "UUID",
  "expires_in": 28800,
  "uuid": "account UUID",
  "nickname": "current nickname"
}
```

Patchwork atomically consumes the launch ticket and creates the process
session. Replaying it fails. The process token stays only in client memory and
is discarded on exit. UUID is stable and authoritative; nickname is mutable
display data. Process expiry prevents new authorization and transfer actions.

## Binary representation and transcript

All 32-byte values in JSON use RFC 4648 URL-safe Base64 without `=` padding.
This includes public keys, nonces, transcript hashes, server secrets, process
tokens, and tickets. UUIDs use canonical lowercase hyphenated text.

Client and server must reject an all-zero X25519 shared secret. Patchwork also
rejects an all-zero public key at its HTTP boundary.

The canonical version-1 transcript is the exact concatenation:

```text
ASCII "patchwork-game-handshake-v1"
u16 protocol_version, big-endian
16 raw handshake UUID bytes
u16 server_id UTF-8 byte length, big-endian
server_id UTF-8 bytes
32 raw server public-key bytes
32 raw client public-key bytes
32 raw server-nonce bytes
32 raw client-nonce bytes
```

`handshake_hash = SHA-256(canonical_transcript)`, encoded as unpadded
Base64URL. Patchwork recomputes this hash during authorization. The game server
must independently reconstruct and verify the same transcript.

## Direct join

### 1. Server registers a handshake

After its game-level hello, the server generates a random handshake UUID,
32-byte nonce, and fresh X25519 keypair:

```http
POST /server/handshakes
Authorization: Bearer <server-secret>
Content-Type: application/json

{
  "handshake_id": "UUID",
  "protocol_version": 1,
  "server_public_key": "...",
  "server_nonce": "..."
}
```

Patchwork derives server identity only from the secret and creates a `waiting`
handshake valid for 20 seconds:

```json
{
  "registered": true,
  "server_id": "7e63a4e8-9c65-4ec6-9bca-25ca5e303065",
  "expires_in": 20
}
```

The server sends protocol version, handshake UUID, server ID, public key, and
nonce over the game connection. Its private key never leaves the process.

### 2. Client authorizes

The client generates a fresh nonce and X25519 keypair, computes shared secret
`Z`, constructs the transcript, and calls:

```http
POST /game/handshakes/authorize
Authorization: Bearer <process-token>
Content-Type: application/json

{
  "protocol_version": 1,
  "handshake_id": "UUID",
  "server_id": "7e63a4e8-9c65-4ec6-9bca-25ca5e303065",
  "server_public_key": "...",
  "client_public_key": "...",
  "server_nonce": "...",
  "client_nonce": "...",
  "handshake_hash": "...",
  "transfer_ticket": null
}
```

Patchwork checks the registered server values, active server lease, protocol,
transcript hash, process session, expiry, and one-time state. It binds the
account/process/client values, sets kind `direct`, and atomically changes the
handshake from `waiting` to `authorized`.

### 3. Server redeems

The client sends its key, nonce, and hash over the game connection. After
verifying them itself, the server asks Patchwork for admission:

```http
POST /server/handshakes/<handshake-id>/redeem
Authorization: Bearer <server-secret>
Content-Type: application/json

{
  "client_public_key": "...",
  "client_nonce": "...",
  "handshake_hash": "..."
}
```

Patchwork verifies server ownership, lease, authorization, exact client
values, handshake expiry, and process activity. It atomically consumes the
handshake and creates an active player session:

```json
{
  "accepted": true,
  "admission": "direct",
  "player_session_id": "UUID",
  "account": {
    "uuid": "trusted account UUID",
    "nickname": "trusted current nickname"
  },
  "source_server_id": null
}
```

Only this response makes account data trusted server-side. Persistent player
profiles must be keyed by UUID, never nickname. Redeeming twice fails.

## Session keys and encrypted channel

Patchwork never receives or derives game encryption keys. Client and server
use their locally computed shared secret `Z` and transcript hash `H`:

```text
PRK    = HKDF-Extract(salt = H, IKM = Z)
K_C2S  = HKDF-Expand(PRK, ASCII "patchwork-c2s-key-v1", 32)
K_S2C  = HKDF-Expand(PRK, ASCII "patchwork-s2c-key-v1", 32)
IV_C2S = HKDF-Expand(PRK, ASCII "patchwork-c2s-iv-v1", 12)
IV_S2C = HKDF-Expand(PRK, ASCII "patchwork-s2c-iv-v1", 12)
```

Each direction has an independent sequence number starting at zero. Encode it
as an unsigned 96-bit big-endian value and XOR it with that direction's
12-byte base IV to get the AES-GCM nonce. A sequence number must never wrap or
be reused with the same key.

The version-1 AAD is:

```text
u16 protocol_version, big-endian
u8 direction: 0 = client-to-server, 1 = server-to-client
u64 sequence_number, big-endian
u32 ciphertext_length, big-endian, including the 16-byte GCM tag
```

Encrypt with AES-256-GCM. Increment a receive sequence only after successful
authentication and processing. The first encrypted messages can be client
`ClientFinish`, containing the handshake hash, and server `LoginSuccess`,
containing player session ID and trusted account data. Invalid tags, sequence
numbers, or finish hashes terminate the connection.

## Server-to-server transfer

The backend does not need to know server B when server A starts a transfer.

### 1. Server A creates a ticket

```http
POST /server/player-sessions/<player-session-id>/transfers
Authorization: Bearer <server-A-secret>
```

There is no request body. Patchwork verifies that the player is active on A
and that the associated process session is active. It creates a transfer with
no target yet:

```json
{
  "transfer_id": "UUID",
  "transfer_ticket": "unpadded-base64url-32-random-bytes",
  "expires_in": 60
}
```

At this point `player_session.current_server_id` remains A. Patchwork stores
the source, account, process and ticket hash, while `target_server_id` and
`target_handshake_id` remain null.

Server A sends an application packet directly to the client, for example:

```text
Transfer {
    address,
    port,
    cookie,
    transfer_ticket
}
```

Address, port, and cookie are game-owned and are neither stored nor trusted by
Patchwork. Do not put a trusted UUID or nickname in the cookie.

### 2. Client and server B create a new connection

Server B must already have registered its own dynamic instance. It creates a
new handshake through `/server/handshakes` using B's secret. The client creates
a new X25519 keypair and nonce, computes a new `Z` and transcript, then calls
the ordinary authorize route with the transfer ticket:

```json
{
  "protocol_version": 1,
  "handshake_id": "new B handshake UUID",
  "server_id": "B instance UUID",
  "server_public_key": "...",
  "client_public_key": "...",
  "server_nonce": "...",
  "client_nonce": "...",
  "handshake_hash": "...",
  "transfer_ticket": "..."
}
```

Patchwork checks the waiting handshake, active B lease, unexpired `created`
ticket, matching account/process, and that the player is still active on A. In
one transaction it binds the actual destination:

```text
transfer.target_server_id = handshake.server_id
transfer.target_handshake_id = handshake.id
transfer.status = reserved
transfer.reservation_expires_at = now + 20 seconds
handshake.kind = transfer
handshake.transfer_id = transfer.id
handshake.status = authorized
```

The target cannot be A and another B cannot reserve the same ticket.

### 3. Server B redeems

B calls the normal redeem route using B's secret. Patchwork additionally
requires:

- transfer status is `reserved`;
- both the 60-second ticket and 20-second reservation are live;
- target server is authenticated B;
- target handshake is exactly the redeemed handshake;
- the player is still active on source A.

One transaction consumes the handshake and transfer, then changes the existing
player session from A to B. The response uses `admission: "transfer"`, keeps
the same player session ID, and includes A in `source_server_id`.

This is make-before-break. A remains authoritative until B successfully
redeems. If connection, authorization, or reservation fails, expiry leaves the
player on A. Server A can poll:

```http
GET /server/transfers/<transfer-id>
Authorization: Bearer <server-A-secret>
```

Only the recorded source server may read it. The response reports `CREATED`,
`RESERVED`, `CONSUMED`, or `EXPIRED`, plus the target IDs after reservation. A
can close its old connection after observing `CONSUMED`.

```json
{
  "transfer_id": "UUID",
  "status": "RESERVED",
  "target_server_id": "B instance UUID",
  "target_handshake_id": "B handshake UUID"
}
```

Every transfer uses a new X25519 keypair, `Z`, PRK, C2S/S2C keys, IVs, nonces,
and sequence numbers starting at zero. No cryptographic state from A is reused
with B.

## State machines, TTLs, and cleanup

```text
server instance: active --heartbeat--> active -> expired | closed
launch ticket:   fresh -> consumed
handshake:       waiting -> authorized -> consumed
transfer:        created -> reserved -> consumed
                       \-> expired <-/
player session:  active on A -> active on B -> disconnected
```

Recommended and implemented defaults:

| State | Lifetime |
| --- | --- |
| Launch ticket | 60 seconds |
| Handshake | 20 seconds |
| Transfer ticket | 60 seconds total |
| Transfer reservation | 20 seconds |
| Server lease | 10 minutes |
| Server heartbeat interval | About 2 minutes, implemented by the game server |
| Process session | 8 hours by default, configurable |

Status predicates and transactions prevent concurrent duplicate requests from
both succeeding. Expired state is rejected immediately, even before cleanup.

The backend runs cleanup every minute on a blocking worker. It marks expired instances, disconnects
their player sessions, expires stale transfers, and removes old launch tickets,
handshakes, transfers, disconnected player sessions, process sessions, and
unreferenced server instances after the current 24-hour retention period. Cleanly closing an
instance immediately disconnects its players and expires transfers originating
from it.

Errors use JSON:

```json
{
  "error": "machine_readable_code",
  "message": "human-readable explanation"
}
```

Malformed requests use `400`, missing or invalid credentials use `401`, absent
owned state uses `404`, and expired/replayed/stale state generally uses `409`.
Never fall back to a client-supplied UUID after an admission error.

## Client responsibilities

- Read the optional launch ticket once and never log it. On Unix read
  `PATCHWORK_AUTH_FD`; on Windows open `PATCHWORK_AUTH_PIPE` for reading.
- Exchange it for a process token and keep that token only in RAM.
- Use an OS CSPRNG for every nonce and X25519 private key.
- Reject all-zero X25519 shared secrets.
- Reproduce the canonical transcript byte-for-byte before authorization.
- Keep independent C2S/S2C keys and monotonic sequence numbers.
- On transfer, discard all old connection crypto and perform the complete new
  handshake with B.
- Treat address, port, and cookie from A as application routing data, not
  Patchwork identity proof.
- Continue anonymously only when the launcher supplied no auth descriptor.

## Server responsibilities

- Register a fresh instance at process startup; keep ID and secret only in RAM.
- Send heartbeat around every two minutes and stop serving authenticated joins
  if the lease can no longer be renewed.
- Send `DELETE /server/instances/<id>` during a clean shutdown when possible.
- Generate a fresh handshake UUID, nonce, and X25519 keypair per connection.
- Register the handshake before sending it to the client and honor its expiry.
- Recompute the transcript and shared secret locally; reject mismatches and
  all-zero results.
- Redeem before assigning persistent player state. Trust only the account
  object returned by Patchwork and key profiles by UUID.
- Keep destination address/port selection and transfer cookies in the game
  protocol; Patchwork does not provide service discovery.
- Let A keep the player until B redeems; optionally poll transfer status before
  closing A's old connection.
- Never reuse keys, IVs, nonces, or sequence numbers across joins/transfers.

Patchwork currently implements the backend authority and launcher bootstrap.
Participating games implement the optional transport packets and encrypted
channel according to this contract.
