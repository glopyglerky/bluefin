package org.bluefin.android

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class CatalogAccessTest {
    private val video = CatalogIdentity("movie", listOf(CatalogIdentifier("tmdb", "movie/10331")))
    private val artist = CatalogIdentity("artist", listOf(CatalogIdentifier("musicbrainz", "5b11f4ce-a62d-471e-81fc-a69a8278c7da")))
    private val recording = CatalogIdentity("recording", listOf(CatalogIdentifier("musicbrainz", "00000000-1111-2222-3333-444444444444")))

    private fun source(status: String = "partial") = JSONObject()
        .put("source", "tmdb-watch-page").put("status", status).put("coverage", "partial")
        .put("scope", "regionalAvailability").put("observedAt", 1795760000L)
        .put("attributions", JSONArray().put(JSONObject().put("name", "JustWatch").put("url", "https://www.justwatch.com")))

    private fun offer(id: String = "offer-1", country: String? = "US", identity: CatalogIdentity = video): JSONObject =
        JSONObject().put("id", id).put("identity", identity.json()).put("country", country ?: JSONObject.NULL)
            .put("service", JSONObject().put("source", "tmdb").put("id", "future-4001").put("name", "Small Future Service"))
            .put("accessType", "rent").put("price", JSONObject().put("amount", "2.99").put("currency", "USD"))
            .put("quality", "HD").put("destinations", JSONArray().put(JSONObject().put("kind", "title")
                .put("url", "https://future-stream.example/title/night?id=42&utm_source=justwatch")))
            .put("evidence", JSONObject().put("source", "tmdb-watch-page").put("observedAt", 1795760000L)
                .put("regionVerified", country != null).put("expiresSoon", false))

    private fun response(country: String? = "US", offers: JSONArray = JSONArray().put(offer()),
                         identity: CatalogIdentity = video, status: String = "partial") =
        JSONObject().put("schemaVersion", 1).put("identity", identity.json())
            .put("country", country ?: JSONObject.NULL).put("offers", offers).put("sources", JSONArray().put(source(status)))

    private fun parsed(country: String, status: String = "partial") = CatalogOfferResponse.parse(
        response(country, JSONArray().put(offer("offer-" + country, country)), status = status), video, country)

    @Test fun actualSharedOfferFieldsRetainUnknownServicesQueriesPricesAndSeconds() {
        val result = CatalogOfferResponse.parse(response(), video, "US")
        val item = result.offers.single()
        assertEquals("Small Future Service", item.service.name)
        assertEquals("tmdb:future-4001", item.service.key)
        assertEquals("2.99 USD", item.price?.display)
        assertEquals("HD", item.quality)
        assertEquals(1795760000L, item.evidence.observedAt)
        assertTrue(item.destinations.single().url.endsWith("?id=42&utm_source=justwatch"))
        assertEquals("https://www.justwatch.com", result.sources.single().attributions.single().url)
    }

    @Test fun priceCurrencyAloneNeverBecomesZeroOrFree() {
        val price = CatalogPrice.parse(JSONObject().put("currency", "USD").put("amount", JSONObject.NULL))
        assertNull(price?.display)
        assertNull(CatalogPrice.parse(JSONObject().put("amount", "NaN"))?.display)
        assertEquals("0 USD", CatalogPrice.parse(JSONObject().put("amount", "0").put("currency", "USD"))?.display)
    }

    @Test fun exactTitleDestinationsRejectSearchHomeIntermediariesAndPrivateUrls() {
        val rejected = listOf(
            "https://www.youtube.com/results?search_query=Night+of+the+Living+Dead",
            "https://future-stream.example/search/night", "https://future-stream.example/",
            "https://future-stream.example/us", "https://future-stream.example/movies",
            "https://www.justwatch.com/us/movie/night-of-the-living-dead",
            "https://www.themoviedb.org/movie/10331/watch?locale=US",
            "https://tv.apple.com/us/movie/example/umc.1", "https://127.0.0.1/title/42",
            "https://10.0.0.1/title/42", "https://[::1]/title/42", "https://user:secret@example.com/title/42",
            "http://example.com/title/42", "file:///title/42"
        )
        rejected.forEach { assertFalse(it, safeCatalogDestinationUrl(it, "Tubi")) }
        assertTrue(safeCatalogDestinationUrl("https://tv.apple.com/us/movie/example/umc.1", "Apple TV"))
        assertTrue(safeCatalogDestinationUrl("https://www.youtube.com/watch?v=real-title-id", "YouTube"))
        assertTrue(safeCatalogDestinationUrl("https://fawesome.tv/movies/10862175/night?utm_source=justwatch", "Fawesome"))
    }

    @Test fun unusableDestinationKeepsAvailabilityWithoutAnAction() {
        val row = offer().put("destinations", JSONArray().put(JSONObject().put("kind", "title")
            .put("url", "https://future-stream.example/search?q=night")))
        val item = CatalogOfferResponse.parse(response(offers = JSONArray().put(row)), video, "US").offers.single()
        assertEquals("Small Future Service", item.service.name)
        assertTrue(item.destinations.isEmpty())
        assertNull(item.bestDestination(1795760000L))
    }

    @Test fun responseIdentityAndCountryCannotBeRelabeledByUi() {
        assertThrows(IllegalArgumentException::class.java) { CatalogOfferResponse.parse(response(), artist, "US") }
        assertThrows(IllegalArgumentException::class.java) { CatalogOfferResponse.parse(response(), video, "GB") }
        assertThrows(IllegalArgumentException::class.java) {
            CatalogOfferResponse.parse(response(country = null).put("country", "USA"), video, null)
        }
        val mixed = JSONArray().put(offer("good")).put(offer("wrong-country", "GB"))
            .put(offer("wrong-title", identity = video.copy(ids = listOf(CatalogIdentifier("tmdb", "movie/278")))))
        val result = CatalogOfferResponse.parse(response(offers = mixed), video, "US")
        assertEquals(listOf("good"), result.offers.map { it.id })
        assertEquals(2, result.excluded)
    }

    @Test fun linkedMusicKeepsUnknownRegionExplicitAndRetainsTrackIdentity() {
        val row = offer("music", null, artist).put("accessType", "download")
        val result = CatalogOfferResponse.parse(response("GB", JSONArray().put(row), artist), artist, "GB")
        assertNull(result.offers.single().country)
        assertFalse(result.offers.single().evidence.regionVerified)
        val track = JSONObject().put("identity", recording.json()).put("title", "One song")
            .put("sourceUrl", "https://musicbrainz.org/recording/00000000-1111-2222-3333-444444444444").put("number", "2-03")
        val person = JSONObject().put("identity", artist.json()).put("title", "An artist")
            .put("sourceUrl", "https://musicbrainz.org/artist/5b11f4ce-a62d-471e-81fc-a69a8278c7da")
        val detail = CatalogMusicDetail.parse(JSONObject().put("item", person).put("tracks", JSONArray().put(track))
            .put("facts", JSONArray()).put("related", JSONArray()).put("sources", JSONArray()), artist)
        assertEquals(recording, detail.tracks.single().identity)
        assertEquals("2-03", detail.tracks.single().number)
        assertNull(CatalogMusicDetail.parse(JSONObject().put("item", JSONObject.NULL), artist).item)
    }

    @Test fun editionOffersRequireAnExplicitVerifiedParentAndRetainTheirReleaseId() {
        val group = CatalogIdentity("releaseGroup", listOf(CatalogIdentifier("musicbrainz", "11111111-1111-2222-3333-444444444444")))
        val release = CatalogIdentity("release", listOf(
            CatalogIdentifier("musicbrainz", "22222222-1111-2222-3333-444444444444"),
            CatalogIdentifier("musicbrainz-release-group", group.ids.single().id)))
        val wrong = release.copy(ids = release.ids.map {
            if (it.source == "musicbrainz-release-group") it.copy(id = "33333333-1111-2222-3333-444444444444") else it
        })
        val rows = JSONArray().put(offer("exact-edition", identity = release)).put(offer("wrong-edition", identity = wrong))
            .put(offer("unlinked-edition", identity = release.copy(ids = release.ids.take(1))))
        val parsed = CatalogOfferResponse.parse(response(offers = rows, identity = group), group, "US")
        assertEquals(listOf("exact-edition"), parsed.offers.map { it.id })
        assertEquals(release, parsed.offers.single().identity)
    }

    @Test fun onlyLinkedArtistOffersCanOpenAnExactArtistHostRoot() {
        val artistOffer = offer("artist-root", null, artist).put("destinations", JSONArray().put(JSONObject()
            .put("kind", "title").put("url", "https://artist-fixture.bandcamp.com/")))
        artistOffer.getJSONObject("evidence").put("source", "musicbrainz")
        assertEquals("https://artist-fixture.bandcamp.com/", CatalogOffer.parse(artistOffer)!!.destinations.single().url)
        assertTrue(CatalogOffer.parse(artistOffer)!!.artistLinkedAccess)
        val movieOffer = offer("movie-root").put("destinations", artistOffer.getJSONArray("destinations"))
        assertTrue(CatalogOffer.parse(movieOffer)!!.destinations.isEmpty())
        assertFalse(safeCatalogDestinationUrl("https://artist-fixture.bandcamp.com/?q=unrelated", "Bandcamp", true))
    }

    @Test fun expiredAndFutureOffersHaveNoCurrentPlaybackAction() {
        val item = CatalogOffer.parse(offer())!!
        assertNotNull(item.bestDestination(200))
        assertNull(item.copy(evidence = item.evidence.copy(expiresAt = 100)).bestDestination(200))
        assertNull(item.copy(evidence = item.evidence.copy(availableSince = 300)).bestDestination(200))
    }

    @Test fun countriesAndServicesComeOnlyFromActualRegistryRows() {
        val value = JSONObject().put("schemaVersion", 1).put("countries", JSONArray()
            .put(JSONObject().put("code", "GB").put("name", "United Kingdom")
                .put("services", JSONArray().put(JSONObject().put("service", JSONObject()
                    .put("source", "tmdb").put("id", "new-id").put("name", "New Source Service")))))
            .put(JSONObject().put("code", "USA").put("name", "Invalid")))
            .put("sources", JSONArray())
        val registry = CatalogRegistry.parse(value)
        assertEquals(listOf("GB"), registry.countries.map { it.code })
        assertEquals("New Source Service", registry.countries.single().services.single().name)
    }

    @Test fun coverArtRedirectsAreHttpsBoundedToTheSuppliedArchiveHosts() {
        val original = "https://coverartarchive.org/release-group/00000000-1111-2222-3333-444444444444/front-250"
        assertTrue(isCatalogMusicArtwork(original))
        assertTrue(safeCatalogArtworkRedirect(original, "https://ia801234.us.archive.org/image/item/cover.jpg"))
        assertFalse(safeCatalogArtworkRedirect(original, "http://archive.org/image/cover.jpg"))
        assertFalse(safeCatalogArtworkRedirect(original, "https://archive.org.attacker.example/image.jpg"))
        assertFalse(safeCatalogArtworkRedirect("https://upload.wikimedia.org/wikipedia/commons/x.jpg", "https://archive.org/x.jpg"))
    }

    @Test fun allRegionsChecksAtMostThreeAtOnceAndUpdatesIncrementally() = runBlocking {
        val countries = listOf("US", "GB", "CA", "AU", "DE", "FR", "JP").map { CatalogCountry(it, it, emptyList()) }
        var active = 0
        var maximum = 0
        val updates = mutableListOf<Int>()
        val result = loadAllCatalogCountries(video, countries, null, lookup = { code ->
            active++; maximum = maxOf(maximum, active)
            delay(if (code == "US") 8 else 2)
            active--
            parsed(code)
        }, update = { updates += it.completed })
        assertEquals(3, maximum)
        assertEquals(countries.map { it.code }.toSet(), result.offers.mapNotNull { it.country }.toSet())
        assertEquals(7, result.completed)
        assertFalse(result.running)
        assertTrue(updates.containsAll((1..7).toList()))
    }

    @Test fun sourceRateLimitStopsSchedulingAdditionalRegionsWithoutDiscardingResults() = runBlocking {
        val countries = listOf("US", "GB", "CA", "AU", "DE").map { CatalogCountry(it, it, emptyList()) }
        val requested = mutableListOf<String>()
        val result = loadAllCatalogCountries(video, countries, null, lookup = { code ->
            requested += code
            if (code == "GB") parsed(code, "rateLimited") else parsed(code)
        }, update = {})
        assertEquals(setOf("US", "GB", "CA"), requested.toSet())
        assertEquals(3, result.completed)
        assertNotNull(result.stoppedReason)
        assertEquals(setOf("US", "CA"), result.offers.mapNotNull { it.country }.toSet())
        assertTrue(result.sources.any { it.status == "rateLimited" })
    }

    @Test fun changingRegionOrDismissingCancelsTheRemainingQueue() = runBlocking {
        val started = CompletableDeferred<Unit>()
        val hold = CompletableDeferred<Unit>()
        val requested = mutableListOf<String>()
        val countries = listOf("US", "GB", "CA", "AU", "DE").map { CatalogCountry(it, it, emptyList()) }
        val job = launch {
            loadAllCatalogCountries(video, countries, null, lookup = { code ->
                requested += code
                if (requested.size == 3) started.complete(Unit)
                hold.await()
                parsed(code)
            }, update = {})
        }
        started.await()
        job.cancelAndJoin()
        assertEquals(3, requested.size)
    }

    @Test fun publicOperationsRunBeforePairingWithoutAccountCredentials() = runBlocking {
        val store = object : CleanupStore {
            override fun load() = CleanupState()
            override fun begin() = Unit
            override fun write(state: CleanupState) = Unit
            override fun end() = Unit
        }
        val requests = mutableListOf<String>()
        val core = CoreClient(store) { op, fields, owner ->
            assertNull(owner)
            assertFalse(fields.has("server"))
            assertFalse(fields.has("token"))
            requests += op
            when (op) {
                "publicCatalogOffers" -> {
                    assertEquals("US", fields.getString("country"))
                    assertEquals(video, CatalogIdentity.parse(fields.getJSONObject("identity")))
                    response()
                }
                "publicCatalogCountries" -> JSONObject().put("schemaVersion", 1).put("countries", JSONArray()).put("sources", JSONArray())
                "publicMusicSearch" -> {
                    assertEquals("releaseGroup", fields.getString("kind"))
                    assertEquals(20L, fields.getLong("offset"))
                    JSONObject().put("results", JSONArray()).put("total", 0).put("offset", 20).put("sources", JSONArray())
                }
                "publicMusicDetail" -> JSONObject().put("item", JSONObject.NULL).put("sources", JSONArray())
                else -> error("Unexpected operation")
            }
        }
        core.publicCatalogOffers(video, "US")
        core.publicCatalogCountries(video)
        core.publicMusicSearch("releaseGroup", "Kind of Blue", 20)
        core.publicMusicDetail(artist)
        assertEquals(listOf("publicCatalogOffers", "publicCatalogCountries", "publicMusicSearch", "publicMusicDetail"), requests)
        assertNull(core.account)
    }
}
