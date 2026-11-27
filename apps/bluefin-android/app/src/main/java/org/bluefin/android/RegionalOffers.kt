package org.bluefin.android

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.text.DateFormat
import java.util.Date
import java.util.Locale

internal data class CatalogOfferReference(val kind: String, val source: String, val id: String) {
    companion object {
        private val number = Regex("[1-9][0-9]{0,11}")
        private val imdb = Regex("tt[0-9]{7,12}")
        private val mbid = Regex("[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}")

        fun video(kind: String, tmdbId: String?, imdbId: String?): CatalogOfferReference? {
            if (kind != "movie" && kind != "series") return null
            if (tmdbId?.matches(number) == true) return CatalogOfferReference(kind, "tmdb", tmdbId)
            return imdbId?.takeIf { it.matches(imdb) }?.let { CatalogOfferReference(kind, "imdb", it) }
        }

        fun library(item: LibraryItem): CatalogOfferReference? {
            fun id(key: String) = item.providerIds.entries.firstOrNull { it.key.equals(key, true) }?.value
            fun music(kind: String, key: String): CatalogOfferReference? =
                id(key)?.takeIf { it.matches(mbid) && it.any { ch -> ch != '-' && ch != '0' } }
                    ?.let { CatalogOfferReference(kind, "musicbrainz", it.lowercase(Locale.ROOT)) }
            return when (item.type) {
                "Movie" -> video("movie", id("Tmdb"), item.imdbId)
                "Series" -> video("series", id("Tmdb"), item.imdbId)
                "MusicArtist" -> music("artist", "MusicBrainzArtist")
                "MusicAlbum" -> music("releaseGroup", "MusicBrainzReleaseGroup")
                    ?: music("release", "MusicBrainzAlbum")
                "Audio" -> music("recording", "MusicBrainzRecording")
                else -> null
            }
        }
    }
}

private val countryFormat = Regex("[A-Z]{2}")
private fun JSONObject.text(key: String): String? = (opt(key) as? String)?.takeIf { it.isNotBlank() }
private fun JSONArray?.objects(): List<JSONObject> =
    this?.let { list -> (0 until list.length()).mapNotNull(list::optJSONObject) }.orEmpty()
private fun exactHttps(value: String?): String? = value?.takeIf {
    val uri = runCatching { URI(it) }.getOrNull()
    uri?.scheme == "https" && uri.host != null && uri.rawUserInfo == null
}

private fun timestamp(seconds: Long): String? = seconds.takeIf { it in 1..253_402_300_799L }
    ?.let { DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(it * 1000)) }

private fun countryOf(offer: JSONObject): String = offer.text("country")
    ?.takeIf { it.matches(countryFormat) && offer.optJSONObject("evidence")?.optBoolean("regionVerified") == true }
    .orEmpty()

private fun sourceStatus(source: JSONObject): String = when (source.text("status")) {
    "complete" -> "Reported coverage"
    "partial" -> "Partial coverage"
    "noOffers" -> "No offers reported"
    "missingCredentials" -> "Source not configured"
    "notFound" -> "Title not found"
    "identityMismatch" -> "Title could not be confirmed"
    "denied" -> "Access denied"
    "rateLimited" -> "Rate limited"
    "invalidResponse" -> "Invalid response"
    "unsupportedCountry" -> "Country unsupported"
    "unsupportedIdentity" -> "Title type unsupported"
    else -> "Unavailable"
}

@Composable
internal fun CatalogOfferPanel(core: CoreClient, reference: CatalogOfferReference) {
    val context = LocalContext.current
    val origin = remember(core, core.account?.server) { core.catalogOrigin() }
    var country by remember(reference, origin) { mutableStateOf("") }
    var discovered by remember(origin) { mutableStateOf(emptyMap<String, String>()) }
    var response by remember(reference, origin, country) { mutableStateOf<JSONObject?>(null) }
    var unavailable by remember(reference, origin, country) { mutableStateOf(false) }
    var retry by remember(reference) { mutableStateOf(0) }
    var menu by remember { mutableStateOf(false) }
    var openMarkets by remember(reference, origin, country) { mutableStateOf(emptySet<String>()) }
    var openError by remember(reference, origin, country) { mutableStateOf(false) }

    LaunchedEffect(origin) {
        if (origin != null) try {
            discovered = core.regionalCatalog(origin).optJSONArray("countries").objects()
                .mapNotNull { record ->
                    val code = record.text("code")?.takeIf { it.matches(countryFormat) } ?: return@mapNotNull null
                    code to (record.text("name") ?: code)
                }.toMap()
        } catch (error: CancellationException) { throw error }
        catch (_: Exception) { discovered = emptyMap() }
    }
    LaunchedEffect(origin, reference, country, retry) {
        if (origin != null) {
            response = null
            unavailable = false
            try { response = core.regionalCatalog(origin, reference, country) }
            catch (error: CancellationException) { throw error }
            catch (_: Exception) { unavailable = true }
        }
    }
    val offers = remember(response) { response?.optJSONArray("offers").objects() }
    val sources = remember(response) { response?.optJSONArray("sources").objects() }
    val markets = remember(offers, country) {
        offers.groupBy(::countryOf).filterKeys { country.isEmpty() || it.isEmpty() || it == country }.toSortedMap()
    }
    fun regionName(code: String): String = discovered[code] ?: Locale("", code).displayCountry.ifBlank { code }
    fun open(url: String) {
        openError = runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }.isFailure
    }
    val choices = (discovered.keys + markets.keys + country).filter(String::isNotEmpty).distinct()
        .sortedBy(::regionName)

    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Text("Availability", style = MaterialTheme.typography.titleLarge)
        TextButton(onClick = { menu = true }, enabled = origin != null) {
            Text("Country · " + if (country.isEmpty()) "All regions" else regionName(country))
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(text = { Text("All regions") }, onClick = { country = ""; menu = false })
            choices.forEach { code ->
                DropdownMenuItem(text = { Text(regionName(code)) }, onClick = { country = code; menu = false })
            }
        }
        when {
            origin == null -> Text("Availability is unavailable.")
            unavailable -> {
                Text("Availability is unavailable.")
                TextButton(onClick = { retry += 1 }) { Text("Retry availability") }
            }
            response == null -> Text("Checking availability…")
            markets.isEmpty() -> {
                val reported = sources.any {
                    it.text("scope") == "regionalAvailability" && it.text("status") in setOf("complete", "noOffers")
                }
                Text(if (reported) "No offers were reported for this selection." else "Availability is unknown.")
            }
        }
        markets.forEach { (market, rows) ->
            val name = if (market.isEmpty()) "Region unverified" else regionName(market)
            val expanded = market in openMarkets
            if (markets.size == 1) Text(name, style = MaterialTheme.typography.titleMedium)
            else TextButton(onClick = { openMarkets = if (expanded) openMarkets - market else openMarkets + market }) {
                Text(if (expanded) "Hide $name" else name)
            }
            if (markets.size == 1 || expanded) rows.forEach { OfferRow(it, ::open) }
        }
        sources.forEach { source ->
            val name = source.text("source").orEmpty()
            Text("$name · ${sourceStatus(source)}", style = MaterialTheme.typography.bodySmall)
            source.optJSONArray("attributions").objects().forEach { attribution ->
                val url = exactHttps(attribution.text("url"))
                val label = attribution.text("name")
                if (url != null && label != null) TextButton(onClick = { open(url) }) { Text(label) }
            }
        }
        if (openError) Text("Could not open this destination.")
    }
}

@Composable
private fun OfferRow(offer: JSONObject, open: (String) -> Unit) {
    val provider = offer.optJSONObject("service")?.text("name") ?: return
    val offerType = offer.text("accessType")
    val access = when (offerType) {
        "free" -> "Free"
        "ads" -> "With ads"
        "subscription" -> "Subscription"
        "rent" -> "Rent"
        "buy" -> "Buy"
        "download" -> "Download"
        "addon" -> "Add-on"
        else -> offer.text("rawAccessType") ?: "Other access"
    }
    val evidence = offer.optJSONObject("evidence")
    val expired = evidence?.let { !it.isNull("expiresAt") &&
        it.optLong("expiresAt", Long.MAX_VALUE) <= System.currentTimeMillis() / 1000 } == true
    val priorities = if (offerType == "download") listOf("download", "title") else listOf("player", "title", "download")
    val urls = offer.optJSONArray("destinations").objects()
    val destination = if (expired) null else priorities.firstNotNullOfOrNull { kind ->
        urls.firstNotNullOfOrNull { url -> exactHttps(url.text("url"))?.takeIf { url.text("kind") == kind }?.let { kind to it } }
    }
    val action = when {
        offerType in setOf("rent", "buy", "download") -> access
        destination?.first == "player" -> "Play"
        destination?.first == "download" -> "Download"
        else -> "Open"
    }
    if (destination != null) TextButton(onClick = { open(destination.second) }) { Text("$action on $provider") }
    else Text(provider)
    val price = offer.optJSONObject("price")?.let { it.text("formatted") ?:
        listOfNotNull(it.text("amount"), it.text("currency")).joinToString(" ").ifBlank { null } }
    val facts = listOfNotNull(access, offer.optJSONObject("addon")?.text("name"), offer.text("quality"), price,
        if (expired) "Expired" else null, evidence?.text("source"), evidence?.optLong("observedAt")?.let(::timestamp))
    Text(facts.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
}
