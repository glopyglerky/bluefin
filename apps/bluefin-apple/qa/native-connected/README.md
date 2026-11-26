# Native connected QA fixture

Test-only Rust service, excluded from product Xcode Sources. It owns one synthetic account and one original 180-second PCM track. The accompaniment is a 220 Hz tone; the lead is a 440 Hz tone. The source mix is their sum. Words and lead/partner/together timing are manually authored. This does not demonstrate singing quality, automatic alignment, separation, provider access, or Apple Music Sing parity.

Build with the Rust toolchain and an external target directory. Run the resulting executable with three arguments: loopback port, current ISO UTC schedule date, and a test-owned stop sentinel path that does not yet exist. Creating the sentinel drains and joins workers. The service otherwise exits after one hour. Eight concurrent connections, 131072 request bytes, 60000 body bytes, a 15-second read deadline, 5-second individual reads and 10-second writes bound its work. It binds only IPv4 loopback.

The source contains conspicuous synthetic credentials solely for this fixture. Standard MediaBrowser identity and Token authorization are required. Legacy header and token-query authentication are refused. The supplied-stem manifest binds item, revision, SHA256 and byte sizes. Sports settings use If-Match CAS and live only in this process. Restart resets the synthetic account. All sports names, scores and schedules are fictional; playback_verified stays false and the provider list is empty.

`cargo test` exercises real loopback sockets for auth refusals, unowned item refusal, CAS save/read/stale conflict, HEAD, partial GET and invalid ranges. These are fixture checks. They do not exercise RustBridge, AVFoundation or native UI.

Connected native playback and UI require separate simulator or device tests. Keep machine addresses, credentials, run logs and captures in private evidence outside the checkout.
