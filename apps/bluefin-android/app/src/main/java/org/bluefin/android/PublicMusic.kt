package org.bluefin.android

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withTimeout

internal fun musicKindLabel(kind: String) = when (kind) {
    "artist" -> "Artist"
    "releaseGroup" -> "Album"
    "release" -> "Release"
    "recording" -> "Song"
    else -> kind
}

@Composable
internal fun PublicMusicResults(core: CoreClient, query: String) {
    val kinds = listOf("artist", "releaseGroup", "recording")
    var kind by remember(query) { mutableStateOf("all") }
    var offset by remember(query) { mutableLongStateOf(0) }
    var offsetHistory by remember(query, kind) { mutableStateOf<List<Long>>(emptyList()) }
    var pages by remember(query, kind, offset) { mutableStateOf<List<Pair<String, CatalogMusicSearch>>?>(null) }
    var failure by remember(query, kind, offset) { mutableStateOf(false) }
    var selected by remember(query) { mutableStateOf<CatalogMusicItem?>(null) }
    LaunchedEffect(core, query, kind, offset) {
        try {
            pages = coroutineScope {
                (if (kind == "all") kinds else listOf(kind)).map { requestedKind ->
                    async {
                        val result = try { withTimeout(30_000) { core.publicMusicSearch(requestedKind, query, offset) } }
                        catch (_: TimeoutCancellationException) { CatalogMusicSearch(emptyList(), 0, offset, listOf(CatalogSource.failure())) }
                        catch (cancelled: CancellationException) { throw cancelled }
                        catch (_: Exception) { CatalogMusicSearch(emptyList(), 0, offset, listOf(CatalogSource.failure())) }
                        requestedKind to result
                    }
                }.awaitAll()
            }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failure = true }
    }
    Text("Music catalog", style = MaterialTheme.typography.titleLarge)
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        (listOf("all") + kinds).forEach { value ->
            FilterChip(selected = kind == value, onClick = { kind = value; offset = 0 },
                label = { Text(when (value) { "all" -> "All music"; "artist" -> "Artists"; "releaseGroup" -> "Albums"; else -> "Songs" }) })
        }
    }
    val results = pages?.flatMap { it.second.results }?.distinctBy { it.identity.key }
    when {
        failure -> Text("Music search is temporarily unavailable.")
        results == null -> Text("Searching MusicBrainz…")
        results.isEmpty() -> Text(if (pages.orEmpty().flatMap { it.second.sources }.any { it.transientFailure })
            "Music search could not be completed. Try another search or return later." else "No music matches reported for this search.")
        else -> LazyRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            items(results, key = { it.identity.key }) { item ->
                Column(Modifier.width(160.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    PublicCatalogArtwork(item.artwork, Modifier.size(88.dp))
                    Text(musicKindLabel(item.identity.kind), style = MaterialTheme.typography.labelMedium)
                    TextButton(onClick = { selected = item }) { Text(item.title) }
                    item.subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 2) }
                }
            }
        }
    }
    if (kind != "all") {
        val page = pages?.firstOrNull()?.second
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (offset > 0) TextButton(onClick = {
                offset = offsetHistory.lastOrNull() ?: 0
                offsetHistory = offsetHistory.dropLast(1)
            }) { Text("Previous") }
            if (page != null && page.results.isNotEmpty() && page.offset + page.results.size < page.total)
                TextButton(onClick = {
                    offsetHistory = offsetHistory + offset
                    offset = page.offset + page.results.size
                }) {
                    Text("More " + when (kind) { "artist" -> "artists"; "releaseGroup" -> "albums"; else -> "songs" })
                }
        }
    }
    val failures = pages.orEmpty().flatMap { it.second.sources }.filter { !it.knownResult }
    if (!results.isNullOrEmpty() && failures.isNotEmpty())
        Text("Some music sources could not be checked.", style = MaterialTheme.typography.bodySmall)
    selected?.let { PublicMusicDetails(core, it) { selected = null } }
}

@Composable
private fun PublicMusicDetails(core: CoreClient, initial: CatalogMusicItem, dismiss: () -> Unit) {
    var stack by remember(initial.identity.key) { mutableStateOf(listOf(initial)) }
    val current = stack.last()
    var detail by remember(current.identity.key) { mutableStateOf<CatalogMusicDetail?>(null) }
    var failed by remember(current.identity.key) { mutableStateOf(false) }
    var retry by remember(current.identity.key) { mutableIntStateOf(0) }
    var openFailure by remember(current.identity.key) { mutableStateOf<String?>(null) }
    val context = LocalContext.current
    val open: (String) -> Unit = { url ->
        if (safeCatalogHttpsUrl(url)) runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
            .onFailure { openFailure = "This device could not open that source." }
    }
    val back: () -> Unit = { if (stack.size > 1) stack = stack.dropLast(1) else dismiss() }
    val select: (CatalogMusicItem) -> Unit = { item ->
        if (!item.identity.matches(current.identity)) stack = stack + item
    }
    LaunchedEffect(core, current.identity.key, retry) {
        try { detail = withTimeout(35_000) { core.publicMusicDetail(current.identity) } }
        catch (_: TimeoutCancellationException) { failed = true }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
    }
    val item = detail?.item ?: current
    Dialog(onDismissRequest = back, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            key(current.identity.key) {
                Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = back) { Text("Back") }
                        if (stack.size > 1) TextButton(onClick = dismiss) { Text("Close") }
                    }
                    Text(item.title, style = MaterialTheme.typography.headlineLarge)
                    Text(musicKindLabel(item.identity.kind), style = MaterialTheme.typography.labelLarge)
                    item.subtitle?.let { Text(it) }
                    PublicCatalogArtwork(item.artwork, Modifier.size(180.dp))
                    item.artworkCredit?.let { credit ->
                        TextButton(onClick = { open(credit.url) }) { Text("Artwork: " + credit.author) }
                        credit.license?.let { license ->
                            if (credit.licenseUrl != null) TextButton(onClick = { open(credit.licenseUrl) }) { Text(license) }
                            else Text(license, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                    detail?.description?.let { Text(it) }
                    detail?.facts.orEmpty().forEach { (label, value) -> Text("$label: $value") }
                    if (detail == null && !failed) Text("Loading music details…")
                    if (failed || detail?.item == null && detail != null) {
                        Text("Some details could not be loaded.")
                        TextButton(onClick = { failed = false; detail = null; retry++ }) { Text("Retry details") }
                    }
                    CatalogOffersSection(core, current.identity)
                    val related = detail?.related.orEmpty()
                    if (related.isNotEmpty()) {
                        Text(if (item.identity.kind == "artist") "Albums and releases" else "Artists and related works",
                            style = MaterialTheme.typography.titleLarge)
                        LazyRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            items(related, key = { it.identity.key }) { other ->
                                Column(Modifier.width(160.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                                    PublicCatalogArtwork(other.artwork, Modifier.size(88.dp))
                                    TextButton(onClick = { select(other) }) { Text(other.title) }
                                    other.subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 2) }
                                }
                            }
                        }
                    }
                    val tracks = detail?.tracks.orEmpty()
                    if (tracks.isNotEmpty()) {
                        Text("Tracks", style = MaterialTheme.typography.titleLarge)
                        detail?.trackEdition?.let { edition ->
                            TextButton(onClick = { select(edition) }) { Text("Edition: " + edition.title) }
                        }
                        tracks.forEachIndexed { index, track ->
                            TextButton(onClick = { select(track) }) {
                                Text((track.number ?: (index + 1).toString()) + ". " + track.title)
                            }
                        }
                    }
                    TextButton(onClick = { open(item.sourceUrl) }) { Text("View MusicBrainz source") }
                    CatalogSources(detail?.sources.orEmpty(), open)
                    openFailure?.let { Text(it) }
                }
            }
        }
    }
}
