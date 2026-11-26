# Mesh connection protocol, version 1

Status: implementation under qualification. A successful codec decode is not an account login, an unused-invitation assertion, or authorization to contact a relay.

## Capsule

A binary capsule uses the following fields in order. Integers are unsigned and big endian. There is no padding, trailing data, compression or optional extension area.

| Field | Encoding |
| --- | --- |
| Magic/version/reserved | Four bytes: 4d 43 01 00 |
| authority_id | 32 bytes |
| authority_generation | u64 |
| authority_epoch | 32 bytes |
| endpoint_id | 32 bytes (iroh endpoint public key) |
| server_certificate_sha256 | 32 bytes (SHA-256 of exact DER leaf certificate) |
| invitation_id | 32 bytes |
| secret | 32 random bytes |
| expires_at_unix_ms | u64 |
| service | u8 byte length, then 1–64 ASCII bytes from A–Z, a–z, 0–9, slash, dot, underscore, hyphen |
| intent | u8: 0 login, 1 signup; all other values rejected |
| account_hint | u8: 0 absent, 1 followed by 32 opaque bytes; all other values rejected |
| relay_urls | u8 count (0–2), then each u16 byte length and 1–256 ASCII bytes |
| direct_addresses | u8 count (0–4), then each u8 family (4 or 6), raw 4/16 IP bytes and u16 port |
| checksum | First 8 bytes of SHA-256(domain || all preceding bytes) |

The checksum domain is the UTF-8 bytes of "mesh-connect/invite-checksum/v1" followed by one zero byte. This is an accidental-corruption checksum, not a digital signature.

Maximum complete binary length is 1024 bytes. Each required 32-byte field must be nonzero; optional account_hint cannot be all zero. Generation and expiry must be positive. At least one reachability hint must exist. Relay list uses strict ascending byte order, with canonical HTTPS root URLs and no userinfo, query, fragment or non-root path. Direct list uses IPv4 before IPv6, then ascending numeric address and port; no duplicates, zero port, unspecified, multicast, IPv6 scope/flow labels. App-configured relay allowlists apply independently to all imported hints.

Text is MC1- followed by unpadded Crockford base32, alphabet 0123456789ABCDEFGHJKMNPQRSTVWXYZ, grouped in sets of five. Case and ASCII whitespace/hyphens are ignored. O/I/L aliases are not accepted; padding bits must be zero; decoded bytes must reencode to the same compact base32 text. Maximum original input is 2048 bytes. The only URI wrappers are mesh-connect://invite/ and bluefin://connect/. No network lookup, general URL interpretation or redirects occur while parsing. QR payloads use the same text without grouping hyphens for capacity.

PNG, JPEG, screenshot, camera QR text, clipboard and manual code all decode to the same capsule. Rendering another representation never mints another invitation. Image import is local and bounded: 8 MiB encoded, 4096 maximum width/height, 8 MiPixels total, 64 MiB decoder allocation budget, at most 32 candidate grids. Distinct recognized invitations in one image are rejected. Theme requires at least four quiet-zone modules, square unmodified data/finder modules, contrast at least 7:1 and dark-on-light colors; Bluefin branding is outside the full quiet zone.

Synthetic conformance fixture: tests/fixtures/invite-v1.json. Expected binary length 273; SHA-256 ffb7c8b7b49869c615e227204d3d99091a254a8b6857c41be47eb838902daac8. No fixture is an active secret or live infrastructure address.

## Connection

The service string is the exact ALPN in both iroh's outer connection and the inner TLS connection. Bluefin uses bluefin/connect/1. Each service owner rejects any different service. Public address discovery is disabled. Only direct hints and application-approved relay URLs are dialed. A relay transports ciphertext and never owns account authorization.

Before any application bytes, the client and server complete an inner TLS 1.3 handshake with the single allowed key exchange X25519MLKEM768, cipher TLS13_AES_256_GCM_SHA384 and signature scheme ML_DSA_65. TLS 1.2, early data, PSK/resumption and classical fallback are disabled. Post-handshake checks repeat the version/group/suite/ALPN checks.

Server certificate DER is pinned to server_certificate_sha256. Both leaf and signature algorithm must be ML-DSA-65 with absent algorithm parameters; public key is exactly 1952 bytes. Certificate chains are not accepted; each leaf is at most 16 KiB. Server name is mesh-connect.invalid. Clients present a self-signed ML-DSA-65 certificate and prove possession through fresh TLS CertificateVerify; the server validates its validity and client authentication use, but that certificate alone grants no account permission.

The server passes its internally constructed authenticated peer context to its account handler. It must not accept a JSON or HTTP header claim of TLS identity. The Begin device key and every subsequent stored challenge/session must match this fresh authenticated key.

The TLS exporter uses label mesh-connect/device-binding/v1, context equal to the service ASCII bytes, and output length 32. This context is never reusable authentication outside its live connection.

After consumption the client retains only ServerAddress (endpoint_id, server_certificate_sha256, service, relay_urls, direct_addresses), its protected device seed and an opaque public session descriptor. It discards the invitation secret. Subsequent connection establishment uses this public address without checking the old invitation expiry. Account/session expiry remains enforced by the live owner.

## Framed onboarding

Each inner-TLS frame is u32 big-endian JSON byte length followed by exactly that many UTF-8 bytes. Allowed length is 1–65536. Frames are sequential; a framing or decoding error closes the connection. Applications must bound connection counts, requests, deadlines and media body sizes separately.

Envelope: {"version":1,"body":{...}}. Unknown versions/fields are rejected. Arrays of bytes are JSON numeric arrays; IP sockets are strings; no base64 alternative. Byte-exact request identity is the canonical serialized typed request hashed by the owner, with stable request_id for each logical operation. A request ID cannot be rebound to changed request data.

Begin body:
{"type":"begin","request_id":[32 bytes],"invitation":{capsule fields},"device_public_key":[1952 bytes],"device_label":"1–128 UTF-8 bytes; no control characters","account_name":"optional signup name"}

account_name is optional, absent/null for login and bounded to 1–64 UTF-8 bytes without controls. Login owners reject a supplied account_name. Signup owners validate and apply their own account naming policy; it conveys no approval.

Begin response:
{"type":"begin_result","request_id":[32 bytes],"challenge_id":"64 lowercase hex","expires_at_unix_ms":0,"passkey_options":{},"device_proof_message":"owner-defined domain-separated proof"}

Finish body:
{"type":"finish","request_id":[32 bytes],"challenge_id":"64 lowercase hex","passkey_response":{},"device_signature":[3309 bytes]}

Finish response:
{"type":"finish_result","request_id":[32 bytes],"session":{}}

Error body:
{"type":"error","request_id":[32 bytes],"code":"invalid|expired|used|denied|retry_later"}

request_id is nonzero, client-generated and stable across retries; Finish has a distinct ID from Begin. A retried result is released only to the same authenticated device over a fresh connection; copied Finish bytes or a session descriptor never become bearer authority. Server-to-server or application-specific methods must retain the same live-device check.

## Account and global single use

Only the live durable account owner validates generation/epoch, expiry, secret and stored grant policy. Capsule account_hint/intent/service do not add scope. The invitation secret and QR possession do not replace the required passkey.

An owner may consume/reserve an invitation atomically at Begin into exactly one durable device-bound challenge; this reservation never expires back into an open invitation. Its exact same-device retry can recover that challenge. Alternatively, an owner may atomically consume at Finish with the device grant. In both cases, no account grant exists until successful owner passkey and device checks; the final challenge consume, credential counter update and grant/receipt commit are atomic. Signup consumption and creation of one durable signup request commit atomically; a pending approval can resume using proof-bound state, not the consumed invitation. Autoapproval is an owner policy, never a field the importing client can enable.

The durable consume operation permits exactly one winner across simultaneous devices, all image/text representations and owner replicas. An exact same-device retry can recover its previously committed public result; it cannot mint another grant. Unknown/offline/stale replicas fail closed. Random live authority epoch plus fenced authority generation prevent database snapshot restore or process restart from making old open invites usable again. A separate proof-bound continuation, with owner approval and expiry, handles pending requests.

Native passkeys require an application-associated relying-party domain. An imported QR cannot grant an app permission to use an arbitrary RP. Existing registered RP/origin policy remains authoritative; the mesh transports challenges and validated responses.

## Ownership and qualification

This Bluefin crate retains the existing GPL-3.0-only license. It contains newly authored neutral transport/codec source, with no private application source imports. Other implementations can use this explicit protocol description and synthetic fixture to verify compatibility while retaining their own source/license boundaries.

The inner TLS provider is rustls-provider-rustcrypto-pq 0.1.1, public upstream ctf16/rustls-provider-rustcrypto-pq at ca7acfa4d980ef70a2f3ed3eac52ae198e656609. Primitive signing uses RustCrypto ml-dsa 0.1.1. Both must be admitted and qualified by the existing build/security process; no production rollout is implied by this source change. Outer iroh currently uses its existing ring transport provider. The strict inner profile, rather than the outer endpoint key, authenticates protected application traffic.

## Bluefin session revocation extension

Inside a fresh authenticated PQ channel, send an envelope with version 1 and a
body containing type `revoke_session`, the 64-character `session_id`, a random
32-byte `request_id`, and the current connection's 32-byte exporter `binding`.

Persist the request identifier before sending. After a lost response, reuse that
identifier, the session identifier, and the protected device key. A new
connection supplies its fresh binding. The response has version 1 and body type
`session_revoked`, with the matching session and request identifiers.

The account owner commits terminal revocation and one immutable receipt in its
existing database. Existing SQL triggers remove the server-only bearer in that
transaction and prohibit reminting. Exact retries recover the retained result;
a different request identifier for the recorded revocation is refused. The
owner retries remaining live-session cleanup before acknowledging.

Local UI logout can happen immediately. An uncertain remote result retains
protected recovery material until the operation succeeds or the user explicitly
discards it. A public session descriptor cannot revoke authority. The original
device may revoke an expired or disabled account's session because it only
removes its own access. This Bluefin extension does not change generic Node
onboarding requests.
