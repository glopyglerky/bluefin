# Server-owned discovery and requests

Seerr integration belongs only to the Bluefin server and web administration
application. Native Apple, Android, desktop and TV applications must not contain
a Seerr SDK, integration module, API credential, upstream URL configuration,
user mapping or direct request to a Seerr service. Compiling a web administration
bundle for WASM does not make that bundle eligible for a native client package.

Server administrators may opt in to exposing discovery/request actions to their
clients. The default is off. Clients receive only generic Bluefin search results
and server-authorized actions through the Bluefin protocol. They must not know
which optional service fulfills an action. The server owns validation, viewer
permissions, provider identity mapping, upstream communication and error handling.
A server-hosted request page can provide richer behavior without packaging the
integration into an app.

The client option belongs to server administration. Changing it must not require
a special client build, a provider login in a client, or downloading executable
integration code. When disabled, the server omits discovery/request actions from
its generic capability response and rejects their API calls, including stale
actions held by an already-open app. Missing support and old-server 404 responses
also mean unavailable. Library and guide search remain usable in every case.

When enabled, the server must still check the current viewer, explicit request
permission and upstream account mapping on every search and submission. A result
ID or action handle is opaque to clients and grants no authority by itself.
Recheck authorization when a request is submitted. Do not forward upstream
URLs, keys, account IDs, request bodies or raw upstream errors to clients.
Administrators manage the connector in the web UI; native clients have no
connector settings or provider-specific request workflow.

The web administration UI manages provider connections through authenticated
server APIs. Upstream credentials remain server-side; they are not returned to
the browser, native client or search response. Never accept caller-selected
upstream user IDs as authority. A service API key must not silently elevate an
ordinary viewer's request permissions.

Library search, live channel/programme search and native provider-app handoff
are independent features. They continue to work without Seerr. Optional discovery
can add licensed catalog results; finding a title does not make it playable.
Unknown provider access remains unknown. Only deliberate authorized actions may
be forwarded, and requests must not trigger automatic behavior on installation.

Bluefin does not include the arr stack. A separately installed Seerr service
retains its own integrations, policy, permissions and terms. An administrator's
opt-in and a user's request do not establish copyright ownership or provider
access. Apply the [authorized-use notice](authorized-use.md) and server access
controls at every boundary.

This is an implementation boundary and acceptance requirement. It does not
assert that a Seerr connector or generic discovery endpoint is already shipped.
Qualify the server adapter, administrator controls, viewer denial, account
isolation, default-off behavior and native package dependency graphs before
claiming the integration complete.

The [native client boundary check](../../gates/client-boundary/README.md) enforces
the current Apple dependency allowlist before Rust artifact builds and in CI.
The server option and adapter still require implementation and runtime acceptance;
passing the packaging check does not claim they are shipped.
