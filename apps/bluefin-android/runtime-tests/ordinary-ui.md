# Ordinary UI acceptance

Run only `OrdinaryUiRuntimeTest#ordinaryUiJourney` against an owned synthetic server.
Reuse the qualified production APK and JNI; package only instrumentation. The host
must drive ordinary MainActivity through the device panel, including every account
field and all library, playback and sign-out actions. No injected account or UI.

The observer enables the existing debug-only in-process HTTP exception for exactly
127.0.0.1:18765, emits `ORDINARY_UI_READY`, and observes account/cleanup state and
existing screen/service players on Main. This transport differs from production
HTTPS and provides no TLS parity evidence. It never creates a player or controller.

The host records visible UI, exact actual-server identity and synthetic library
admission, account separation and player time/decoder counters. It writes PASS to
the app-private `ordinary-ui-finish` event only after the accepted journey and UI
sign-out; any other verdict fails. The observer is bounded to twenty minutes.
Restore only owned routes/settings and dispose fixture credentials after evidence.
Record partial/red journeys explicitly; observer success alone is not acceptance.
