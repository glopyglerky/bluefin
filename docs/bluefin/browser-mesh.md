# Browser mesh connections

## What this source adds

The Rust/Wasm Bluefin application accepts the same invitation capsule as the
native clients. A launch link places that capsule in an HTTPS URL fragment.
The page removes the fragment before loading the Wasm application or media
scripts. A native client importing this link extracts the capsule locally
without visiting the web origin.

The browser implementation uses iroh 1.4.1 with its default discovery and
relay choices disabled. It connects only through a relay selected by the
application's separately supplied allowlist. Browser iroh is relay-only.
The browser sandbox does not expose the UDP sockets used by native iroh
for direct connections.

The inner connection shares the native device identity and TLS verifier
source. It requires the pinned certificate, TLS 1.3, X25519MLKEM768,
TLS13_AES_256_GCM_SHA384 and ML-DSA-65 device authentication. Early data,
resumption and classical fallback remain disabled. The browser supplies a
web clock to rustls and browser-compatible timers to iroh operations.

These are authored integrations. Read the exact-source Forge receipt before
claiming an accepted build or browser journey. A successful Wasm compile
does not establish browser passkeys, relay interoperability or playback.

## Browser trust and launch

The HTTPS certificate authenticates the page that loads the application.
The invitation authenticates a different object, the application peer.
Both checks matter. A QR code does not make a self-signed website trusted,
and an iroh certificate pin does not replace browser certificate validation.

Deploy the static shell under an existing publicly trusted HTTPS origin.
The owner can use normal ACME certificate issuance, including DNS-01 when
the web server is not publicly reachable for HTTP validation. Certificate
issuance does not itself make the browser able to reach a private address.

An example launch is:

`https://app.example/web/bluefin/#bluefin-connect=MC1-...`

The request to the static host contains the path but no invitation fragment.
The page and its code still handle the capsule, so the application origin
and its delivered code remain part of the trust boundary. No third-party
QR decoder or upload service is used.

No certificate-warning bypass, mixed-content fallback or automatic custom
CA installation is implemented.

## Admission and local state

The application loads `mesh-policy.json` from its own HTTPS origin with
redirects and ambient credentials disabled. The shipped policy is closed:

```json
{"version":1,"enabled":false,"relays":[]}
```

An operator must supply an admitted HTTPS relay before enabling browser
pairing. The invitation does not authorize relay egress. A direct-only
invitation or one with no admitted relay fails before Begin can claim it.

The browser stores one protected connection journal in IndexedDB. An
origin-bound, nonextractable WebCrypto AES-GCM key encrypts its contents.
The journal uses compare-and-swap revisions so tabs cannot silently
overwrite a different device identity or signed request.

The journal retains the device identity and Begin/Finish operation IDs
before sending Begin. It retains the exact signed Finish before sending
Finish. An unknown Finish outcome retransmits those same bytes. Once
accepted, the journal replaces the invitation with the device-bound
session descriptor and removes its secret.

This protects stored bytes against accidental plaintext disclosure. It
does not make a compromised application origin harmless. The origin's
running code can use the key and receives decrypted state while the app
runs. Browser storage availability and eviction remain platform behavior;
a browser installation is not a promised durable storage server.

## Pairing and reconnection

The primary browser journey is:

1. Open the trusted HTTPS launch, paste a code, or import a PNG/JPEG QR.
2. Check the service, expiry and admitted browser route.
3. Save identity and operation IDs, then claim at the existing account owner.
4. Verify a passkey at the exact origin supplied by the pinned peer.
5. Save the exact Finish and send it through fresh mutual TLS.
6. Save the public device descriptor and load the authenticated library.

When the app and RP origins differ, the UI opens the existing HTTPS
passkey continuation in another tab. That page stores proof with the
original device's pending challenge. It does not return a bearer to the
app. The user returns to Bluefin and finishes with the same saved device.

Signup requests keep a separate protected handle while administrator
approval is pending. They do not repeatedly redeem the invitation.

Resume reloads protected state and reauthenticates the peer. A sign-out
request closes local media and endpoints before any fallible storage or
remote step, retains a revocation request ID, and waits for the peer's
device-bound revocation acknowledgement before clearing the journal.
Failure preserves the difference between local shutdown and confirmed
remote revocation.

New controls and media streams refresh the protected journal. Cross-tab
journal notifications, explicit retirement notifications and foreground
checks stop a replaced or retiring connection. The live server remains
the authority for account access and revocation.

## Library, playback and invitations

The existing Rust controller dispatches a mesh session through the device
connection. It cannot serialize its local mesh reference into an HTTP
Authorization header. Ordinary HTTPS account sessions keep their existing
transport.

For mesh playback, the existing media worker sends a scoped request to
the owning browser document. Rust opens the pinned media request and
returns one bounded 8 KiB chunk for each stream pull. The worker receives
no device seed or account bearer. It retains the existing item scope,
Range forwarding, HLS URL rewriting, idle deadlines and stream limits.
Cancel, account retirement and sign-out close the corresponding Rust
connection.

The Account section can create a one-use code for another device of the
current account, display a Bluefin QR, save its PNG, or copy its code or
browser launch link. The UI checks the authority's status and hides a
claimed or expired code.

This account-login capability must not be relabeled as permission for a
different person to access the sender's account. The broader user request
for pairwise resource sharing needs a distinct principal and scoped grant.
A-to-B plus B-to-C must not grant C access to A. This source does not
establish that new resource-sharing authority or a browser storage owner.

## Required qualification

Use the current admitted Forge lane and exact committed lockfiles.

- Native shared codec and transport regressions, including launch-link and
  web-QR round trips.
- `mesh-connect` browser/images Wasm compile.
- `bluefin-universal` Wasm compile and matching wasm-bindgen generation.
- JavaScript syntax checks for the platform bindings, player, media worker
  and passkey continuation.
- Real trusted-HTTPS browser plus admitted relay: Begin, user-verified
  passkey, Finish, library request, audio playback and media seek.
- Wrong pin, wrong device, unadmitted relay, unavailable relay, direct-only
  invitation, expired code and simultaneous redemption refusal.
- Lost Begin and Finish replies, reload during each pending operation,
  concurrent tabs, database unavailability, sign-out during media, and
  restored pending revocation.
- Browser navigation/back/refresh without invitation secrets in requests,
  page URLs after startup, logs, service-worker messages or media URLs.

The static shell does not currently provide an offline application cache.
It also does not embed a durable per-installation library/storage authority.
Phone-only import, background peer service, friend resource grants and
Music Assistant/Home Assistant integration belong to the wider work.

## Primary sources and pinned mechanisms

- [iroh browser documentation](https://docs.iroh.computer/languages/wasm-browser):
  relay-only browser connectivity, default-feature requirements and E2E
  encryption through relays.
- iroh 1.4.1, `src/endpoint.rs`, read from the retained checksum-verified
  crate archive: explicit relay mode adds a relay transport after clearing
  defaults; browser builds omit native IP transports.
- rustls 0.23.46, `src/client/client_conn.rs` and
  `src/time_provider.rs`, read from the retained crate archive:
  configurable time provider with the existing strict verifier.
- n0-future 0.3.2 and web-time 1.1.0, read from retained crate archives:
  browser timers and wall time.
- [WebAuthn Level 3](https://www.w3.org/TR/2026/REC-webauthn-3-20260825/):
  secure contexts and exact RP/origin validation.
- [Let's Encrypt challenge types](https://letsencrypt.org/docs/challenge-types/):
  DNS-01 domain control for servers without public HTTP validation.

The existing provider provenance and licenses remain in
`vendor/rustls-provider-rustcrypto-pq/PROVENANCE.json`.
The application remains GPL-3.0-only. The browser platform APIs and
cryptographic dependencies remain part of the implementation boundary;
this source makes no whole-stack memory-safety or distribution-clearance
claim.
