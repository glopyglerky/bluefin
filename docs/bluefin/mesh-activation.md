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
