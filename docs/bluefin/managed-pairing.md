# Managed companion pairing v1

This optional Bluefin contract is separate from stock QuickConnect. Discover it
with `GET /Bluefin/Pairing/Capabilities`: only an explicit
`{"managed_pairing_v1":true}` opts in. A false result, missing route, timeout or
non-Bluefin server does not qualify durable cleanup. Legacy QuickConnect may
remain available, but clients must retain unresolved challenge/token ownership
and block replacement setup rather than treating an expired/unknown 404 as
revocation proof. This source candidate is not installed-service acceptance.

All remaining routes are POST, with JSON bodies limited to 1024 bytes. Use HTTPS,
refuse redirects, and never put a secret or token in a URL or diagnostic output.
Responses containing credentials must be treated as sensitive. Initiation reads
standard MediaBrowser client/device/version headers. It requires bounded,
nonempty client, device, deviceId and version strings (128 bytes each).

- `/Bluefin/Pairing/Initiate`: no JSON body. Returns
  `{"proof":{"challenge":"random-id","secret":"sensitive"},"device_id":"original-device-id","expires_at":123}`.
  Persist the proof before asking the authenticated phone to authorize it.
- `/Bluefin/Pairing/Authorize`: actual current user authentication is mandatory;
  API keys and caller-selected user IDs are refused. Body:
  `{"proof":{"challenge":"…","secret":"…"},"device_id":"original-device-id"}`.
  The server checks the exact challenge/device binding. The phone must also
  bind its captured server/account/pairing generation and recheck after awaits.
- `/Bluefin/Pairing/Status`, `/Bluefin/Pairing/Exchange`, and
  `/Bluefin/Pairing/Cancel`: body `{"challenge":"…","secret":"…"}`.
  Status and Cancel return `{"state":"…"}`. Exchange returns the standard
  MediaBrowser AuthenticationResult envelope and the same token on retries.

States are pending, minting, authorized, exchanged and cancelled. Authorization
and first exchange have a five-minute deadline. First exchange transfers to a
long-lived session but retains server-side challenge cleanup ownership even if
that exchange reply is lost. Exchanged sessions do not expire with the pairing
deadline; ordinary session/logout rules still apply. Status/Cancel remain usable
when QuickConnect is disabled. Pending/minting/authorized expiry is reaped on
subsequent managed operations. Authorization is one-shot; on ambiguous failures,
check status or cancel rather than minting a replacement.

Only a successful status/cancel response explicitly reporting cancelled is
cleanup proof. 401, 404, expiry, timeout and persistence failure are not proof.
The proof remains sensitive authority to revoke exactly its own session even
after publication, expiry or restart. Never discard it merely because a session
was received. A client must retain it until confirmed cleanup. No arbitrary
access token or user identity is accepted by cancellation.

SQLite persists the secret hash, identity, ownership and terminal receipt.
Triggers atomically fence device insertion against cancellation and record mint
success in the same transaction as the token. Cancellation atomically removes
the owned device token; failure rolls the receipt back. A late insert after
cancellation is refused. Managed auth cache hits and misses check the database
fence, so a cached token cannot survive revocation. Requests already admitted
before cancellation may complete, as with ordinary logout; cancellation does
not undo completed media operations. In-memory session display entries may
remain until ordinary idle eviction, but cannot authenticate with a revoked
token. No automatic replay of a stranded minting operation is performed.

Admission: 16 concurrent managed operations, 64 operations per second per
manager, at most one initiation per source IP every two seconds, at most 64
retained records per IP and 4096 globally. These include terminal receipts;
capacity exhaustion returns 503 with Retry-After and never evicts cleanup
ownership. No proxy header is trusted independently of existing server address
handling. Receipt compaction/administrative reset is deliberately unsupported
in v1. Sustained anonymous load can cause availability refusal; it cannot grant
pairing authority. Unknown JSON fields are refused on credential operations.

Rollback: stop use of this capability before reverting source/binary. Keep the
additive table/triggers and database ownership records; never drop them while
managed sessions may exist. An older binary does not implement the managed auth
cache fence, so invalidate managed sessions before reverting a deployed server.
No deployed rollback or database operation was performed by this candidate.
