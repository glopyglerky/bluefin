# Native acceptance of real server model output

`NativeServerModelTests` consumes the four public synthetic files retained by the
real authenticated server model fixture: `public-manifest.json`,
`original-encoded.flac`, `accompaniment.wav`, and `lead-vocal.wav`.

Build the app-hosted tests for testing against the candidate Rust archive. Copy
only those four files into a new `ServerModelFixture` directory inside the built
test bundle, then run `NativeServerModelTests` with Xcode's test-without-building
action. Record producer source, archive, output and test-bundle hashes alongside
the test result. Never stage the private preparation receipt or model weights.
The production app has no dependency on this external fixture. The test explicitly
skips when it is absent; a skip is not model-output acceptance.

The test uses the exact public manifest and encoded stems without rewriting their
identity, hashes, PCM duration or timing. It passes them through authenticated
loopback HTTP, the actual Rust FFI downloader, native admission and AVPlayer.
Checks cover the shared playback clock, vocal control, seek, original playback
restoration and last-consumer temporary-file cleanup. Original synthetic FLAC
playback uses its local file. The test adds ten milliseconds of catalog rounding
to verify that the exact PCM duration remains the public admission authority.

This proves server-output/native compatibility. It does not prove a live remote
server journey, model perceptual quality, provider rights, microphone latency,
hardware routes, UI appearance or Apple Music Sing parity.
