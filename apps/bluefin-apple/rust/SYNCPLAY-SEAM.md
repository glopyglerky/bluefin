# Apple SyncPlay seam — BDP-064 / SYSPLAN-027

Static source inspection at bdd8c52110a8dc14194f5e12aaac5c9bd13b9234.
Method SHA256: 5744f9bc85c2fb61d01bba3434f200312efce584b1d2cbbd73f7a42219a3dd30.

The Apple Rust crate now exposes the existing no_std `bluefin-sync-play`
implementation as `bluefin_apple_core::sync_play`. It supplies RoomClock,
ClockExchange and the existing membership/media scheduling types without a
second clock implementation. This is a Rust integration seam, not a Swift
room operation or native playback implementation.

The shared room wire contract now lives in bluefin-sync-play's optional
requests module. Both clients use its typed operations, room-bound controls
and admission readback. The contract depends on alloc and leaves I/O with the
caller. Apple exposes syncPlay through the existing Rust bridge and
Api::request. The room operation requires a persistent, explicit device ID
and refuses the shared platform fallback.

The native player still needs to bind room observation and controls to the
current account generation. Discard stale responses when the account or
connection changes. Playback still uses the existing independent permission
and lease checks. Keep one synchronization controller per player so native
SharePlay and a Bluefin room cannot issue competing commands.

Tests cover missing or wrong-room admission, exact item confirmation and
room-scoped command refusal. The lock entries use dependencies already
present in each workspace. Forge and an admitted Apple device executor are
required to establish compilation, playback and installed-device behavior.
