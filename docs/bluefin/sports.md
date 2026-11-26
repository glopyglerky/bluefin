# Standalone Sports API

Sports is an account presentation surface on the existing authenticated server.
Paths below are relative to the configured server base URL. There is no extra
root alias, service, account store, provider connector or authorization authority.
Standard viewer sessions are required; API keys, nil users and user overrides
are refused. Normal remote and schedule policies still apply. Responses are
private (`Cache-Control: no-store`).

`GET sports/api/v1/preferences` returns `{ "version": 0, "preferences": ... }`
for a fresh viewer, with ETag `"0"`. `PUT` accepts the preferences object alone
and requires one canonical quoted unsigned decimal `If-Match`. Missing tags
return 428, malformed/weak/wildcard/multiple tags 400, and stale tags 412.
A successful atomic SQLite compare-and-replace increments the version and
returns the saved object with its matching ETag. Revision exhaustion and corrupt
or failed storage return errors without resetting or overwriting the record.
Accepted writes retain ownership through commit if HTTP is cancelled.

Schema 1 requires all fields: `sports`, `leagues`, `conferences`, `teams`,
`players` (string arrays), `all_conferences`, `all_teams`, `all_players`, `home`,
`away`, `native_home`, `auto_record`, `sports_only`, `completed` (booleans).
Unknown fields are refused. Each selection array has at most 256 entries, each
opaque nonempty ID at most 128 UTF-8 bytes. Raw request and canonical preferences
are each limited to 60,000 bytes. At least one of home/away must be true. Sets
serialize in stable order. Defaults are empty selectors, all-selection flags and
home/away/native_home true; auto_record/sports_only/completed false. Saving
`auto_record` is inert and creates no recording or timer. Generic display
preferences cannot read or write the reserved Sports namespace.

`GET sports/api/v1/home` additionally requires existing LiveTV access. It returns
schema 1, the saved `sports_only`, `games: []`, `providers: []`,
`presentation_only: true` and authorized `local_broadcasts`. Empty guide data is
valid 200. Both programme and channel visibility are checked through Core for
the actual viewer on every request. Stream/adoption authorization remains at its
existing boundaries.

Coverage is finite and may be truncated: only the first 256 sports-classified
guide rows ordered by start date are considered, from 24 hours before the request
through seven days ahead. Ended rows and invisible rows are omitted after paging;
this does not refill the page or promise all events/channels. Listings lacking a
start date are outside this bounded query. Missing/uncertain end dates produce
UNKNOWN, which is non-actionable. Known current intervals are ON_NOW and future
intervals UPCOMING. A guide listing is never a verified LIVE game, and
`playback_verified` is always false. IDs, titles and dates come from the local
guide; scores, teams and event/provider identities are not synthesized.

Opaque favorites are retained but cannot be matched to verified events with the
available guide metadata. This feed therefore makes no personalized-event-match
claim. `sports_only` is presentation state and grants no access. A licensed event
feed and verified identity matching are separate future extensions. No external
provider fetch occurs in these endpoints.
