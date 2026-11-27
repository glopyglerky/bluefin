# Bluefin catalog access

This crate shares typed catalog identities, country-aware access evidence and source projections across Bluefin hosts and clients. Its default build uses Rust core and alloc. The optional http feature contains credential-owning native HTTP adapters; the portable model and projections do not depend on a server framework or runtime.

## Request and response contract

A LookupRequest contains an identity with a media kind and source-owned identifiers, plus an optional two-letter country. For example:

    {
      "identity": {
        "kind": "movie",
        "ids": [
          { "source": "tmdb", "id": "movie/238" },
          { "source": "imdb", "id": "tt0068646" }
        ]
      },
      "country": "US"
    }

Country codes normalize to uppercase. A missing or null country requests every country reported by the source; there is no implicit US storefront. A source registry establishes actual supported countries. Numeric TMDB IDs also work when the explicit media kind supplies the movie/TV namespace. MusicBrainz identifiers remain typed by entity: an artist, release group, release, recording and work are distinct identities.

All serialized field names and enum variants use camelCase. Each offer carries its own country, service identity, access type, optional add-on, exact destinations, price, quality and source evidence. The caller's requested country remains in the response even when a linked music destination has no established territory. Unix timestamps are UTC seconds. Observation time is acquisition time, not an assertion that an upstream URL will remain playable.

Source service IDs remain opaque strings. New providers and public domains are accepted without a provider-name allowlist. Source identities use distinct namespaces; providers are not conflated because their display names happen to match. Stable offer IDs incorporate identity, country, service, access mode, add-on, quality and destinations, while excluding volatile prices and observation times.

The shared response distinguishes:

- Complete or partial source evidence.
- A source reporting no offers for the request.
- Missing credentials, denial, rate limiting, outage, not found, malformed data and an identity mismatch.
- Source-catalog coverage from incomplete coverage and unknown coverage.
- Regional availability from a curated music link whose country is unknown.

NoOffers applies only to the named source and scope. Neither an empty list nor a failed upstream establishes worldwide unavailability. Catalog access does not establish the user's subscription, purchase, playback rights or access to a private library.

## Projection entry points

- video::project_tmdb_watch_providers projects the documented movie/TV country response. It preserves subscription, rent, buy, free, ads and future returned categories. The source's aggregate watch page is a catalogUrl, never a provider playback destination.
- video::project_movie_of_the_night projects exact v4 show identity, service IDs, regions, destinations, prices, quality, add-ons and availability dates. Expired or future access is not reported as currently available.
- video::project_tmdb_registry and video::project_movie_of_the_night_registry preserve source-reported countries and services. Registry membership alone does not imply an access mode for a title.
- tmdb_links::project_tmdb_watch_page accepts the final page URL and visible public page HTML. The host controls acquisition, country selection and rate limits. Its fixture documentation records the observed HTML contract and its limits.
- musicbrainz::project_musicbrainz_relations maps active typed relationships to their actual destinations. MusicBrainz links retain unknown country unless separate evidence establishes a territory.
- musicbrainz::project_itunes_music_lookup supplements exact cross-referenced music IDs only for an explicitly selected, returned storefront.

A reported offer may have no usable destination. Render its provider availability without a Watch or Play action. An exact provider title/player link can be presented as an outbound action; an aggregate catalog page, service homepage or search page cannot substitute for it.

## Hosted adapters

With http enabled:

    let client = bluefin_catalog::CatalogClient::new(credentials)?;
    let offers = client.lookup(&request).await;
    let registry = client.registry(request.country.as_ref()).await;

Credentials supports an optional TMDB API key or bearer token, and an optional Movie of the Night direct or RapidAPI key. Callers supply their own authorized configuration. The credentials are not serializable, their Debug representation is redacted, and no credential value appears in an offer response. No embedded or borrowed provider keys are included.

The direct clients use canonical upstream origins, sensitive authentication headers, bounded response bodies and timeouts. They do not follow redirects or use an ambient proxy. Transport failures become structured source statuses without returning raw URLs or upstream bodies. The v4 adapter requests show-level granularity for movie/series lookups, avoiding unused season/episode payloads.

The hosted MusicBrainz client owns its public-service throttling, bounded response cache and exact storefront enrichment. The application owns aggregate caching and can compose a configured read-only Seerr adapter with public page projections. Public clients can consume the same serialized response before library pairing; credentialed network access stays on the host.

The initial direct video adapter supports movie and series access. Season and episode identities are represented by the shared model but return UnsupportedIdentity until an exact episode/season adapter is supplied. Sports and person identities likewise require their own source adapter; they are not relabeled as films or empty video catalogs.

## Upstream contracts and provenance

These modules implement public response contracts and relationship semantics; they do not vendor an upstream application or reuse its credentials.

- [TMDB movie watch providers](https://developer.themoviedb.org/reference/movie-watch-providers), [TV watch providers](https://developer.themoviedb.org/reference/tv-series-watch-providers), [regions](https://developer.themoviedb.org/reference/watch-providers-available-regions). TMDB availability carries TMDB and JustWatch attribution and does not itself supply full provider deep links.
- [Movie of the Night show contract](https://docs.movieofthenight.com/resource/shows), [country/service contract](https://docs.movieofthenight.com/resource/countries). The reviewed [OpenAPI revision](https://github.com/movieofthenight/streaming-availability-api/blob/5e9299d0d57ca915d54da4aa15fd99af771285a6/openapi.yaml) is 5e9299d0d57ca915d54da4aa15fd99af771285a6.
- [MusicBrainz artist relationships](https://musicbrainz.org/relationships/artist-url), [release relationships](https://musicbrainz.org/relationships/release-url), [recording relationships](https://musicbrainz.org/relationships/recording-url), [web service](https://musicbrainz.org/doc/MusicBrainz_API).
- [Apple iTunes Search API](https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/iTuneSearchAPI/).

Source attribution metadata travels with responses so each client can preserve it. The crate inherits this repository's package license and is not independently published.

## Verification

Run these commands through the repository's admitted build/test lane:

    cargo test -p bluefin-catalog --features http
    cargo check -p bluefin-catalog --no-default-features --target wasm32-unknown-unknown

Boundary tests cover country isolation and global requests, exact identity mismatch, unknown providers/access modes, malformed sources, missing credentials, unsafe destinations, price and quality retention, expiration and stable evidence identity. Public-page fixtures test observed links as well as denied, malformed and wrong-country responses. Live source acceptance is separate from fixture tests and must record the acquired source and observed region.
