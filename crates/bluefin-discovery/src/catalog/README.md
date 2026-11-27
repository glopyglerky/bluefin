# Public catalog metadata composition

The existing discovery-owned `CatalogService` serves anonymous public metadata at
`GET /Bluefin/Catalog/Search?kind=movie&query=The%20Matrix&page=1` and
`GET /Bluefin/Catalog/Details/movie/603`. Its regional access endpoints continue
to use the single `bluefin-catalog::OfferResponse` contract. A library account,
paired server library and playback entitlement are not prerequisites for public
metadata.

## Sources and identities

Movies, series and people use authorized TMDB credentials when configured.
If those are absent or that acquisition fails, the same protected catalog
configuration can use an operator's existing Seerr service through its supported
read-only API. The service's own `X-Api-Key` remains in the server process.
No embedded upstream credentials are extracted or distributed.

Live API shape checks confirmed Seerr's `/search`, `/movie/{id}`,
`/tv/{id}`, `/person/{id}` and `/person/{id}/combined_credits` routes.
Movie, TV and person identities remain distinct. Camel-case fields are projected
explicitly into the public wire record; private `mediaInfo`, requests,
watchlists, users and service addresses cannot enter that record.
The search endpoint returns mixed movie/TV/person pages. After filtering by the
requested type, pagination follows that original page and its total remains
unknown; filtered row counts are never claimed as catalog-wide totals.
Percent-encoded spaces are used because a live supported endpoint rejected
form-style `+` spaces.

Music search and details use the same `MusicBrainzClient` as access relations,
sharing its process-wide rate gate, retry state, bounded cache and freshness.
Artists, release groups, releases, recordings and works retain typed MusicBrainz
UUIDs. Related albums, actual editions, performers and track recordings are
clickable `credits` with their own identities. Source-supplied truncation and
the bounded response cap remain explicit. Artwork is emitted only from actual
TMDB paths or a release's positive Cover Art Archive front-cover flag.

## Source state

Metadata responses carry source observations, acquisition time and attribution.
Missing credentials, denial, rate limits, unavailable services, malformed data
and true identity mismatches are distinct from a successful empty search.
A person record can remain usable when its separate credits request fails, with
both partial metadata coverage and the credits failure retained. Stale music
metadata keeps its older observation time and failure status; it is not
relabeled fresh.

Source documents and schemas:
- [Seerr supported search route](https://github.com/seerr-team/seerr/blob/develop/server/routes/search.ts)
- [Seerr supported person routes](https://github.com/seerr-team/seerr/blob/develop/server/routes/person.ts)
- [TMDB API](https://developer.themoviedb.org/reference/intro/getting-started)
- [MusicBrainz API](https://musicbrainz.org/doc/MusicBrainz_API)

The metadata wire design follows the existing specialist catalog work, composed
into the shared discovery service rather than introducing a second public
runtime or a parallel offers model.

## Qualification

The source change includes pure projection tests and an HTTP integration test
covering anonymous search/details, configured fallback, source pagination,
artwork, credits, private-field removal and explicit failure semantics.
Run only through the repository's current admitted Forge build owner:

```sh
cargo test -p bluefin-discovery catalog
cargo test -p ferrofin-api --test bluefin_catalog
cargo test -p bluefin-catalog --features http
cargo check -p bluefin-catalog --no-default-features
```

These commands are qualification requirements, not evidence of a completed run.
