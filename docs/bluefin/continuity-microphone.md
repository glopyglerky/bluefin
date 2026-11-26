# iPhone microphone on Apple TV

Bluefin implements Apple's public Continuity audio path on tvOS 17 and later.
This is local voice monitoring, not access to Apple Music Sing internals or proof
of equivalent acoustic quality. There is no custom network microphone protocol,
recording, video capture, cloud processing, server endpoint, or remote account.
Apple's system Continuity transport carries audio from the selected iPhone to TV.

## Parent integration

Include `KaraokeContinuityLifecycle.swift`, `KaraokeContinuityAudio.swift`, and
`KaraokeContinuityControls.swift` in the Apple targets, and include
`KaraokeContinuityTests.swift` in the test target. No Rust FFI change is required.
Within a tvOS-only, `if #available(tvOS 17.0, *)` branch, retain one
`KaraokeContinuityMicrophone` for the karaoke playback lifetime and render
`KaraokeContinuityControls(microphone: model)` beside the lyrics controls.
The model is main-actor isolated. Its `stop()` hook must run BEFORE account/server
change, playback stop/item replacement, leaving Sing, restoring original audio,
or the parent changes the shared AVAudioSession. View disappearance and scene
inactivity also stop this lane, but do not replace those playback-owner hooks.
Never run the wired iOS monitor and Continuity monitor concurrently.

On tvOS 15/16 render an unavailable message. iOS 15 keeps KaraokeMicrophone's
wired-headphone-only path; speaker, USB and wireless output remain rejected.
The existing `KaraokeMicrophone.start()` cannot present a picker. The parent must
render the new TV control instead of routing a TV enable button to that method.
Project generation, Info.plist, playback hooks, signing and devices are parent-owned.

Provide nonempty `NSMicrophoneUsageDescription` and `NSCameraUsageDescription` in
the tvOS Info.plist before presenting the system Continuity picker. Suggested text:

- Microphone: "Use your selected iPhone microphone for local karaoke voice monitoring."
- Camera: "Connect your iPhone using Apple's Continuity device picker. Bluefin does not capture camera video."

The control checks both descriptions before presentation. The app requests only
audio capture authorization through `AVCaptureDevice.requestAccess(for: .audio)`;
it creates no AVCaptureSession or camera input. The camera description covers the
camera-capable system picker, not an implemented video feature. Preserve the iOS
microphone usage description. No special entitlement, background audio addition,
provider privilege, local-network exception or private API is added by this lane.
Qualify actual system prompts and shipping signing on devices before release.

## Consent, routing and lifecycle

Connect opens Apple's native full-screen Continuity picker through SwiftUI's
`continuityDevicePicker`. Unsupported hardware is detected using
`AVContinuityDevicePickerViewController.isSupported`. A returned device enters
connected-muted state. Enable is a separate explicit action that requests audio
permission and starts an AVAudioEngine only if granted. A denied/restricted grant
requires Settings and reconnection; OS prompts cannot be programmatically dismissed.
Cancel/stop invalidates selection and permission generations, so a late grant cannot
start audio. Task cancellation is checked after permission returns. Stop itself
silences any owned engine immediately and does not wait for the prompt.

The adapter selects only an available input whose UID appears in the chosen
AVContinuityDevice.audioSessionInputs, sets preferredInput, and checks the actual
current input. It uses playAndRecord/voiceChat, enables voice processing before
starting, keeps processing bypass disabled, and refuses an unprocessed fallback.
HDMI, line-out, built-in speaker and wired headphones are allowed TV output ports;
AirPlay, Bluetooth and unknown outputs are rejected. Output-route change, invalid
input, device disconnect, engine configuration stop, interruption, media-service
loss/reset and backgrounding stop capture. No event automatically resumes it.
Input-availability and device-connected KVO supplement route notifications.
Setup notifications are ignored only during synchronous route setup or when the
selected input and output UIDs remain valid and the engine still runs. Every
startup performs final selected-input/output checks before raising gain.

Voice gain defaults to 0.1 and is capped at 0.25; nonfinite gain becomes zero.
TV controls offer quieter/louder buttons and a stop action. System voice processing
mitigates echo; it does not guarantee feedback suppression for every room, receiver,
TV delay, speaker volume or microphone position. Start at low TV volume. No acoustic
latency, round-trip delay, feedback threshold or singing fidelity has been measured.
The voiceChat processing may alter singing tone and accompaniment behavior.

Teardown zeros gain, stops/resets the engine, releases input/device observations,
and restores the prior category, mode, options and preferred input. Partial startup
owns rollback before the first session mutation. A resource owner also tears down
on release. Restore errors are visible during explicit stop. AVAudioSession has no
public previous-active-state getter, so this lane keeps the shared playback session
active and leaves final activation/deactivation to the playback owner. It does not
claim to restore an unknowable active state. Parent must stop before reconfiguring
the shared session, otherwise a stale snapshot could overwrite the parent's change.
Deinit restoration is best effort; explicit stop is the observable integration hook.

## Evidence and remaining qualification

Six host XCTest cases execute the production lifecycle coordinator with a fake
audio adapter: separate consent/gain bounds, denied permission, stale picker/grant
rejection, cancellation/disconnect during permission, partial-start rollback and
restore failure, repeated stop and selection cancellation. They do not execute a
real authorization prompt, Continuity link or audio hardware. SDK source checks use
installed Xcode 27 SDKs with iOS/tvOS 15 deployment targets, including simulator and
device triples. No physical device or simulator UI was opened by this lane.

Parent must qualify compatible Apple TV and iPhone hardware with the actual app:
picker dismissal/cancel, deny/allow/revoke prompts, delayed input availability,
selected-device identity, conservative gain, output and sample-rate changes,
interruption/phone lock/disconnect/reconnect, backgrounding, permission cancellation,
media-service reset, failed restoration, account/item teardown, and accompaniment
continuity. Measure acoustic latency, echo/feedback and vocal quality before any
Sing parity claim. Apple documents Apple TV 4K second generation or later and
iOS/iPadOS 17+ for its sample; honor runtime isSupported rather than inventing a
broader hardware promise. Simulator cannot establish Continuity acceptance.

## Public references

- [Apple's tvOS Continuity sample and audio configuration](https://developer.apple.com/documentation/avkit/supporting-continuity-camera-in-your-tvos-app)
- [AVContinuityDevice](https://developer.apple.com/documentation/avfoundation/avcontinuitydevice)
- [Continuity picker](https://developer.apple.com/documentation/avkit/avcontinuitydevicepickerviewcontroller)
- [Audio capture authorization](https://developer.apple.com/documentation/avfoundation/avcapturedevice/requestaccess(for:completionhandler:))

Installed SDK declarations were checked for tvOS17 availability of the picker,
AVContinuityDevice.audioSessionInputs, audio authorization and voice-processing APIs.
The tvOS-unavailable AVAudioSession.requestRecordPermission API is used only in the
iOS implementation, never in the TV path.
