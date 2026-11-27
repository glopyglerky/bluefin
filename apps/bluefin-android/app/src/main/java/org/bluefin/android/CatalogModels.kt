package org.bluefin.android

import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.net.InetAddress
import java.util.Locale

private fun JSONObject.catalogText(key: String, limit: Int = 256): String? =
    (opt(key) as? String)?.trim()?.takeIf { it.isNotEmpty() && it.length <= limit && it.none(Char::isISOControl) }
private fun JSONObject.catalogSeconds(key: String): Long? =
    (opt(key) as? Number)?.toLong()?.takeIf { it >= 0 }
private fun JSONArray.objects(limit: Int = 20000): List<JSONObject> =
    (0 until minOf(length(), limit)).mapNotNull(::optJSONObject)

data class CatalogIdentifier(val source: String, val id: String)
data class CatalogIdentity(val kind: String, val ids: List<CatalogIdentifier>) {
    val key: String get() = kind + "|" + ids.sortedWith(compareBy({ it.source.lowercase(Locale.ROOT) }, { it.id }))
        .joinToString("|") { it.source.lowercase(Locale.ROOT) + ":" + it.id }
    val video: Boolean get() = kind in setOf("movie", "series", "season", "episode")
    fun json() = JSONObject().put("kind", kind).put("ids",
        JSONArray().apply { ids.forEach { put(JSONObject().put("source", it.source).put("id", it.id)) } })
    fun matches(other: CatalogIdentity): Boolean = key == other.key
    fun isOfferFor(expected: CatalogIdentity): Boolean {
        if (matches(expected)) return true
        if (kind != "release" || expected.kind != "releaseGroup") return false
        val requested = expected.ids.singleOrNull { it.source.equals("musicbrainz", ignoreCase = true) } ?: return false
        val release = ids.singleOrNull { it.source.equals("musicbrainz", ignoreCase = true) } ?: return false
        val parent = ids.singleOrNull { it.source.equals("musicbrainz-release-group", ignoreCase = true) } ?: return false
        return parent.id == requested.id && release.id.matches(Regex("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"))
    }

    companion object {
        fun parse(value: JSONObject?): CatalogIdentity? {
            value ?: return null
            val kind = value.catalogText("kind", 40)?.takeIf { it.matches(Regex("[a-z][A-Za-z]{1,39}")) } ?: return null
            val raw = value.optJSONArray("ids") ?: return null
            if (raw.length() !in 1..12) return null
            val ids = raw.objects(12).map { row ->
                val source = row.catalogText("source", 64)?.takeIf { it.matches(Regex("[A-Za-z0-9_-]+")) } ?: return null
                val id = row.catalogText("id", 256)?.takeIf { it.none(Char::isWhitespace) } ?: return null
                CatalogIdentifier(source, id)
            }
            if (ids.size != raw.length()) return null
            if (ids.groupBy { it.source.lowercase(Locale.ROOT) }.values.any { group -> group.map { it.id }.distinct().size > 1 }) return null
            return CatalogIdentity(kind, ids.distinct())
        }
    }
}

internal fun libraryCatalogIdentity(type: String, providers: JSONObject?): CatalogIdentity? {
    providers ?: return null
    fun provider(key: String): String? {
        val exact = providers.keys().asSequence().firstOrNull { it.equals(key, ignoreCase = true) } ?: return null
        return providers.opt(exact) as? String
    }
    fun music(kind: String, key: String): CatalogIdentity? {
        val raw = provider(key) ?: return null
        if (!raw.matches(Regex("[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}")) ||
            raw == "00000000-0000-0000-0000-000000000000") return null
        return CatalogIdentity(kind, listOf(CatalogIdentifier("musicbrainz", raw.lowercase(Locale.ROOT))))
    }
    return when (type) {
        "Movie", "Series" -> provider("Tmdb")?.takeIf { it.matches(Regex("[1-9][0-9]{0,11}")) }?.let {
            CatalogIdentity(if (type == "Movie") "movie" else "series", listOf(CatalogIdentifier("tmdb", it)))
        }
        "MusicArtist" -> music("artist", "MusicBrainzArtist")
        "MusicAlbum" -> music("releaseGroup", "MusicBrainzReleaseGroup") ?: music("release", "MusicBrainzAlbum")
        "Audio" -> music("recording", "MusicBrainzRecording")
        else -> null
    }
}

fun safeCatalogHttpsUrl(raw: String): Boolean = runCatching {
    if (raw.isEmpty() || raw.length > 8192 || raw.any { it.isWhitespace() || it.isISOControl() }) return@runCatching false
    val uri = URI(raw)
    val host = uri.host?.lowercase(Locale.ROOT)?.trimEnd('.') ?: return@runCatching false
    if (uri.scheme != "https" || uri.userInfo != null || uri.port !in listOf(-1, 443)) return@runCatching false
    if (host.contains(':')) {
        if (host.contains('%')) return@runCatching false
        val ip = InetAddress.getByName(host.removePrefix("[").removeSuffix("]"))
        val bytes = ip.address
        return@runCatching !ip.isAnyLocalAddress && !ip.isLoopbackAddress && !ip.isLinkLocalAddress &&
            !ip.isSiteLocalAddress && !ip.isMulticastAddress &&
            !(bytes.size == 16 && (bytes[0].toInt() and 0xfe) == 0xfc) &&
            !(bytes.size == 16 && bytes.take(4).map { it.toInt() and 255 } == listOf(0x20, 0x01, 0x0d, 0xb8))
    }
    if (!host.contains('.') || listOf("localhost", "local", "internal", "home", "lan", "invalid")
        .any { host == it || host.endsWith("." + it) }) return@runCatching false
    if (host.all { it.isDigit() || it == '.' }) {
        val parts = host.split('.').map { it.toIntOrNull() ?: return@runCatching false }
        if (parts.size != 4 || parts.any { it !in 0..255 }) return@runCatching false
        val a = parts[0]; val b = parts[1]; val c = parts[2]
        if (a == 0 || a == 10 || a == 127 || a >= 224 || a == 100 && b in 64..127 ||
            a == 169 && b == 254 || a == 172 && b in 16..31 || a == 192 && b == 168 ||
            a == 192 && b == 0 && c in listOf(0, 2) || a == 198 && b in 18..19 ||
            a == 198 && b == 51 && c == 100 || a == 203 && b == 0 && c == 113) return@runCatching false
    }
    true
}.getOrDefault(false)

/** Provider queries and opaque IDs are retained; generic searches are never title actions. */
fun safeCatalogDestinationUrl(raw: String, provider: String, allowArtistRoot: Boolean = false): Boolean = runCatching {
    if (!safeCatalogHttpsUrl(raw)) return@runCatching false
    val uri = URI(raw); val host = uri.host.lowercase(Locale.ROOT).trimEnd('.')
    if (listOf("justwatch.com", "themoviedb.org").any { host == it || host.endsWith("." + it) }) return@runCatching false
    if (host == "tv.apple.com" && !provider.contains("apple", ignoreCase = true)) return@runCatching false
    val parts = uri.path.orEmpty().split('/').filter(String::isNotBlank).map { it.lowercase(Locale.ROOT) }
    if (parts.any { it in setOf("search", "results", "login", "signin", "sign-in",
        "signup", "sign-up", "redirect", "click", "subscribe", "register") }) return@runCatching false
    val keys = uri.rawQuery.orEmpty().split('&').map { java.net.URLDecoder.decode(it.substringBefore('='), "UTF-8").lowercase(Locale.ROOT) }
    if (keys.any { it in setOf("q", "query", "search", "search_query", "searchquery", "searchterm", "search_term", "keyword", "keywords") }) return@runCatching false
    if (parts.isEmpty()) return@runCatching allowArtistRoot && uri.rawQuery.isNullOrEmpty()
    val meaningful = parts.filterNot { it.matches(Regex("[a-z]{2}(-[a-z]{2})?")) }
    val queryIdentifiesTitle = meaningful == listOf("watch") && uri.rawQuery.orEmpty().split('&').any {
        val key = java.net.URLDecoder.decode(it.substringBefore('='), "UTF-8").lowercase(Locale.ROOT)
        key in setOf("v", "id", "video_id", "videoid", "movieid", "assetid", "contentid") && it.substringAfter('=', "").isNotBlank()
    }
    meaningful.isNotEmpty() && (queryIdentifiesTitle || !(meaningful.size == 1 && meaningful[0] in setOf(
        "home", "browse", "movies", "movie", "tv", "shows", "series", "watch", "welcome", "stream", "all", "account")))
}.getOrDefault(false)

data class CatalogAttribution(val name: String, val url: String) {
    companion object {
        fun parse(value: JSONObject): CatalogAttribution? {
            val name = value.catalogText("name", 160) ?: return null
            val url = value.catalogText("url", 8192)?.takeIf(::safeCatalogHttpsUrl) ?: return null
            return CatalogAttribution(name, url)
        }
    }
}
data class CatalogSource(val source: String, val status: String, val scope: String, val coverage: String,
    val observedAt: Long?, val attributions: List<CatalogAttribution>, val region: String? = null) {
    val transientFailure get() = status in setOf("denied", "rateLimited", "unavailable", "invalidResponse", "identityMismatch")
    val knownResult get() = status in setOf("complete", "partial", "noOffers")
    companion object {
        fun parse(value: JSONObject): CatalogSource = CatalogSource(
            value.catalogText("source", 120) ?: "Catalog", value.catalogText("status", 40) ?: "invalidResponse",
            value.catalogText("scope", 40) ?: "regionalAvailability", value.catalogText("coverage", 40) ?: "unknown",
            value.catalogSeconds("observedAt"), value.optJSONArray("attributions")?.objects(32)?.mapNotNull(CatalogAttribution::parse).orEmpty())
        fun failure(status: String = "unavailable", region: String? = null) =
            CatalogSource("Catalog", status, "regionalAvailability", "unknown", System.currentTimeMillis() / 1000, emptyList(), region)
    }
}
data class CatalogService(val source: String, val id: String, val name: String) {
    val key get() = source + ":" + id
    companion object {
        fun parse(value: JSONObject?): CatalogService? {
            value ?: return null
            return CatalogService(value.catalogText("source", 64) ?: return null,
                value.catalogText("id", 256) ?: return null, value.catalogText("name", 256) ?: return null)
        }
    }
}
data class CatalogDestination(val kind: String, val url: String)
data class CatalogPrice(val amount: String?, val currency: String?, val formatted: String?) {
    val display: String? get() = formatted ?: amount?.let { amount ->
        listOfNotNull(amount, currency).joinToString(" ")
    }
    companion object {
        fun parse(value: JSONObject?): CatalogPrice? {
            value ?: return null
            val amount = value.catalogText("amount", 64)?.takeIf {
                it.toDoubleOrNull()?.let { number -> number.isFinite() && number >= 0 } == true
            }
            return CatalogPrice(amount, value.catalogText("currency", 12), value.catalogText("formatted", 120))
        }
    }
}
data class CatalogEvidence(val source: String, val observedAt: Long?, val availableSince: Long?,
    val expiresAt: Long?, val expiresSoon: Boolean, val regionVerified: Boolean)
data class CatalogOffer(val id: String, val identity: CatalogIdentity, val country: String?, val service: CatalogService,
    val accessType: String, val rawAccessType: String?, val addon: CatalogService?, val destinations: List<CatalogDestination>,
    val price: CatalogPrice?, val quality: String?, val evidence: CatalogEvidence) {
    val artistLinkedAccess get() = identity.kind == "artist" && evidence.source == "musicbrainz" &&
        !evidence.regionVerified && country == null
    fun expired(now: Long) = evidence.expiresAt?.let { it <= now } == true
    fun future(now: Long) = evidence.availableSince?.let { it > now } == true
    fun bestDestination(now: Long): CatalogDestination? {
        if (expired(now) || future(now)) return null
        val priority = if (accessType == "download") listOf("download", "title", "player") else listOf("player", "title", "download")
        return priority.firstNotNullOfOrNull { kind -> destinations.firstOrNull { it.kind == kind } }
    }
    val accessLabel: String get() = when (accessType) {
        "free" -> "Free"; "ads" -> "With ads"; "subscription" -> "Subscription"; "rent" -> "Rent"
        "buy" -> "Buy"; "download" -> "Download"; "addon" -> "Add-on"; else -> rawAccessType ?: accessType
    }
    companion object {
        fun parse(value: JSONObject): CatalogOffer? {
            val id = value.catalogText("id", 256) ?: return null
            val identity = CatalogIdentity.parse(value.optJSONObject("identity")) ?: return null
            val country = value.catalogText("country", 2)
            if (value.has("country") && !value.isNull("country") && country?.matches(Regex("[A-Z]{2}")) != true) return null
            val service = CatalogService.parse(value.optJSONObject("service")) ?: return null
            val access = value.catalogText("accessType", 64) ?: return null
            val evidence = value.optJSONObject("evidence") ?: return null
            val destinations = value.optJSONArray("destinations")?.objects(16)?.mapNotNull { row ->
                val kind = row.catalogText("kind", 32)?.takeIf { it in setOf("title", "player", "download") } ?: return@mapNotNull null
                val artistRoot = identity.kind == "artist" && country == null &&
                    evidence.catalogText("source", 120) == "musicbrainz" && !evidence.optBoolean("regionVerified")
                val url = row.catalogText("url", 8192)?.takeIf { safeCatalogDestinationUrl(it, service.name, artistRoot) } ?: return@mapNotNull null
                CatalogDestination(kind, url)
            }.orEmpty().distinct()
            return CatalogOffer(id, identity, country, service, access, value.catalogText("rawAccessType", 80),
                CatalogService.parse(value.optJSONObject("addon")), destinations, CatalogPrice.parse(value.optJSONObject("price")),
                value.catalogText("quality", 80), CatalogEvidence(evidence.catalogText("source", 120) ?: "Catalog",
                    evidence.catalogSeconds("observedAt"), evidence.catalogSeconds("availableSince"),
                    evidence.catalogSeconds("expiresAt"), evidence.optBoolean("expiresSoon"), evidence.optBoolean("regionVerified")))
        }
    }
}
data class CatalogOfferResponse(val identity: CatalogIdentity, val country: String?, val offers: List<CatalogOffer>,
    val sources: List<CatalogSource>, val excluded: Int = 0) {
    val knownResult get() = sources.any(CatalogSource::knownResult)
    companion object {
        fun parse(value: JSONObject, expected: CatalogIdentity, country: String?): CatalogOfferResponse {
            require(value.optInt("schemaVersion") == 1) { "Unsupported catalog response" }
            val identity = CatalogIdentity.parse(value.optJSONObject("identity"))
            require(identity?.matches(expected) == true) { "Catalog identity changed" }
            val returned = value.catalogText("country", 2)
            require(!value.has("country") || value.isNull("country") || returned?.matches(Regex("[A-Z]{2}")) == true) { "Invalid catalog country" }
            require(returned == country && (returned == null || returned.matches(Regex("[A-Z]{2}")))) { "Catalog country changed" }
            val rows = value.getJSONArray("offers")
            val offers = rows.objects().mapNotNull(CatalogOffer::parse).filter { offer ->
                offer.identity.isOfferFor(expected) && (country == null || offer.country == country ||
                    offer.country == null && !offer.evidence.regionVerified)
            }.distinctBy(CatalogOffer::id)
            val sources = value.getJSONArray("sources").objects(5000).map(CatalogSource::parse)
            return CatalogOfferResponse(expected, returned, offers, sources, rows.length() - offers.size)
        }
        fun failed(identity: CatalogIdentity, country: String?) =
            CatalogOfferResponse(identity, country, emptyList(), listOf(CatalogSource.failure()))
    }
}
data class CatalogCountry(val code: String, val name: String, val services: List<CatalogService>)
data class CatalogRegistry(val countries: List<CatalogCountry>, val sources: List<CatalogSource>) {
    companion object {
        fun parse(value: JSONObject): CatalogRegistry {
            require(value.optInt("schemaVersion") == 1) { "Unsupported country registry" }
            val countries = value.getJSONArray("countries").objects(300).mapNotNull { row ->
                val code = row.catalogText("code", 2)?.takeIf { it.matches(Regex("[A-Z]{2}")) } ?: return@mapNotNull null
                val name = row.catalogText("name", 160) ?: code
                CatalogCountry(code, name, row.optJSONArray("services")?.objects(2000)?.mapNotNull {
                    CatalogService.parse(it.optJSONObject("service"))
                }.orEmpty())
            }.distinctBy(CatalogCountry::code)
            return CatalogRegistry(countries, value.optJSONArray("sources")?.objects(100)?.map(CatalogSource::parse).orEmpty())
        }
    }
}
data class CatalogArtCredit(val author: String, val url: String, val license: String?, val licenseUrl: String?) {
    companion object {
        fun parse(value: JSONObject?): CatalogArtCredit? {
            value ?: return null
            return CatalogArtCredit(value.catalogText("author") ?: return null,
                value.catalogText("url", 8192)?.takeIf(::safeCatalogHttpsUrl) ?: return null,
                value.catalogText("license"), value.catalogText("licenseUrl", 8192)?.takeIf(::safeCatalogHttpsUrl))
        }
    }
}
data class CatalogMusicItem(val identity: CatalogIdentity, val title: String, val subtitle: String?,
    val artwork: String?, val artworkCredit: CatalogArtCredit?, val sourceUrl: String, val number: String?) {
    companion object {
        fun parse(value: JSONObject): CatalogMusicItem? {
            val identity = CatalogIdentity.parse(value.optJSONObject("identity")) ?: return null
            return CatalogMusicItem(identity, value.catalogText("title", 512) ?: return null,
                value.catalogText("subtitle", 1000), value.catalogText("artwork", 8192)?.takeIf(::isCatalogMusicArtwork),
                CatalogArtCredit.parse(value.optJSONObject("artworkCredit")),
                value.catalogText("sourceUrl", 8192)?.takeIf(::safeCatalogHttpsUrl) ?: return null, value.catalogText("number", 64))
        }
    }
}
data class CatalogMusicSearch(val results: List<CatalogMusicItem>, val total: Long, val offset: Long,
    val sources: List<CatalogSource>) {
    companion object {
        fun parse(value: JSONObject) = CatalogMusicSearch(value.getJSONArray("results").objects(100).mapNotNull(CatalogMusicItem::parse),
            value.optLong("total").coerceAtLeast(0), value.optLong("offset").coerceAtLeast(0),
            value.optJSONArray("sources")?.objects(100)?.map(CatalogSource::parse).orEmpty())
    }
}
data class CatalogMusicDetail(val item: CatalogMusicItem?, val description: String?, val facts: List<Pair<String,String>>,
    val related: List<CatalogMusicItem>, val tracks: List<CatalogMusicItem>, val trackEdition: CatalogMusicItem?,
    val sources: List<CatalogSource>) {
    companion object {
        fun parse(value: JSONObject, expected: CatalogIdentity): CatalogMusicDetail {
            val item = value.optJSONObject("item")?.let(CatalogMusicItem::parse)
            require(item == null || item.identity.matches(expected)) { "Music detail identity changed" }
            val facts = value.optJSONArray("facts")?.objects(100)?.mapNotNull {
                val label = it.catalogText("label", 160) ?: return@mapNotNull null
                val text = it.catalogText("value", 2000) ?: return@mapNotNull null
                label to text
            }.orEmpty()
            return CatalogMusicDetail(item, value.catalogText("description", 16000), facts,
                value.optJSONArray("related")?.objects(1000)?.mapNotNull(CatalogMusicItem::parse).orEmpty(),
                value.optJSONArray("tracks")?.objects(2000)?.mapNotNull(CatalogMusicItem::parse).orEmpty(),
                value.optJSONObject("trackEdition")?.let(CatalogMusicItem::parse),
                value.optJSONArray("sources")?.objects(100)?.map(CatalogSource::parse).orEmpty())
        }
    }
}

internal fun deviceCatalogCountry(): String? =
    Locale.getDefault().country.uppercase(Locale.ROOT).takeIf { it.matches(Regex("[A-Z]{2}")) }

internal fun isCatalogMusicArtwork(url: String): Boolean {
    if (!safeCatalogHttpsUrl(url)) return false
    if (isCommonsArtwork(url)) return true
    val uri = runCatching { URI(url) }.getOrNull() ?: return false
    return uri.host == "coverartarchive.org" && uri.rawQuery == null && uri.rawFragment == null &&
        uri.path.matches(Regex("/release(?:-group)?/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/front-250"))
}


/** CAA publicly redirects its image URLs to Internet Archive; other image redirects stay disabled. */
internal fun safeCatalogArtworkRedirect(original: String, destination: String): Boolean {
    if (!isCatalogMusicArtwork(original) || URI(original).host != "coverartarchive.org" ||
        !safeCatalogHttpsUrl(destination)) return false
    val host = URI(destination).host.lowercase(Locale.ROOT)
    return host == "coverartarchive.org" || host == "archive.org" || host.endsWith(".archive.org")
}
