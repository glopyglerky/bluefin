package org.bluefin.android

import android.content.Intent
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.Job
import kotlin.coroutines.coroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URI
import java.util.UUID
import android.net.Uri
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import android.app.UiModeManager
import android.content.res.Configuration
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import org.json.JSONObject

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val television = (getSystemService(UI_MODE_SERVICE) as UiModeManager).currentModeType == Configuration.UI_MODE_TYPE_TELEVISION
        setContent {
            MaterialTheme(colorScheme = bluefinColorScheme()) {
                Surface(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing), color = MaterialTheme.colorScheme.background) { Bluefin(television) }
            }
        }
    }
}
@Composable private fun Action(label: String, enabled: Boolean = true, modifier: Modifier = Modifier, click: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    Button(onClick = click, enabled = enabled,
        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.secondary, contentColor = MaterialTheme.colorScheme.onSecondary),
        modifier = modifier
        .onFocusChanged { focused = it.isFocused }
        .border(if (focused) 3.dp else 0.dp, if (focused) MaterialTheme.colorScheme.primary else Color.Transparent, RoundedCornerShape(24.dp))) { Text(label) }
}
@Composable fun Bluefin(television: Boolean, model: LibraryModel = viewModel()) {
    var accountScreen by remember { mutableStateOf(false) }
    var downloadsScreen by remember { mutableStateOf(false) }
    var queueIndex by remember { mutableStateOf<Int?>(null) }
    var offline by remember { mutableStateOf(false) }
    var playing by remember { mutableStateOf<LibraryItem?>(null) }
    var queueScreen by remember { mutableStateOf(false) }
    val current = playing
    if (current != null) {
        key(current.id) { PlaybackScreen(current, model, offline, queueIndex?.let { model.queue.drop(it).takeWhile { song -> song.audio } } ?: listOf(current)) { ended ->
            val next = queueIndex?.plus(1)
            if (ended && next != null && next < model.queue.size) { queueIndex = next; playing = model.queue[next] }
            else { playing = null; queueIndex = null }
        } }
        return
    }
    BackHandler(enabled = accountScreen || downloadsScreen || queueScreen || model.canGoBack) {
        when {
            accountScreen -> accountScreen = false
            downloadsScreen -> downloadsScreen = false
            queueScreen -> queueScreen = false
            else -> model.goBack()
        }
    }
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).padding(if (television) 32.dp else 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("BLUEFIN", style = MaterialTheme.typography.headlineLarge, color = MaterialTheme.colorScheme.primary)
        Row(Modifier.fillMaxWidth().focusGroup().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("Home", "Sports", "TV Shows", "Movies", "Live TV", "Music").forEach { name ->
                Action(name) { accountScreen = false; downloadsScreen = false; queueScreen = false; model.openSection(name) }
            }
        }
        Row(Modifier.fillMaxWidth().focusGroup().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Action("Search") { accountScreen = false; downloadsScreen = false; queueScreen = false; model.openSection("Search") }
            Action("Collections") { accountScreen = false; downloadsScreen = false; queueScreen = false; model.openSection("Library") }
            Action("Queue ${model.queue.size}") { accountScreen = false; downloadsScreen = false; queueScreen = true }
            Action("Downloads") { accountScreen = false; queueScreen = false; downloadsScreen = true; if (!model.demo) model.downloads() }
            Action("Account") { queueScreen = false; downloadsScreen = false; accountScreen = true }
        }
        model.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (model.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        when {
            accountScreen -> AccountScreen(model, television) { accountScreen = false }
            downloadsScreen -> Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Offline audio", style = MaterialTheme.typography.headlineMedium)
                Text("Downloads stay in app-private storage for this account. Server download rights are required.")
                Action("Refresh", !model.demo) { model.downloads() }
                model.downloads.forEach { raw ->
                    val v = JSONObject(raw); val id = v.optString("itemId")
                    Text("${v.optString("name", id)} · ${v.optString("state")} · ${v.optLong("bytes")} bytes")
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Action("Play downloaded", v.optString("state") == "completed") {
                            queueIndex = null; offline = true
                            playing = LibraryItem(id, v.optJSONObject("item")?.optString("Name", id) ?: id, "Audio")
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Action("Cancel") { model.downloads("downloadCancel", id) }
                            Action("Remove") { model.downloads("downloadRemove", id) }
                        }
                    }
                }
                if (model.downloads.isEmpty()) Text("No downloaded audio.")
            }
            queueScreen -> Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Play queue", style = MaterialTheme.typography.headlineMedium)
                if (model.queue.isEmpty()) Text("Add a movie, episode, song or channel from its detail page.")
                LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(model.queue.withIndex().toList(), key = { it.index }) { indexed ->
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Action("Play ${indexed.value.name}") { offline = false; queueIndex = indexed.index; playing = indexed.value }
                            Action("Remove") { model.remove(indexed.index) }
                        }
                    }
                }
            }
            else -> {
                if (model.section == "Search") {
                    var term by rememberSaveable { mutableStateOf(model.query) }
                    val search = remember { FocusRequester() }
                    TextEntry(term, { term = it }, "Search titles, artists, and channels", television,
                        modifier = Modifier.fillMaxWidth(), next = if (!model.busy) search else null)
                    Action("Search", !model.busy, Modifier.focusRequester(search)) { model.submitSearch(term) }
                    SearchRefinements(model)
                    val filters = model.searchFilters
                    if (filters.studio == null && filters.tag == null && filters.collection == null &&
                        model.query.length >= 2 && model.selected == null) {
                        if (filters.scope in listOf(MediaSearchScope.ALL, MediaSearchScope.SHOWS)) {
                            PublicTvResults(model.core, model.query)
                        }
                        if (filters.scope in listOf(MediaSearchScope.ALL, MediaSearchScope.MOVIES)) {
                            PublicFilmResults(model.core, model.query, if (model.demo) emptyList() else model.rows) {
                                offline = false; queueIndex = null; playing = it
                            }
                        }
                    }
                }
                Text(model.section, style = MaterialTheme.typography.headlineMedium)
                val detail = model.selected
                if (detail != null) {
                    Detail(detail, model, { offline = false; queueIndex = null; playing = detail })
                } else if (model.section == "Home") {
                    LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                        items(model.homeShelves) { shelf ->
                            Text(shelf.title, style = MaterialTheme.typography.titleLarge)
                            LazyRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                items(shelf.items, key = { it.id }) { item ->
                                    Box(Modifier.width(if (television) 240.dp else 160.dp)) {
                                        LibraryTile(item, model) { model.select(item) }
                                    }
                                }
                            }
                        }
                    }
                    if (model.homeShelves.isEmpty() && !model.busy) Text("Your Home fills from the media and activity on your server.")
                } else if (model.section == "Sports") {
                    LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        items(model.sportsListings) { listing ->
                            Text(listing.first, style = MaterialTheme.typography.titleMedium)
                            listing.second?.let { channel -> Action("View " + channel.name) { model.select(channel) } }
                        }
                    }
                    if (model.sportsListings.isEmpty() && !model.busy) Text("No sports listings are available from this server.")
                } else {
                    LazyVerticalGrid(columns = GridCells.Adaptive(if (television) 240.dp else 160.dp), modifier = Modifier.weight(1f),
                        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        items(model.rows, key = { it.id }) { item -> LibraryTile(item, model) { model.select(item) } }
                    }
                    if (model.rows.isEmpty() && !model.busy) Text("No items found.")
                    if (model.hasMore) Action("Load more", !model.busy) { model.more() }
                }
            }
        }
    }
}
@Composable private fun PublicTvResults(core: CoreClient, query: String) {
    var selected by remember(query) { mutableStateOf<PublicTvTitle?>(null) }
    var titles by remember(query) { mutableStateOf<List<PublicTvTitle>?>(null) }
    var failed by remember(query) { mutableStateOf(false) }
    LaunchedEffect(core, query) {
        try {
            titles = core.publicTvSearch(query)
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            failed = true
        }
    }
    Text("TV catalog", style = MaterialTheme.typography.titleLarge)
    val results = titles
    when {
        failed -> Text("Could not load TVmaze results.")
        results == null -> Text("Searching TV catalog…")
        results.isEmpty() -> Text("No matches from TVmaze.")
        else -> {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(results, key = { it.id }) { item ->
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        PublicTvArtwork(item, Modifier.width(60.dp).height(84.dp))
                        Action(listOfNotNull(item.name, item.year).joinToString(" · ")) {
                            selected = item
                        }
                    }
                }
            }
        }
    }
    selected?.let { title ->
        PublicMediaDetails(core, title.id.toString(), "series", title.name, title.imdbId, title.artwork,
            null, title.details, emptyList(), {}, { selected = null })
    }
}
@Composable private fun PublicFilmResults(core: CoreClient, query: String, library: List<LibraryItem>, play: (LibraryItem) -> Unit) {
    var titles by remember(query) { mutableStateOf<List<PublicFilmTitle>?>(null) }
    var failed by remember(query) { mutableStateOf(false) }
    var selected by remember(query) { mutableStateOf<PublicFilmTitle?>(null) }
    LaunchedEffect(core, query) {
        try { titles = core.publicFilmSearch(query) }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { failed = true }
    }
    Text("Film catalog", style = MaterialTheme.typography.titleLarge)
    val results = titles
    when {
        failed -> Text("Could not load film results.")
        results == null -> Text("Searching film catalog…")
        results.isEmpty() -> Text("No matches from Wikidata.")
        else -> LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(results, key = { it.id }) { item ->
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    PublicCatalogArtwork(item.artwork, Modifier.width(60.dp).height(84.dp))
                    Action(item.name) { selected = item }
                }
            }
        }
    }
    selected?.let { title ->
        PublicMediaDetails(core, title.id, "movie", title.name, null, title.artwork,
            title.artworkCredit, title.details, library, play, { selected = null })
    }
}
@Composable private fun PublicMediaDetails(
    core: CoreClient, id: String, kind: String, name: String, imdbId: String?,
    artwork: String?, credit: PublicArtworkCredit?, initial: PublicTitleSources?,
    library: List<LibraryItem>, play: (LibraryItem) -> Unit, dismiss: () -> Unit
) {
    val context = LocalContext.current
    var sources by remember(id, kind) { mutableStateOf<PublicTitleSources?>(null) }
    var failure by remember(id, kind) { mutableStateOf<String?>(null) }
    LaunchedEffect(core, id, kind) {
        try { sources = core.publicTitleSources(id, kind, imdbId) }
        catch (error: CancellationException) { throw error }
        catch (_: Exception) { failure = "Provider lookup is temporarily unavailable." }
    }
    val destinations = (sources?.links.orEmpty() + initial?.links.orEmpty()).distinctBy { it.url }
    val exactIds = sources?.imdbIds.orEmpty() + initial?.imdbIds.orEmpty()
    val local = if (kind == "movie") {
        library.filter { it.type == "Movie" && it.imdbId?.let(exactIds::contains) == true }
    } else emptyList()
    val credits = sources?.credits.orEmpty()
    val open: (String) -> Unit = { url ->
        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
            .onFailure { failure = "Could not open this destination." }
    }
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Action("Back", click = dismiss)
                Text(name, style = MaterialTheme.typography.headlineLarge)
                PublicCatalogArtwork(artwork, Modifier.width(130.dp).height(182.dp))
                credit?.let {
                    TextButton(onClick = { open(it.url) }) { Text(it.author) }
                    it.licenseUrl?.let { license -> TextButton(onClick = { open(license) }) { Text(it.license) } }
                }
                (sources?.description ?: initial?.description)?.let { Text(it) }
                local.forEach { item ->
                    Action(if (item.ticks > 0) "Resume in Bluefin" else "Play in Bluefin") { dismiss(); play(item) }
                }
                val offer = CatalogOfferReference.video(kind, sources?.tmdbId ?: initial?.tmdbId,
                    exactIds.singleOrNull() ?: imdbId)
                if (offer != null) CatalogOfferPanel(core, offer)
                else Text("Regional availability is unknown for this title.")
                Text("Catalog links", style = MaterialTheme.typography.titleLarge)
                destinations.forEach { link ->
                    Action((if (link.player) "Watch on " else "Open on ") + link.provider) { open(link.url) }
                }
                failure?.let { Text(it) }
                if (sources == null && failure == null) Text("Looking up providers…")
                if (sources != null && destinations.isEmpty() && local.isEmpty()) Text("No direct provider destinations found.")
                if (credits.isNotEmpty()) {
                    Text("Credits", style = MaterialTheme.typography.titleLarge)
                    credits.forEach { Text(it.name + " · " + it.role) }
                }
            }
        }
    }
}
@Composable private fun SearchRefinements(model: LibraryModel) {
    val filters = model.searchFilters
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SearchChoice(filters.scope.title, MediaSearchScope.entries.map { it.title }) { index ->
            model.refine(filters.copy(scope = MediaSearchScope.entries[index]))
        }
        SearchChoice(filters.studio?.name ?: "All studios", listOf("All studios") + model.searchFacets.studios.map { it.name }) { index ->
            model.refine(filters.copy(studio = model.searchFacets.studios.getOrNull(index - 1)))
        }
        SearchChoice(filters.tag ?: "All tags / services", listOf("All tags / services") + model.searchFacets.tags) { index ->
            model.refine(filters.copy(tag = model.searchFacets.tags.getOrNull(index - 1)))
        }
        SearchChoice(filters.collection?.name ?: "All collections", listOf("All collections") + model.searchFacets.collections.map { it.name }) { index ->
            model.refine(filters.copy(collection = model.searchFacets.collections.getOrNull(index - 1)))
        }
    }
    Text("Studios, service tags and collections come from your server's media metadata.", style = MaterialTheme.typography.bodySmall)
    model.facetsMessage?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
}
@Composable private fun SearchChoice(label: String, choices: List<String>, choose: (Int) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        OutlinedButton(onClick = { expanded = true }) { Text(label) }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            choices.forEachIndexed { index, title ->
                DropdownMenuItem(text = { Text(title) }, onClick = { expanded = false; choose(index) })
            }
        }
    }
}
@Composable private fun LibraryTile(item: LibraryItem, model: LibraryModel, click: () -> Unit) {
    var focus by remember { mutableStateOf(false) }
    Card(onClick = click, modifier = Modifier.fillMaxWidth().onFocusChanged { focus = it.hasFocus }
        .border(if (focus) 3.dp else 0.dp, if (focus) MaterialTheme.colorScheme.primary else Color.Transparent, RoundedCornerShape(12.dp))) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val aspect = when (item.type) {
                "Movie", "Series", "Season" -> 2f / 3f
                "MusicAlbum", "MusicArtist", "Audio" -> 1f
                else -> 16f / 9f
            }
            Box(Modifier.fillMaxWidth().aspectRatio(aspect).clip(RoundedCornerShape(12.dp))
                .background(Brush.linearGradient(listOf(MaterialTheme.colorScheme.surfaceVariant, MaterialTheme.colorScheme.surface, MaterialTheme.colorScheme.background)))) {
                ItemArtwork(item, model, Modifier.fillMaxSize())
            }
            Text(item.name, style = MaterialTheme.typography.titleMedium, maxLines = 2)
        }
    }
}
@Composable private fun Detail(item: LibraryItem, model: LibraryModel, play: () -> Unit) {
    val context = LocalContext.current
    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(item.name, style = MaterialTheme.typography.headlineLarge)
        Text(item.type, color = MaterialTheme.colorScheme.primary)
        Text(item.overview.ifEmpty { "No description available." })
        if (item.playable) {
            Action(if (item.ticks > 0) "Resume" else "Play", click = play)
            Action("Add to queue") { model.add(item) }
        } else Action("Browse contents") { model.children(item) }
        if (item.audio) Action("Download audio", !model.busy) { model.download(item) }
        CatalogOfferReference.library(item)?.let { CatalogOfferPanel(model.core, it) }
        item.providerLinks.forEach { (name, url) ->
            Action("Open $name") {
                if (safeProviderUrl(url)) runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
                    .onFailure { model.message("No app is available to open this provider page.") }
            }
        }
        model.guide.forEach { Text(it) }
        if (item.type == "TvChannel") Text("Playback requires a server-authorized tuner or provider source. This app does not supply broadcasts.")
        Action("Back", click = model::goBack)
    }
}
@Composable private fun AccountScreen(model: LibraryModel, television: Boolean, done: () -> Unit) {
    var licenses by rememberSaveable { mutableStateOf(false) }
    if (licenses) { LegalNotices { licenses = false }; return }
    var server by rememberSaveable { mutableStateOf("") }
    var user by rememberSaveable { mutableStateOf("") }
    // Passwords deliberately never use saved instance state.
    var password by remember { mutableStateOf("") }
    var connection by remember { mutableStateOf<AndroidAccountOptions?>(null) }
    var checking by remember { mutableStateOf(false) }
    var mode by remember { mutableStateOf("") }
    var status by remember { mutableStateOf("") }
    var connectGeneration by remember { mutableIntStateOf(0) }
    LaunchedEffect(connectGeneration) {
        if (connectGeneration == 0) return@LaunchedEffect
        checking = true
        status = ""
        try {
            val address = server.trim().let { if (it.contains("://")) it else "https://$it" }.trimEnd('/')
            val info = model.core.accountRequest(address, "info")
            check(info.has("Id") || info.has("ServerName")) { "The address did not return media server information." }
            val options = try {
                model.core.accountRequest(address, "accountOptions")
            } catch (error: CancellationException) { throw error }
            catch (_: Exception) {
                val legacy = runCatching { model.core.accountRequest(address, "quickConnectEnabled") }.getOrNull()
                JSONObject().put("quickConnect", legacy?.optBoolean("enabled") == true)
            }
            connection = AndroidAccountOptions(address, info.optString("ServerName", "Sign in"), options)
        } catch (error: CancellationException) { throw error }
        catch (error: Exception) { status = error.message ?: "Could not connect to this server." }
        finally { checking = false }
    }
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        Column(Modifier.widthIn(max = if (television) 800.dp else 560.dp).fillMaxWidth()
            .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Account", style = MaterialTheme.typography.headlineMedium)
            Action("Software licenses") { licenses = true }
            if (!model.demo && !model.cleanupPending) {
                var peer by remember { mutableStateOf("") }
                var address by remember { mutableStateOf("") }
                val peerFocus = remember { FocusRequester() }
                val addressFocus = remember { FocusRequester() }
                val pinFocus = remember { FocusRequester() }
                val httpsFocus = remember { FocusRequester() }
                val signOutFocus = remember { FocusRequester() }
                Text("Optional pinned Iroh route · obtain and verify the peer ID independently")
                TextEntry(peer, { peer = it }, "Verified peer ID", television,
                    Modifier.fillMaxWidth().focusRequester(peerFocus), next = addressFocus,
                    imeAction = ImeAction.Next)
                TextEntry(address, { address = it }, "Numeric IP:port", television,
                    Modifier.fillMaxWidth().focusRequester(addressFocus), next = pinFocus,
                    previous = peerFocus, keyboardType = KeyboardType.Uri)
                Action("Use pinned route", modifier = if (television) Modifier.focusRequester(pinFocus)
                    .focusProperties { up = addressFocus; down = httpsFocus } else Modifier) {
                    model.pinIroh(peer.trim(), address.trim())
                }
                Action("Use standard HTTPS", modifier = if (television) Modifier.focusRequester(httpsFocus)
                    .focusProperties { up = pinFocus; down = if (model.busy) FocusRequester.Default else signOutFocus } else Modifier) {
                    model.clearIroh()
                }
                Action("Sign out", !model.busy, if (television) Modifier.focusRequester(signOutFocus)
                    .focusProperties { up = httpsFocus } else Modifier) { password = ""; model.signOut(); done() }
            } else if (model.cleanupPending) {
                Text(model.cleanupMessage)
                Action("Finish sign out / retry cleanup", !model.busy) { password = ""; model.signOut() }
            } else {
                val connected = connection
                if (connected == null) {
                    val continueFocus = remember { FocusRequester() }
                    val canContinue = NativeCore.available && !checking && server.isNotBlank()
                    Text("Connect to your server")
                    TextEntry(server, { server = it }, "HTTPS server URL", television,
                        Modifier.fillMaxWidth(), next = if (canContinue) continueFocus else null,
                        imeAction = ImeAction.Done, keyboardType = KeyboardType.Uri)
                    Action(if (checking) "Connecting…" else "Continue", canContinue,
                        Modifier.focusRequester(continueFocus)) { connectGeneration++ }
                } else {
                    Text(connected.name, style = MaterialTheme.typography.headlineSmall)
                    when (mode) {
                        "" -> {
                            Action("Sign in") { mode = "password" }
                            if (connected.options.optBoolean("passkeys")) Action("Sign in with a passkey") { mode = "passkey" }
                            if (connected.options.optBoolean("quickConnect")) Action("Quick Connect") { mode = "code" }
                            if (connected.options.optBoolean("allowSignups")) Action("Create account") { mode = "create" }
                        }
                        "passkey", "code" -> AndroidAccountChallenge(mode, connected.address, model.core, television) {
                            model.accountReady(done)
                        }
                        else -> {
                            val creating = mode == "create"
                            val userFocus = remember { FocusRequester() }
                            val passwordFocus = remember { FocusRequester() }
                            val submitFocus = remember { FocusRequester() }
                            val admitted = !model.busy && user.isNotBlank() &&
                                password.length >= if (creating) 15 else 1
                            TextEntry(user, { user = it }, "Username", television,
                                Modifier.fillMaxWidth().focusRequester(userFocus), next = passwordFocus, imeAction = ImeAction.Next)
                            TextEntry(password, { password = it }, "Password", television,
                                Modifier.fillMaxWidth().focusRequester(passwordFocus), next = if (admitted) submitFocus else null,
                                previous = userFocus, keyboardType = KeyboardType.Password, secret = true)
                            if (creating) Text("Use at least 15 characters. Your server administrator can grant library access.")
                            Action(if (creating) "Create account" else "Sign in", admitted, Modifier.focusRequester(submitFocus)) {
                                val submitted = password
                                password = ""
                                model.login(connected.address, user, submitted, creating, onRefused = {
                                    mode = ""; connectGeneration++
                                }, done = done)
                            }
                        }
                    }
                    if (mode.isNotEmpty()) Action("Back to sign-in options", !model.busy) { mode = ""; password = "" }
                    Action("Change server", !model.busy && !checking && mode !in setOf("passkey", "code")) {
                        connection = null; mode = ""; user = ""; password = ""; status = ""
                    }
                }
                if (status.isNotEmpty()) Text(status, color = MaterialTheme.colorScheme.error)
                if (!NativeCore.available) {
                    Text("Server playback is unavailable in this build. You can still explore the demo.",
                        color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
}

private data class AndroidAccountOptions(val address: String, val name: String, val options: JSONObject)

@Composable private fun AndroidAccountChallenge(mode: String, server: String, core: CoreClient, television: Boolean, done: () -> Unit) {
    val context = LocalContext.current
    var status by remember { mutableStateOf("Preparing sign-in…") }
    var code by remember { mutableStateOf("") }
    var page by remember { mutableStateOf<URI?>(null) }
    var image by remember { mutableStateOf<android.graphics.Bitmap?>(null) }
    LaunchedEffect(mode, server) {
        val owner = coroutineContext[Job]
        val device = UUID.randomUUID().toString()
        val passkey = mode == "passkey"
        var proof: JSONObject? = null
        var completed = false
        try {
            val challenge = core.accountRequest(server, if (passkey) "passkeyInitiate" else "quickConnectInitiate",
                JSONObject().put("deviceId", device))
            val secret = if (passkey) null else challenge.getString("Secret")
            if (passkey) {
                proof = challenge.getJSONObject("proof")
                val base = "${server.trimEnd('/')}/Bluefin/Accounts/Passkey"
                val url = challenge.getString("url")
                val qr = challenge.getString("qr_url")
                check(url.startsWith("$base#") && url.removePrefix("$base#").matches(Regex("[a-f0-9]{32}")) &&
                    qr == "$base/QR/${url.substringAfter('#')}") { "Invalid passkey link." }
                page = URI(url)
                image = withContext(Dispatchers.IO) {
                    val connection = URI(qr).toURL().openConnection() as HttpURLConnection
                    connection.instanceFollowRedirects = false
                    connection.connectTimeout = 5000
                    connection.readTimeout = 5000
                    try {
                        check(connection.responseCode == 200 && connection.contentType == "image/png")
                        val data = ByteArray(65_537)
                        var size = 0
                        connection.inputStream.use { input ->
                            while (size < data.size) {
                                val n = input.read(data, size, data.size - size)
                                if (n < 0) break
                                size += n
                            }
                        }
                        check(size < data.size)
                        BitmapFactory.decodeByteArray(data, 0, size)
                    } finally { connection.disconnect() }
                }
            }
            code = challenge.getString(if (passkey) "code" else "Code")
            status = if (passkey) "Confirm the code in your browser." else "Enter this code in a client signed in to your server."
            repeat(98) {
                delay(3000)
                val fields = if (passkey) JSONObject().put("proof", proof) else JSONObject().put("secret", secret).put("deviceId", device)
                if (passkey || core.accountRequest(server, "quickConnectStatus", fields).optBoolean("Authenticated")) {
                    val accepted = withContext(NonCancellable) {
                        core.exchangeSignIn(server, device, if (passkey) "passkeyExchange" else "quickConnectAuthenticate", fields)
                            .also { accepted ->
                                completed = accepted
                                if (accepted && owner?.isActive == false) core.logout()
                            }
                    }
                    if (accepted && owner?.isActive != false) { done(); return@LaunchedEffect }
                }
            }
            status = "This sign-in request expired."
        } catch (error: CancellationException) { throw error }
        catch (error: Exception) { status = error.message ?: "Sign in failed." }
        finally {
            if (!completed && proof != null) {
                withContext(NonCancellable) {
                    runCatching { core.accountRequest(server, "passkeyCancel", JSONObject().put("proof", proof)) }
                }
            }
        }
    }
    Text(status)
    if (code.isNotEmpty()) Text(code, style = MaterialTheme.typography.headlineMedium)
    image?.let { Image(it.asImageBitmap(), contentDescription = "Scan to sign in with a passkey", modifier = Modifier.size(208.dp)) }
    if (!television) page?.let { target ->
        Action("Open passkey page") { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(target.toString()))) }
    }
}
