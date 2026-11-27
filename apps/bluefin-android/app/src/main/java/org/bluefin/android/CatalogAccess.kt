package org.bluefin.android

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withTimeout
import kotlin.coroutines.coroutineContext
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

internal fun catalogObservedDate(seconds: Long?): String? = seconds?.let {
    runCatching { DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)
        .withZone(ZoneId.systemDefault()).format(Instant.ofEpochSecond(it)) }.getOrNull()
}

internal fun catalogSourceLabel(source: String): String = when (source) {
    "tmdb", "tmdb-watch-page" -> "TMDB / JustWatch"
    "musicbrainz" -> "MusicBrainz"
    "apple" -> "Apple Music"
    else -> source
}

/** This is an ordinary exact HTTPS title/player handoff; Android resolves installed apps. */
internal fun openCatalogDestination(
    context: android.content.Context, offer: CatalogOffer, now: Long, failed: (String) -> Unit
) {
    val primary = offer.bestDestination(now) ?: return
    val candidates = (listOf(primary) + offer.destinations.filter { it.kind == "title" }).distinctBy { it.url }
    for (destination in candidates) {
        if (!safeCatalogDestinationUrl(destination.url, offer.service.name, offer.artistLinkedAccess)) continue
        if (runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(destination.url))) }.isSuccess) return
    }
    failed("This device could not open ${offer.service.name}.")
}

@Composable
internal fun CatalogSources(sources: List<CatalogSource>, open: (String) -> Unit) {
    val grouped = sources.groupBy { listOf(it.source, it.status, it.scope, it.coverage) }
    grouped.values.forEach { rows ->
        val source = rows.first()
        val regions = rows.mapNotNull { it.region }.distinct().size
        val status = when (source.status) {
            "complete" -> if (source.coverage == "sourceCatalog") "Source catalog checked" else "Reported results"
            "partial" -> "Partial source coverage"
            "noOffers" -> "No offers reported by this source"
            "rateLimited" -> "Source rate limit reached"
            "denied" -> "Source declined access"
            "missingCredentials" -> "Source credentials are unavailable"
            "unavailable" -> "Source temporarily unavailable"
            "unsupportedCountry" -> "Source does not support this country"
            "unsupportedIdentity" -> "No supported source identifier"
            "notFound" -> "Title not found by this source"
            "identityMismatch" -> "Source returned a different title"
            "invalidResponse" -> "Source response could not be verified"
            else -> "Source status: ${source.status}"
        }
        val observed = catalogObservedDate(rows.mapNotNull { it.observedAt }.maxOrNull())
        Text(listOfNotNull(catalogSourceLabel(source.source), status,
            if (regions > 1) "$regions regions" else null, observed?.let { "Checked $it" })
            .joinToString(" · "), style = MaterialTheme.typography.bodySmall)
        if (source.scope == "linkedAccess")
            Text("Artist or title links do not confirm availability in a country.", style = MaterialTheme.typography.bodySmall)
    }
    val attributions = sources.flatMap { it.attributions }.distinctBy { it.name to it.url }
    if (attributions.isNotEmpty()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            attributions.forEach { attribution ->
                TextButton(onClick = { open(attribution.url) }) { Text(attribution.name) }
            }
        }
    }
}

@Composable
internal fun CatalogOffersSection(core: CoreClient, identity: CatalogIdentity) {
    val context = LocalContext.current
    val defaultCountry = remember(identity.key) { deviceCatalogCountry() }
    var country by remember(identity.key) { mutableStateOf(defaultCountry) }
    var allRegions by remember(identity.key) { mutableStateOf(!identity.video && defaultCountry == null) }
    var registry by remember(identity.key) { mutableStateOf<CatalogRegistry?>(null) }
    var registrySources by remember(identity.key) { mutableStateOf<List<CatalogSource>>(emptyList()) }
    var snapshot by remember(identity.key) { mutableStateOf(CatalogLoadSnapshot()) }
    var retry by remember(identity.key) { mutableIntStateOf(0) }
    var selectedService by remember(identity.key) { mutableStateOf<String?>(null) }
    var regionMenu by remember { mutableStateOf(false) }
    var serviceMenu by remember { mutableStateOf(false) }
    var currentJob by remember(identity.key) { mutableStateOf<Job?>(null) }
    var actionFailure by remember(identity.key) { mutableStateOf<String?>(null) }
    val open: (String) -> Unit = { url ->
        if (safeCatalogHttpsUrl(url)) runCatching {
            context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        }.onFailure { actionFailure = "This device could not open that source." }
    }

    LaunchedEffect(core, identity.key, retry) {
        registry = null
        registrySources = emptyList()
        if (!identity.video) {
            // ISO names are selection metadata, not a claim of service availability.
            registry = CatalogRegistry(java.util.Locale.getISOCountries().map { code ->
                CatalogCountry(code, java.util.Locale.Builder().setRegion(code).build().getDisplayCountry(), emptyList())
            }, emptyList())
            return@LaunchedEffect
        }
        try {
            registry = withTimeout(25_000) { core.publicCatalogCountries(identity) }
            registrySources = registry?.sources.orEmpty()
        } catch (_: TimeoutCancellationException) {
            registry = CatalogRegistry(emptyList(), listOf(CatalogSource.failure()))
            registrySources = registry!!.sources
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) {
            registry = CatalogRegistry(emptyList(), listOf(CatalogSource.failure()))
            registrySources = registry!!.sources
        }
    }

    LaunchedEffect(core, identity.key, country, allRegions, retry, if (allRegions && identity.video) registry else null) {
        currentJob = coroutineContext[Job]
        selectedService = null
        actionFailure = null
        if (identity.video && country == null && !allRegions) {
            snapshot = CatalogLoadSnapshot()
            return@LaunchedEffect
        }
        snapshot = CatalogLoadSnapshot(running = true)
        try {
            if (identity.video && allRegions) {
                val countries = registry?.countries ?: return@LaunchedEffect
                if (countries.isEmpty()) {
                    snapshot = CatalogLoadSnapshot(sources = registrySources,
                        stoppedReason = "The source country list is unavailable. Choose a reported region or retry.")
                } else {
                    loadAllCatalogCountries(identity, countries, null,
                        lookup = { code ->
                            try { withTimeout(25_000) { core.publicCatalogOffers(identity, code) } }
                            catch (_: TimeoutCancellationException) { CatalogOfferResponse.failed(identity, code) }
                        }, update = { snapshot = it })
                }
            } else {
                val response = withTimeout(25_000) { core.publicCatalogOffers(identity, if (allRegions) null else country) }
                snapshot = CatalogLoadSnapshot(response.offers, response.sources, 1, 1)
            }
        } catch (_: TimeoutCancellationException) {
            snapshot = CatalogLoadSnapshot(sources = listOf(CatalogSource.failure()))
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) {
            snapshot = CatalogLoadSnapshot(sources = listOf(CatalogSource.failure()))
        }
    }

    val knownCountries = (registry?.countries.orEmpty() + snapshot.offers.mapNotNull { it.country }.distinct()
        .filter { code -> registry?.countries?.none { it.code == code } != false }
        .map { CatalogCountry(it, it, emptyList()) }).distinctBy { it.code }.sortedBy { it.name }
    val serviceChoices = snapshot.offers.map { it.service }.distinctBy { it.key }.sortedBy { it.name }
    val filtered = snapshot.offers.filter { selectedService == null || it.service.key == selectedService }
    val now = System.currentTimeMillis() / 1000

    Text(if (identity.video) "Where to watch" else "Listen, buy or download", style = MaterialTheme.typography.titleLarge)
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box {
            OutlinedButton(onClick = { regionMenu = true }) {
                Text(if (allRegions) "All reported regions" else knownCountries.firstOrNull { it.code == country }?.name
                    ?: country ?: "Choose a region")
            }
            DropdownMenu(expanded = regionMenu, onDismissRequest = { regionMenu = false },
                modifier = Modifier.heightIn(max = 400.dp)) {
                DropdownMenuItem(text = { Text("All reported regions") }, onClick = {
                    allRegions = true; country = null; regionMenu = false
                })
                knownCountries.forEach { region ->
                    DropdownMenuItem(text = { Text("${region.name} (${region.code})") }, onClick = {
                        allRegions = false; country = region.code; regionMenu = false
                    })
                }
            }
        }
        Box {
            OutlinedButton(onClick = { serviceMenu = true }, enabled = serviceChoices.isNotEmpty()) {
                Text(serviceChoices.firstOrNull { it.key == selectedService }?.name ?: "All services")
            }
            DropdownMenu(expanded = serviceMenu, onDismissRequest = { serviceMenu = false },
                modifier = Modifier.heightIn(max = 400.dp)) {
                DropdownMenuItem(text = { Text("All services") }, onClick = { selectedService = null; serviceMenu = false })
                serviceChoices.forEach { service ->
                    DropdownMenuItem(text = { Text(service.name) }, onClick = {
                        selectedService = service.key; serviceMenu = false
                    })
                }
            }
        }
        TextButton(onClick = { retry++ }) { Text("Refresh") }
    }

    if (identity.video && country == null && !allRegions)
        Text("Choose a region to check the services available there.")
    if (snapshot.running) {
        Text(if (allRegions && identity.video) {
            if (registry == null) "Loading the source country list…"
            else "Checking reported regions: ${snapshot.completed} of ${snapshot.total}. Results appear as each region finishes."
        } else "Checking source availability…")
        LinearProgressIndicator(Modifier.fillMaxWidth())
        TextButton(onClick = {
            currentJob?.cancel()
            snapshot = snapshot.copy(running = false, stoppedReason = if (allRegions && identity.video)
                "Stopped after ${snapshot.completed} of ${snapshot.total} regions." else "Lookup stopped.")
        }) { Text("Stop lookup") }
    }
    snapshot.stoppedReason?.let { Text(it) }

    if (filtered.isNotEmpty()) {
        val groups = filtered.groupBy { it.country }.entries.sortedBy { it.key ?: "ZZZ" }
        groups.forEach { group ->
            val region = group.key
            var expanded by remember(identity.key, country, allRegions, selectedService, region) {
                mutableStateOf(!allRegions || !identity.video || groups.size == 1)
            }
            val name = knownCountries.firstOrNull { it.code == region }?.name ?: region ?: "Country not confirmed"
            if (allRegions || region == null) {
                TextButton(onClick = { expanded = !expanded }) {
                    Text("$name · ${group.value.size} reported options · ${if (expanded) "Hide" else "Show"}")
                }
            }
            if (expanded || !allRegions && region != null) {
                group.value.sortedWith(compareBy({ it.service.name }, { it.accessType }, { it.quality.orEmpty() })).forEach { offer ->
                    Column(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(offer.service.name, style = MaterialTheme.typography.titleMedium)
                        Text(listOfNotNull(offer.accessLabel, offer.addon?.name?.let { "Add-on: $it" },
                            offer.price?.display, offer.quality).joinToString(" · "))
                        if (!offer.evidence.regionVerified) Text("Country availability not verified", style = MaterialTheme.typography.bodySmall)
                        catalogObservedDate(offer.evidence.observedAt)?.let { Text("Checked $it", style = MaterialTheme.typography.bodySmall) }
                        when {
                            offer.expired(now) -> Text("This reported offer expired ${catalogObservedDate(offer.evidence.expiresAt).orEmpty()}.")
                            offer.future(now) -> Text("Reported from ${catalogObservedDate(offer.evidence.availableSince).orEmpty()}.")
                            offer.evidence.expiresSoon -> Text("This offer may expire soon.")
                        }
                        if (offer.bestDestination(now) != null) {
                            Button(onClick = { openCatalogDestination(context, offer, now) { actionFailure = it } }) {
                                Text(when (offer.bestDestination(now)?.kind) {
                                    "player" -> "Play on ${offer.service.name}"
                                    "download" -> "Download from ${offer.service.name}"
                                    else -> "Open on ${offer.service.name}"
                                })
                            }
                        } else if (!offer.expired(now) && !offer.future(now)) {
                            Text("The source did not supply a direct title link.", style = MaterialTheme.typography.bodySmall)
                        }
                    }
                    HorizontalDivider()
                }
            }
        }
    } else if (!snapshot.running && (country != null || allRegions)) {
        Text(when {
            selectedService != null -> "No reported options match this service."
            snapshot.sources.any { it.transientFailure || it.status == "missingCredentials" } ->
                "Availability could not be fully checked. Refresh to retry."
            snapshot.sources.any { it.knownResult } -> "No offers reported for this selection."
            else -> "Availability has not been confirmed for this selection."
        })
    }
    Text("Availability is limited to the sources that returned results and can change. A listed service may require an account or payment.",
        style = MaterialTheme.typography.bodySmall)
    CatalogSources((snapshot.sources + registrySources).distinct(), open)
    actionFailure?.let { Text(it) }
}
