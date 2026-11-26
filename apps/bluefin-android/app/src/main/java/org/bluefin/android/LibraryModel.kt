package org.bluefin.android

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CancellationException
import org.json.JSONArray
import java.util.UUID
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import org.json.JSONObject

class LibraryModel(application: Application) : AndroidViewModel(application) {
    val core = BluefinRuntime.core(application)
    var demo by mutableStateOf(core.account == null); private set
    var section by mutableStateOf("Home"); private set
    var rows by mutableStateOf(Demo.items); private set
    var selected by mutableStateOf<LibraryItem?>(null); private set
    var queue by mutableStateOf<List<LibraryItem>>(emptyList()); private set
    var error by mutableStateOf<String?>(null); private set
    var busy by mutableStateOf(false); private set
    var cleanupPending by mutableStateOf(core.cleanupPending); private set
    var cleanupMessage by mutableStateOf(core.cleanupMessage); private set
    var total by mutableStateOf(0); private set
    var query by mutableStateOf(""); private set
    var guide by mutableStateOf<List<String>>(emptyList()); private set
    var downloads by mutableStateOf<List<String>>(emptyList()); private set
    var homeShelves by mutableStateOf<List<HomeShelf>>(emptyList()); private set
    var sportsListings by mutableStateOf<List<Pair<String, LibraryItem?>>>(emptyList()); private set
    var searchFilters by mutableStateOf(MediaSearchFilters()); private set
    var searchFacets by mutableStateOf(MediaSearchFacets()); private set
    var facetsMessage by mutableStateOf<String?>(null); private set
    private var facetsOwner: Account? = null
    private var nextIndex = 0
    private var homeHistoryGeneration: Long? = null
    private var pinnedOwner: Account? = null
    private var pinned: JSONObject? = null
    fun pinIroh(peer: String, address: String) {
        val owner = core.account ?: return
        // Explicit user-provided pin only. Native core performs endpoint/address and handshake validation.
        if (!peer.matches(Regex("[0-9a-fA-F]{64}")) || address.length !in 3..128 || address.any { it.isWhitespace() }) {
            error = "Enter the verified 64-digit peer ID and numeric IP:port."; return
        }
        pinnedOwner = owner; pinned = JSONObject().put("peerId", peer).put("peerAddress", address)
        error = "Pinned route selected. Native validation occurs on playback; failures do not fall back."
    }
    fun clearIroh() { pinned = null; pinnedOwner = null }
    fun irohConfig(): JSONObject? = if (core.account == pinnedOwner) pinned?.let { JSONObject(it.toString()) } else null
    private var generation = 0
    private var job: Job? = null
    private var parentId: String? = null
    private var artistId: String? = null
    private data class Shelf(
        val section: String, val search: String, val parent: String?,
        val items: List<LibraryItem>, val total: Int, val selected: LibraryItem?, val nextIndex: Int,
        val artist: String?
    )
    private val previousShelves = ArrayDeque<Shelf>()
    val canGoBack: Boolean get() = selected != null || parentId != null || section != "Home"

    fun openSection(name: String) {
        if (name == "Search") {
            searchFilters = MediaSearchFilters()
            loadSearchFacets()
        }
        previousShelves.clear()
        browse(name)
    }
    fun refine(filters: MediaSearchFilters) {
        searchFilters = filters
        if (query.isNotBlank()) browse("Search", query)
    }
    private fun loadSearchFacets() {
        val owner = core.account ?: return
        if (facetsOwner == owner) return
        facetsMessage = null
        viewModelScope.launch {
            try {
                val facets = MediaSearchFacets.parse(core.callOwned(owner, "searchFilters"))
                if (core.account == owner) {
                    searchFacets = facets; facetsOwner = owner
                    if (facets.truncated) facetsMessage = "More studios or collections exist on this server. Search still covers the full catalog."
                }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { if (core.account == owner) facetsMessage = "Server refinements could not load. Media-type search is available." }
        }
    }
    fun submitSearch(term: String) {
        val query = term.trim().take(128)
        browse("Search", query)
        val owner = core.account ?: return
        val version = homeHistoryGeneration ?: return
        if (query.isEmpty()) return
        val event = JSONObject().put("eventId", UUID.randomUUID().toString())
            .put("query", query).put("generation", version).put("occurredAtUnixMs", System.currentTimeMillis())
        viewModelScope.launch {
            try { core.callOwned(owner, "homeSearchRecord", JSONObject().put("event", event)) }
            catch (e: CancellationException) { throw e }
            catch (_: Exception) { /* Search remains usable if history is unavailable. */ }
        }
    }

    fun goBack() {
        if (selected != null) {
            select(null)
            error = null
            return
        }
        val previous = previousShelves.removeLastOrNull()
        if (previous == null) {
            openSection("Home")
            return
        }
        job?.cancel()
        ++generation
        busy = false
        error = null
        section = previous.section
        query = previous.search
        parentId = previous.parent
        artistId = previous.artist
        rows = previous.items
        total = previous.total
        nextIndex = previous.nextIndex
        select(previous.selected)
    }
    fun message(text: String) {
        error = text; cleanupPending = core.cleanupPending; cleanupMessage = core.cleanupMessage
        if (cleanupPending && core.account == null) demo = true
    }
    fun add(item: LibraryItem) { if (item.playable && queue.size < 200) queue = queue + item }
    fun remove(index: Int) { queue = queue.filterIndexed { i, _ -> i != index } }
    fun select(item: LibraryItem?) {
        selected = item; guide = emptyList()
        if (item?.type == "TvChannel") {
            if (demo) guide = listOf("First light · synthetic program, no broadcast")
            else {
                val epoch = generation
                viewModelScope.launch {
                    runCatching { core.call("livePrograms", JSONObject().put("channelIds", org.json.JSONArray().put(item.id)).put("isAiring", true)) }
                        .onSuccess { v -> if (epoch == generation && selected?.id == item.id) {
                            val a = v.optJSONArray("Items")
                            guide = if (a == null) emptyList() else (0 until a.length()).map {
                                val program = a.getJSONObject(it); "${program.optString("Name")} · ${program.optString("StartDate")} to ${program.optString("EndDate")}"
                            }
                        } }
                        .onFailure { if (epoch == generation && selected?.id == item.id) error = "Program guide unavailable." }
                }
            }
        }
        if (item != null && !demo) {
            val epoch = generation
            viewModelScope.launch {
                runCatching { core.call("item", JSONObject().put("itemId", item.id)) }
                    .onSuccess { if (epoch == generation && selected?.id == item.id) selected = LibraryItem.parse(it) }
                    .onFailure { if (epoch == generation && selected?.id == item.id) error = "Item details could not be loaded." }
            }
        }
    }
    fun browse(name: String, search: String = "", parent: String? = null, append: Boolean = false, artist: String? = null) {
        job?.cancel(); val epoch = ++generation
        val owner = core.account
        section = name; query = search; parentId = parent; artistId = artist; selected = null; error = null
        if (!append) { nextIndex = 0; rows = emptyList(); total = 0 }
        if (name != "Home") homeShelves = emptyList()
        if (name != "Sports") sportsListings = emptyList()
        if (demo) {
            rows = Demo.section(name, search).filter { name != "Search" || searchFilters.scope.types.isEmpty() || it.type in searchFilters.scope.types }
            if (name == "Home") homeShelves = listOf(HomeShelf("Explore Bluefin", rows))
            if (name == "Sports") rows = emptyList()
            total = rows.size; busy = false; return
        }
        val criteria = searchFilters
        job = viewModelScope.launch {
            busy = true
            try {
                if (name == "Search" && search.isBlank()) return@launch
                if (name == "Home") {
                    val home = core.callOwned(owner, "home")
                    fun shelf(key: String, title: String): HomeShelf {
                        val array = home.optJSONArray(key) ?: JSONArray()
                        return HomeShelf(title, JSONObject().put("Items", array).items())
                    }
                    val initial = listOf(shelf("resume", "Continue watching"), shelf("nextUp", "Up next"), shelf("latest", "Recently added")).filter { it.items.isNotEmpty() }
                    if (epoch == generation && core.account == owner) { homeShelves = initial; rows = initial.flatMap { it.items }.distinctBy { it.id }; busy = false }
                    val recommendations = core.callOwned(owner, "homeRecommendations")
                    val shelves = recommendations.optJSONArray("recommendations") ?: JSONArray()
                    val personal = (0 until minOf(shelves.length(), 4)).map { index ->
                        val shelf = shelves.getJSONObject(index)
                        HomeShelf(shelf.optString("title", "For you"), JSONObject().put("Items", shelf.optJSONArray("items") ?: JSONArray()).items())
                    }.filter { it.items.isNotEmpty() }
                    if (epoch == generation && core.account == owner) {
                        homeHistoryGeneration = if (recommendations.optBoolean("searchHistorySupported")) recommendations.getLong("searchHistoryGeneration") else null
                        homeShelves = initial.take(2) + personal + initial.drop(2)
                        rows = homeShelves.flatMap { it.items }.distinctBy { it.id }; total = rows.size
                    }
                    return@launch
                }
                if (name == "Sports") {
                    val feed = core.callOwned(owner, "sportsHome")
                    val listings = listOf("games", "local_broadcasts").flatMap { key ->
                        val array = feed.optJSONArray(key) ?: JSONArray()
                        (0 until minOf(array.length(), 200)).map { array.getJSONObject(it) }
                    }.map { listing ->
                        val title = listing.optJSONObject("event")?.optString("name")?.takeIf { it.isNotBlank() } ?: listing.optString("title").ifBlank { listing.optString("native_title").ifBlank { "Sports broadcast" } }
                        val channel = listing.optString("channel_id").takeIf { listing.optString("kind") in setOf("LIVE", "ON_NOW", "UPCOMING") && it.matches(Regex("[A-Za-z0-9-]{1,128}")) }
                        val item = channel?.let { LibraryItem(it, listing.optString("channel_name", title), "TvChannel", title) }
                        title to item
                    }.distinctBy { it.first to it.second?.id }
                    if (epoch == generation && core.account == owner) { sportsListings = listings; total = 0 }
                    return@launch
                }
                val fields = if (name == "Search") criteria.fields(search, if (append) nextIndex else 0)
                    else JSONObject().put("limit", 50).put("startIndex", if (append) nextIndex else 0)
                parent?.let { fields.put("parentId", it) }
                artist?.let { fields.put("artistId", it).put("kind", "albums") }
                if (search.isNotBlank()) fields.put("searchTerm", search.take(128))
                when (name) { "Movies" -> fields.put("includeItemTypes", "Movie"); "TV Shows" -> fields.put("includeItemTypes", "Series"); "Episodes" -> fields.put("includeItemTypes", "Episode"); "Music" -> fields.put("includeItemTypes", "Audio,MusicAlbum,MusicArtist,Playlist") }
                val op = when { artist != null -> "musicItems"; name == "Live TV" -> "liveChannels"; name == "Library" && parent == null && search.isEmpty() -> "libraries"; else -> "items" }
                val result = core.callOwned(owner, op, fields)
                if (epoch == generation && core.account == owner) {
                    val next = result.items()
                    nextIndex = (if (append) nextIndex else 0) + next.size
                    rows = (if (append) rows + next else next).distinctBy { it.id }
                    total = if (next.isEmpty()) nextIndex else result.optInt("TotalRecordCount", nextIndex)
                }
            } catch (e: CancellationException) { throw e }
            catch (e: Exception) { if (epoch == generation && core.account == owner) error = e.message ?: "Library unavailable" }
            finally { if (epoch == generation) busy = false }
        }
    }
    val hasMore: Boolean get() = nextIndex < total && section !in setOf("Home", "Sports")
    fun more() = browse(section, query, parentId, true, artistId)
    fun children(item: LibraryItem) {
        if (previousShelves.size == 16) previousShelves.removeFirst()
        previousShelves.addLast(Shelf(section, query, parentId, rows, total, item, nextIndex, artistId))
        val artist = item.id.takeIf { item.type == "MusicArtist" }
        browse(if (artist != null) "Music" else if (item.type == "Series") "Episodes" else "Library",
            parent = item.id.takeIf { artist == null }, artist = artist)
    }
    fun login(server: String, user: String, password: String, create: Boolean = false, onRefused: () -> Unit = {}, done: () -> Unit) {
        viewModelScope.launch {
            busy = true
            try {
                if (create) core.accountRequest(server, "createAccount", JSONObject().put("username", user.trim()).put("password", password))
                core.login(server.trim(), user.trim(), password)
                accountReady(done)
            }
            catch (e: Exception) { error = e.message ?: "Sign in failed"; if (create) onRefused() }
            finally { cleanupPending = core.cleanupPending; cleanupMessage = core.cleanupMessage; busy = false }
        }
    }
    fun accountReady(done: () -> Unit) {
        demo = false
        queue = emptyList()
        searchFilters = MediaSearchFilters(); searchFacets = MediaSearchFacets(); facetsOwner = null; homeHistoryGeneration = null
        Artwork.clear()
        done()
        openSection("Home")
    }

    fun signOut() {
        job?.cancel(); ++generation; busy = true
        viewModelScope.launch {
            val result = runCatching { core.logout() }
            cleanupPending = core.cleanupPending; cleanupMessage = core.cleanupMessage
            rows = emptyList(); selected = null; queue = emptyList(); downloads = emptyList()
            homeShelves = emptyList(); sportsListings = emptyList(); homeHistoryGeneration = null
            searchFilters = MediaSearchFilters(); searchFacets = MediaSearchFacets(); facetsOwner = null
            Artwork.clear()
            demo = true; openSection("Home")
            val report = result.getOrNull()
            if (report?.complete != true || cleanupPending) error = cleanupMessage
            busy = false
        }
    }
    fun download(item: LibraryItem) {
        if (demo) { error = "Downloads require an authorized server audio item."; return }
        viewModelScope.launch {
            runCatching { core.call("downloadStart", JSONObject().put("itemId", item.id)) }
                .onSuccess { error = "Download started. Refresh Downloads for status." }
                .onFailure { error = it.message }
        }
    }
    fun downloads(operation: String = "downloads", id: String? = null) {
        viewModelScope.launch {
            runCatching {
                val owner = core.account
                if (operation == "downloadRemove") MusicService.stopAndJoin()
                if (operation != "downloads") core.callOwned(owner, operation, JSONObject().put("itemId", id))
                core.callOwned(owner, "downloads")
            }.onSuccess { v -> val a = v.optJSONArray("Items"); downloads = if (a == null) emptyList() else (0 until a.length()).map { a.getJSONObject(it).toString() } }
                .onFailure { error = it.message }
        }
    }
    init { browse("Home") }
}
