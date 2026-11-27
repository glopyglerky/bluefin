# Apple SyncPlay seam — BDP-064 / SYSPLAN-027

Static source inspection at bdd8c52110a8dc14194f5e12aaac5c9bd13b9234.
Method SHA256: 5744f9bc85c2fb61d01bba3434f200312efce584b1d2cbbd73f7a42219a3dd30.

The Apple Rust crate now exposes the existing no_std `bluefin-sync-play`
implementation as `bluefin_apple_core::sync_play`. It supplies RoomClock,
ClockExchange and the existing membership/media scheduling types without a
second clock implementation. This is a Rust integration seam, not a Swift
room operation or native playback implementation.

The minimal next shared seam is extraction of universal/src/sync_play.rs's
RoomRequest, response validation and request_using into a shared crate with
an injected request callback and a transport-independent error type. Its
current private crate::protocol dependency prevents direct dependency reuse.
That extraction is outside this worker's owned paths. Do not copy the wire
implementation into Apple: retain join -> Current admission verification,
create -> admission -> queue -> exact-item verification and tick bounds.

The Apple adapter should bind the callback to Api::request with auth=true,
preserving its mesh_connection::request authenticated Iroh route. Capture
the exact explicit device identity and current account generation at operation
start; reject stale completion after account change or reconnect. A new
portable Membership epoch is required on either transition. Api currently
allows a fallback device identity, so room operations must require an explicit
identity before invoking it. No room receipt authorizes media: resolve each
item through the existing independently authenticated playback path and keep
session capabilities local. A shared room cannot transfer a session token.

Native Apple SharePlay is untouched. Swift playback, lifecycle, account
generation ownership and FFI wiring remain with their existing owners.

No builds, tests, lockfile resolution, installs or runtime qualification ran:
the executor is blocked and no Apple executor exists. Parent owns Forge and
production. Required subsequent checks include clock expiry, stale epoch and
decoder callbacks, denied join despite HTTP success, denied queue item,
account/device change during admission, and mesh control-path preservation.
Cargo.lock needs admitted dependency resolution before --locked qualification.
