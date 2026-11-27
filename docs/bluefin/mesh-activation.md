# Mesh listener activation

The listener is opt-in through `BLUEFIN_MESH_CONNECT_CONFIG`, a bounded JSON
configuration file read by the server. An example shape is:

```json
{
  "secret_seed_file": "/private-bluefin-state/mesh/identity.seed",
  "bind_address": "192.0.2.10:4433",
  "relay_urls": [],
  "web_launch_url": "https://app.example/web/bluefin/",
  "allow_experimental_pqc": true
}
```

The example address and path are placeholders. The deployment owner must select
the actual service-owned state directory and an admitted bind address. The
existing identity store requires a private directory, refuses symlinks, maintains
a private seed file and holds an exclusive process lease for the listener's
lifetime. No system VPN is required.

The PQ provider is still explicitly experimental. Enabling the listener does not
approve any relay. Relay URLs must be chosen and admitted by the operator; the
server supports at most two, and the browser separately requires an enabled,
matching relay policy. An empty relay policy does not provide browser transport
reachability.

`web_launch_url` optionally names a separately hosted HTTPS application page.
It accepts no credentials, query or fragment. If omitted, invitations use
`/web/bluefin/` at the configured published account origin. All rendered forms
retain one invitation capsule and one authority ledger; the capability travels
only in the URL fragment. Invalid launch configuration is rejected before
replacing the live issuer epoch.

The launch page's origin does not change the account API, WebAuthn relying-party
ID, allowed passkey origin, or trusted ceremony callback page. When the app is
hosted on a different origin, the browser still needs to reach the real relying
party to complete its passkey ceremony. Publishing static assets alone does not
make a LAN-only relying-party endpoint reachable from elsewhere.

Before describing external browser pairing as available, qualify the chosen
relay path, the browser policy, a trusted and reachable relying-party ceremony
path, and a real one-use invite/passkey/library journey. Local file playback and
ordinary same-origin HTTPS login are independently available interfaces; they
do not silently replace a pinned PQ connection.

## Restoring account data

A scheduled Bluefin backup restore that includes the database invalidates mesh
authority on its staged SQLite file before the backup owner replaces any live file. This removes
mesh device bearers, sessions, signup requests, invitation state and completion
receipts. The next activation creates a fresh issuer, and devices must pair
again through a new owner-approved invitation. Ordinary restarts retain their
existing session behavior.

Unrelated Jellyfin device rows, accounts, passkeys and media data retain the
existing backup contract. An attempted SQLite invalidation that fails replaces
no live file. Short or non-SQLite files retain the existing database-open
validation behavior. This is a Bluefin extension to restore behavior;
it does not change the Jellyfin HTTP contract or claim upstream parity for mesh.

This hook covers the server's scheduled backup restore only. Direct database
replacement, host snapshots and filesystem rollback can bypass it. Those paths
still require a surviving current recovery authority or explicit re-enrollment
before exposing mesh access. They are not qualified by this hook.

Qualification must restore a backup taken before revocation, then exercise both
mesh session authorization and the ordinary HTTP handler with the old mesh
bearer. Both must refuse, while an unrelated device and the account data remain
usable. Include failure before the staged-file swap and a database without mesh
tables. Source-authored regressions do not replace the admitted Forge run and
actual server acceptance.

### Actual binary restore acceptance

The isolated subprocess journey lives in
`crates/ferrofin-api/tests/mesh_restore_process.rs`. It starts the supplied
candidate executable with a disposable database, loopback HTTP/PQ endpoints and
no public relays. The expected executable SHA-256 is mandatory and is checked
before launch. All users, passkeys, grants and bearer rows come from the server's
real APIs; the driver's database connections only read evidence.

Run the named ignored test inside an admitted Forge job after the candidate
artifact has been built and its digest retained:

```sh
BLUEFIN_TEST_SERVER_BINARY=/path/to/qualified/ferrofin-server \
BLUEFIN_TEST_SERVER_SHA256=the_retained_lowercase_sha256 \
cargo test --locked -p ferrofin-api --test mesh_restore_process \
  actual_binary_restore_retires_old_mesh_authority \
  -- --ignored --exact --nocapture
```

The job receipt must record one executed, passed test and zero ignored tests.
An ordinary suite invocation that reports this test as ignored is not acceptance.
The existing `BLUEFIN_TEST_FFMPEG` and `BLUEFIN_TEST_FFPROBE` overrides may select
the job's admitted media tools.

The journey issues a new-account invitation, uses a software WebAuthn
authenticator through the real browser-proof HTTP adapter and strict PQ
connection, and obtains explicit administrator approval. It proves a live mesh
request before HTTP `Backup/Create`, revokes that device over PQ, then calls
HTTP `Backup/Restore` and waits for the next actual server lifetime. A
post-backup user must disappear, while the original administrator's unrelated
HTTP token remains valid. The restored old session, exact Finish retry, unused
invitation and internal bearer must fail; a new passkey login must succeed and
remain valid after an ordinary restart. An explicit application-level denial
response is required; a socket error alone is not authorization evidence.

This is actual-binary acceptance with both peers on one host and a software
authenticator. It does not establish browser secure-context behavior, physical
iPhone/Apple TV behavior, relay traversal, independent-instance federation, or
arbitrary filesystem rollback protection. The test is authored source until an
admitted job retains its execution result.
