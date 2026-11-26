# Apple launch support

The launch requirement covers Apple device models introduced during the three
years before release, together with supported models already covered by the
clients. Qualification must include the different display sizes and input
methods in that range. It must not exclude a model merely because a newer model
exists. The operating-system qualification target is the current and preceding
major release supported by each device. Existing older-system compatibility can
remain without expanding the first-release test matrix.

Bluefin provides native applications for iPhone, iPad, Mac, Apple TV, Apple Watch
and Apple Vision Pro. These are development targets, not a declaration that all
release tests have passed. Test compact phones, both iPhone Duo display states,
iPad window resizing and multitasking, Mac windows and keyboard navigation,
television remote focus, Watch screen sizes and spatial input. Playback quality
depends on the actual codec, display, audio route and system capabilities.
Show verified playback quality rather than inferring it from a device name.

The paired Watch setup should authorize a separate Watch session through the
signed-in iPhone, without asking the person to retype credentials or copying the
iPhone's session. Retain an independent setup path where a paired companion is
unavailable. Account changes, late authorization responses and retryable cleanup
must preserve the correct session owner. Native Watch playback should support
the system's media controls and the actual audio route.

Siri integrations should expose authenticated library search, item information,
availability and playback through supported App Intents. Answers must come from
the viewer's current authorized library. Requests for additional media use only
generic server-advertised actions when enabled by the server administrator.
Do not embed upstream discovery connectors in a native client or imply Siri can
answer arbitrary questions without a supported data or intent path.

Apple devices that do not install third-party applications use their supported
system integrations. HomePod and compatible speakers use AirPlay and the system
audio controls. AirPods and other supported audio devices use the native audio
route. Bluefin must not present these as separate installable applications or
claim device-specific audio formats without measuring the selected route.

## HomeView

Support both tabletop and wall-mounted HomeView through documented platform
capabilities:

- Safari provides the Bluefin web application with semantic, labeled controls,
  native media playback and captions. Test short and square viewports, including
  WebKit's recommended 448 by 364 CSS pixels, without identifying the device by
  its user-agent string. Controls must remain reachable by touch, keyboard and
  the browser's supported accessibility input.
- iPhone widgets provide glanceable, privacy-aware information through
  WidgetKit. A widget must distinguish unavailable, stale and signed-out state,
  and protect personal library information on shared displays. Widget actions
  need a real supported execution path; a decorative button is not acceptance.
- Compatible CarPlay audio and video applications can appear through the
  paired iPhone. Keep native media controls, account isolation and cleanup. Use
  only the system's reported playback capabilities. Video availability in a
  vehicle remains controlled by the system.
- Use supported Siri, Handoff and AirPlay behavior where it is available.
  Never infer that HomeView can execute an iPhone-only intent, open an arbitrary
  native application or continue a session merely because that path works on
  an iPhone.

HomeView has no separate third-party App Store. Do not advertise a HomeView app
download or invent a standalone HomeView application target. CarPlay
entitlements require approval and correct provisioning. Apple's video
entitlement cannot be combined with other CarPlay entitlements on iOS 26.3 and
later, so an audio/video deployment design must respect that restriction.
Development builds must not claim approved projection before it is provisioned
and tested.

Record browser, simulator, projected-widget and physical-device results
separately. A responsive browser capture proves layout in that browser; it does
not establish HomeView playback, voice input, widget interaction or household
privacy behavior. Unsupported capabilities remain unavailable with a clear
user-facing state.

Primary references:

- [Safari on Apple HomeView](https://webkit.org/blog/18608/safari-on-apple-homeview/)
- [CarPlay and HomeView app support](https://developer.apple.com/carplay/)
- [HomeView software capabilities](https://support.apple.com/en-us/127196)
- [CarPlay video entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.carplay-video)
- [Widget and HomeView agreement updates](https://developer.apple.com/news/?id=17mwd36z)
