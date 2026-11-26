package org.bluefin.android

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import org.json.JSONObject

class LibraryModel(application: Application) : AndroidViewModel(application) {
    val core = BluefinRuntime.core(application)
    var demo by mutableStateOf(core.account == null); private set
    var section by mutableStateOf("Library"); private set
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
    private data class Shelf(
        val section: String, val search: String, val parent: String?,
        val items: List<LibraryItem>, val total: Int, val selected: LibraryItem?
    )
    private val previousShelves = ArrayDeque<Shelf>()
    val canGoBack: Boolean get() = selected != null || parentId != null || section != "Library"

    fun openSection(name: String) {
        previousShelves.clear()
        browse(name)
    }

    fun goBack() {
        if (selected != null) {
            select(null)
            error = null
            return
        }
        val previous = previousShelves.removeLastOrNull()
        if (previous == null) {
            openSection("Library")
            return
        }
        job?.cancel()
        ++generation
        busy = false
        error = null
        section = previous.section
        query = previous.search
        parentId = previous.parent
        rows = previous.items
        total = previous.total
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
    fun browse(name: String, search: String = "", parent: String? = null, append: Boolean = false) {
        job?.cancel(); val epoch = ++generation
        section = name; query = search; parentId = parent; selected = null; error = null
        if (demo) { rows = Demo.section(name, search); total = rows.size; busy = false; return }
        job = viewModelScope.launch {
            busy = true
            try {
                val fields = JSONObject().put("limit", 50).put("startIndex", if (append) rows.size else 0)
                parent?.let { fields.put("parentId", it) }
                if (search.isNotBlank()) fields.put("searchTerm", search)
                when (name) { "Movies" -> fields.put("includeItemTypes", "Movie"); "TV Shows" -> fields.put("includeItemTypes", "Series"); "Episodes" -> fields.put("includeItemTypes", "Episode"); "Music" -> fields.put("includeItemTypes", "Audio,MusicAlbum,MusicArtist") }
                val op = when { name == "Live TV" -> "liveChannels"; name == "Library" && parent == null && search.isEmpty() -> "libraries"; else -> "items" }
                val result = core.call(op, fields)
                if (epoch == generation) {
                    val next = result.items()
                    rows = if (append) rows + next else next
                    total = result.optInt("TotalRecordCount", rows.size)
                }
            } catch (e: Exception) { if (epoch == generation) error = e.message ?: "Library unavailable" }
            finally { if (epoch == generation) busy = false }
        }
    }
    fun more() = browse(section, query, parentId, true)
    fun children(item: LibraryItem) {
        if (previousShelves.size == 16) previousShelves.removeFirst()
        previousShelves.addLast(Shelf(section, query, parentId, rows, total, item))
        browse(if (item.type == "Series") "Episodes" else "Library", parent = item.id)
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
        done()
        openSection("Library")
    }

    fun signOut() {
        job?.cancel(); ++generation; busy = true
        viewModelScope.launch {
            val result = runCatching { core.logout() }
            cleanupPending = core.cleanupPending; cleanupMessage = core.cleanupMessage
            rows = emptyList(); selected = null; queue = emptyList(); downloads = emptyList()
            demo = true; openSection("Library")
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
    init { if (!demo) browse("Library") }
}
