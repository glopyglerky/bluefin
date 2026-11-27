# Private relay admission integration

This component implements the admission decision for Iroh 1.4.1's HTTP
access hook. It does not deploy a relay or complete remote pairing on its own.
Account authorization remains the existing one-use invitation, pinned hybrid
PQ TLS, ML-DSA device possession and passkey transaction.

The registry is bound to one independently configured canonical HTTPS relay.
It stores at most 1024 endpoint leases, at most two live guest endpoints per
invitation, and expires leases after the earlier of two minutes or invitation
expiry. Live retry does not extend a guest lease. Expiration uses a local
monotonic clock. Restarts fail closed because leases are not persisted.

## Account and listener owner seam

1. Declare the independent API module `bluefin_mesh_relay` in the handler
   module. Construct `RelayCallout::new(relay_url, token)` once per live account
   epoch. The bearer must be a dedicated CSPRNG-generated 32-byte value encoded
   as 64 lowercase hex characters. Only its SHA-256 digest is retained here.
2. Mount `callout.clone().router()` on a private listener. Configure the pinned
   relay's `access.http.url` to that listener's `/relay/admission` path, and
   supply its bearer through `IROH_RELAY_HTTP_BEARER_TOKEN` at runtime. Do not
   log authorization headers or persist the bearer in a public configuration.
   Do not expose a generic endpoint-grant operation.
3. Before starting the native PQ listener, use
   `DeviceIdentity::listener_endpoint_id()` and the private trusted listener
   owner's `admit_listener`. This avoids waiting for an online relay before the
   listener identity is admitted. Renew only while that listener and the live
   account-service epoch remain authorized. Clear the registry at shutdown or
   epoch replacement; do not reuse it for another relay or issuer.
4. Add the bootstrap route within the existing account service owner. Under
   its existing gate, check current epoch, active policy, exact stored capsule
   hash, invitation state and expiration before calling `admit_pairing`.
   Capsule validation in this component is an additional field/expiry check;
   it is not the authoritative ledger lookup. The bootstrap route needs its
   existing bounded request size, rate limits and operation budget. It must
   return generic denials and must not log codes, capsules or secrets.
5. Native clients already expose `PqcEndpoint::endpoint_id()`; the browser
   client now does too. Create the endpoint once, submit that endpoint ID and
   the original invitation to the owner bootstrap route, then connect with
   the same endpoint. Iroh authenticates possession of this endpoint key
   before invoking the relay access hook. Endpoint admission grants transport
   access only. Never substitute the endpoint ID for the inner PQ device key.
6. Consume the original invitation only in the existing account transaction.
   Call `revoke_invitation` after consumption or explicit revocation. Keep
   existing pinned PQ checks and independently configured relay allowlists.
   Continued account-session relay admission needs a separate owner-authorized
   device/session lease; it must not reuse a spent invitation.

The API adapter returns exactly HTTP 200 and `true` only for an authenticated
callout and a live endpoint lease. Missing or duplicate headers, an inconsistent
deprecated endpoint header, malformed IDs, bad bearer and unknown endpoints
are denied. Client tokens are not needed and never appear in relay URLs.

## Qualification and limits

Six focused registry tests and one API handler test are supplied for the
integration owner. Run the registry tests with
`cargo test -p mesh-connect --no-default-features --test relay_admission`.
After declaring the API module, run its handler test and the existing account
and native/browser PQ regressions through the admitted qualification lane.
The new direct `subtle` dependency requires the integration owner's workspace
lockfile update before locked qualification. Compilation and runtime pairing
have not been qualified by this component's author.

The registry currently uses hosted Rust `std` collections and `Instant`; this
is not no_std acceptance. The upstream binary's HTTP access hook checks new
connections only. Expiry and revocation here do not disconnect existing relay
connections or impose bandwidth limits. Relay resource limits and disconnect
policy require separate runtime qualification; do not claim a hard two-minute
connection lifetime from this module. Bootstrap, listener renewal, epoch
invalidation, consumed-code refusal, reconnect, outage, and real remote browser
and native pairing still require integration evidence.

## Inspected upstream

Source package: `iroh-relay` 1.4.1, VCS revision
`d6c6963b9602d1b8dd2ba53b6519e7eb19cc7940`, MIT OR Apache-2.0.
Inspected on 2026-11-27 from the pinned Cargo source:

- [HTTP callout and exact response contract](https://github.com/n0-computer/iroh/blob/d6c6963b9602d1b8dd2ba53b6519e7eb19cc7940/iroh-relay/src/main.rs):
  `HttpAccess`, `http_access_check_inner`, and HTTP bearer environment override.
  The callout forwards the authenticated endpoint ID, not the client's token.
- [Authenticated request and lifecycle hook](https://github.com/n0-computer/iroh/blob/d6c6963b9602d1b8dd2ba53b6519e7eb19cc7940/iroh-relay/src/server.rs):
  `ClientRequest::endpoint_id`, `auth_token`, and `AccessControl`.
- [Handshake ordering](https://github.com/n0-computer/iroh/blob/d6c6963b9602d1b8dd2ba53b6519e7eb19cc7940/iroh-relay/src/server/http_server.rs):
  authenticated handshake precedes `authorize_with`.

This is an original adapter; no upstream source was copied. This bounded study
does not claim coverage of relay Git history, all CI, academic literature, or
an independently evaluated remote relay session.
