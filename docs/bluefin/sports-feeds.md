# Sports feeds and artwork

Bluefin exposes authenticated scoreboards, event details, team statistics, play-by-play and standings from ESPN's public primary feeds. Account favorites remain in the existing Sports preferences record; the existing compare-and-swap version and If-Match rules still apply. A follow selector such as espn:nhl:129 distinguishes league-specific team IDs. It grants no playback, recording or subscription rights.

## Routes and data

All routes require an ordinary authenticated account. API keys are refused. Reading public sports information does not require Live TV permission; the existing local broadcast route still does.

| Route | Result |
| --- | --- |
| GET /sports/api/v1/scoreboard?league=nhl&date=YYYY-MM-DD | events array for the requested calendar day |
| GET /sports/api/v1/events/{league}/{id} | one event with statistics and plays when supplied |
| GET /sports/api/v1/standings/{league} | provider groups and standings |
| GET /sports/api/v1/logos/{id} | authenticated same-origin PNG team artwork |

Feed envelopes contain schema: 1, provider: "ESPN", fetchedAt (ISO8601), stale, league and events, event or groups. Event scores and play scores are strings. Missing optional values are null; missing standings stat keys stay absent. Rank comes from an explicit provider rank/seed, never the array index. Ordinary provider group names are retained instead of inventing divisions. Event dates are normalized to ISO8601 UTC with seconds.

The seven league keys are nhl, nfl, nba, mlb, wnba, epl and mls. Their fixed primary paths are hockey/nhl, football/nfl, basketball/nba, baseball/mlb, basketball/wnba, soccer/eng.1 and soccer/usa.1. Client-supplied URLs are never accepted. Calendar dates must be real YYYY-MM-DD dates in 1900–2199; event IDs are at most twenty ASCII digits.

NHL plays use the provider's plays array. NFL plays combine previous/current drives and scoring plays by actual play ID, so scoring summaries mark existing plays instead of duplicating them. Team statistics match the provider's exact team IDs; an absent opposing value remains null rather than zero. No predictions, fabricated examples or external private-reference URLs are added to production responses.

## Bounds, freshness and artwork

The provider client uses HTTPS, fixed ESPN origins, no redirects, a two-second connection timeout and a six-second request deadline. JSON bodies are limited to 4 MiB before parsing. Scoreboard/event caches refresh after thirty seconds; standings after five minutes. A provider failure may serve the preceding public snapshot for at most thirty minutes with stale: true and its original fetchedAt. Without a usable snapshot the route reports unavailability.

The public feed cache is bounded to 48 entries and 16 MiB. Per-key fetch gates coalesce simultaneous refreshes; there are at most four active JSON requests. No account identity, favorites, library data or media hashes are stored in this cache. Feed HTTP responses are no-store.

Only HTTPS PNG URLs under a.espncdn.com/i/teamlogos/ supplied by a normalized primary feed enter the artwork registry. The client receives an opaque same-origin logo path, not the upstream URL. The registry is bounded to 1,024 entries and 1 MiB; the artwork byte cache to 256 entries and 16 MiB. Six queued artwork fetches may run at once independently of scoreboard requests. Each image is capped at 1 MiB, must declare PNG, and must have a PNG header with dimensions no larger than 2,048×2,048. Cached artwork still passes normal account authentication before serving. Unknown, unregistered IDs never trigger a network request.

## Qualification and compatibility

This is a Bluefin-only extension. Jellyfin routes, the local broadcast authority boundary and Sports preference CAS behavior are unchanged.

Focused tests cover fixed-origin/date/event input validation, same-origin logo admission, bounded caches, missing values, exact team matching, NFL play deduplication, genuine standings groups/ranks, API-key rejection and ordinary-account access without Live TV entitlement. An opt-in qualification starts the real authenticated Axum router on a loopback HTTP listener and requests all seven leagues, NHL/NFL event details and a real proxied team logo from the fixed primary provider. It uses disposable local accounts and no production media.

Primary sources used for feed schema qualification:

- https://site.api.espn.com/apis/site/v2/sports/hockey/nhl/scoreboard?dates=20261125
- https://site.api.espn.com/apis/site/v2/sports/hockey/nhl/summary?event=401892781
- https://site.api.espn.com/apis/site/v2/sports/football/nfl/summary?event=401873081
- https://site.api.espn.com/apis/v2/sports/hockey/nhl/standings
