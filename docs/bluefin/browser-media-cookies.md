# Browser media without a service worker

The ordinary HTTPS browser client uses the existing Rust source-pinned playback planner and policy checks. A mesh session never enters this route as an HTTP bearer. The API is registered only when the operator has configured a valid HTTPS relying-party origin and the server has a source pin factory.

`GET /Bluefin/Web/v1` advertises `version: 1` and `cookieMediaLease: true`. `POST /Bluefin/Web/v1/leases` requires the normal account Authorization header and the exact configured Origin. Its body is the existing versioned playback request: a selected item identifier, audio or video kind, bounded bitrate, start position, and optional stream indexes. The server chooses the source and a conservative H.264/AAC HLS plan; clients do not supply upstream paths or credentials.

The reply contains `mediaUrl`, `releaseUrl`, `renewUrl`, `playSessionId`, `expiresInSeconds: 300`, and `idleSeconds: 90`. URLs contain a public random handle. The independent media secret is sent only in a Secure, HttpOnly, SameSite=Strict cookie scoped to that lease media path. Copying a URL is insufficient. No domain-wide cookie or control API authority is created.

Media GET and HEAD support the existing range behavior. Validated HLS child references stay under the same cookie path. Cross-origin browser use is refused, and responses are private, uncached, and carry no referrer. Each request and each continuing response rechecks the original account session, current policy, selected source and item access. Revocation therefore takes effect on an existing body as well as the next request.

POST to `renewUrl` refreshes the cookie for at most five minutes; DELETE to `releaseUrl` stops the exact owned playback work and clears its cookie. Both require the original parent account credential and configured Origin. The absolute lifetime is two hours. Clients stop media locally before fallible remote cleanup and retire the lease on stop, sign-out, or page exit. Storage donation, stored chunks, discovery, and public handles grant no playback rights.

The implementation reuses the TV lease engine without changing the native TV wire response. Qualification must include its original lease tests and the browser origin, cookie separation, parent binding, expiry, HLS and revocation tests.
