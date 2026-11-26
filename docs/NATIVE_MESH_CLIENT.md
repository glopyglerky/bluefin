# Native mesh client continuation

This change connects the preserved Rust mesh client to the Apple application.
The Rust/browser protocol checks require exact-source Forge results. The Apple
UI, SDK build, signing, camera, Bonjour permission, passkey prompt, and physical
pairing journey remain separate acceptance gates.

## Ownership

The existing account owner issues and redeems the invitation. Native
presentation does not grant a role or create another account ledger. A login
invitation connects another device to the same account and still requires that
account's passkey. It is not a library-sharing invitation for another person.

The shared Rust core validates capsule syntax, expiry, service, certificate
pin, browser origin/path/challenge, callback identity and protocol responses.
Swift adapts Keychain, AuthenticationServices, camera/photo/files, Bonjour and
the application lifecycle. This is not a claim that the complete app or its
dependencies are written in safe Rust.

## Retained pairing

Before Begin, the device-only pending vault stores a fresh device identity and
the Begin/Finish operation identifiers. Begin is retried with the same identity
and request identifier after a lost response. A successful Begin replaces the
invitation secret with the pending ceremony and public server descriptor.

The browser page must be HTTPS at the exact origin supplied through the pinned
connection. Its path is `/Bluefin/Accounts/Mesh/Passkey` and the fragment binds
the exact challenge and a separate browser capability. No native entitlement
claims every arbitrary relying party.

On iPhone, Catalyst and visionOS, AuthenticationServices opens the page and
checks the returned custom-scheme callback. Apple TV displays that same
device-bound browser continuation as a QR. A user completes the passkey on a
phone and chooses the finish action on the TV. The server checks the passkey
proof and original device identity; an early or forged callback cannot grant
access.

The exact randomized Finish signature is saved before transmission. Retry
reuses the prepared request. A received session descriptor is retained before
the first ordinary library read, so a lost/failed library request does not
leave an untracked device key. Signup approval resumes using the distinct
device-bound handle.

## Saved session and media

`AccountSession` contains a public vault reference, not the mesh device seed.
A separate ThisDeviceOnly Keychain record stores the protected device identity.
RustBridge checks reference, origin, user and device before attaching the
connection to a request. A missing protected record fails closed. It never
falls back to the HTTPS bearer path.

Artwork calls the authenticated mesh media receiver. Playback continues to use
the captured Rust loopback adapter. The Watch companion path refuses to export
a mesh account; the Watch needs its own separately admitted device connection.

## Discovery

The native browser uses ordinary Bonjour with
`_bluefin-mesh._udp.local.` and no peer-to-peer discovery override or subnet
scan. Unknown sightings can appear as nearby names but cannot establish trust.
Rust validates bounded TXT/address observations and matches the saved endpoint,
service and certificate pin before refreshing a saved route. The next
connection must still authenticate the pinned peer.

Bonjour starts while the app is foregrounded and stops in the background.
Background audio retains its media route. Idle mesh media routes close when the
app backgrounds. No always-running iOS storage/server claim follows.

The bundle's `BluefinMeshRelayAllowlist` is the trusted relay configuration.
Invitation-provided relay hints cannot expand it. The empty default admits no
public relay implicitly.

## Sign-out

Sign-out retains a separate protected revocation intent before clearing the
active account. Ordinary requests refuse a connection with that pending intent.
Local playback cleanup and the existing offline-download join run before the
active device-vault record is retired.

The request uses `revoke_session`, the same durable request ID on every retry,
and a fresh exporter binding for every transport connection. Acknowledgement
must name the same session and request ID. An unavailable server leaves
revocation pending and the UI reports it as unconfirmed.

Only the cleanup queue can read the retained revocation identity. Foreground
activation retries it. Explicitly erasing pending keys abandons remote
reconciliation and does not claim the remote session was revoked. A restart
with a retained sign-out intent fences the previously active session.

## Input and invitation controls

PNG/JPEG/screenshot imports share the Rust binary image decoder with an 8 MiB
outer cap. Camera scanning submits one bounded QR value to the same parser and
refuses multiple codes. No image or code opens an arbitrary HTTP request.

Connections settings can create and display a one-use invitation for another
owned device, copy its link locally with clipboard expiry, and share the
themed PNG through the system share sheet. Status polling hides claimed,
expired or unconfirmable invitations. Every exported representation retains
the same authority-issued capability.

## Remaining acceptance

Required physical journey:

1. Build and sign the exact source through an admitted Apple executor.
2. Open/paste an invitation on the iPhone and complete a real passkey prompt.
3. Observe a successful ordinary library request through the PQ connection.
4. Play media and fetch artwork through the mesh receiver.
5. Relaunch, deny/re-enable LAN permission, change route, and reconnect.
6. Exercise the TV continuation, with the TV's key performing Finish.
7. Lose Begin, browser Verify, Finish and revoke responses and recover using
   retained exact requests.
8. Sign out during media/download activity, observe local stop, then confirm
   idempotent remote revocation after an outage.

A phone-owned imported music library, phone-to-TV resource serving,
non-transitive grants to other principals, serverless browser execution,
native platform passkey entitlements and a complete Music Assistant/Home
Assistant implementation are additional product work. This client change does
not qualify them.

Primary platform guidance:
- https://developer.apple.com/documentation/authenticationservices/aswebauthenticationsession
- https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy
