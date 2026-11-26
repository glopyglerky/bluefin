package org.bluefin.android

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest

object NativeCore {
    val available = runCatching { System.loadLibrary("bluefin_android") }.isSuccess
    @JvmStatic external fun invoke(request: String): String
}

internal fun decodeNativeReply(op: String, response: JSONObject): JSONObject {
    if (!response.optBoolean("ok")) {
        val message = response.optString("error", "Native request failed")
        if (op == "releasePlayback" && response.has("cleanupResources")) {
            throw NativeCleanupFailure(message, readResources(response.getJSONObject("cleanupResources")))
        }
        error(message)
    }
    if (op == "quickConnectEnabled") return JSONObject().put("enabled", response.optBoolean("value"))
    return response.optJSONObject("value") ?: JSONObject().put("Items", response.optJSONArray("value") ?: JSONArray())
}

data class Account(val server: String, val userId: String, val token: String, val deviceId: String) {
    val scope: String get() = MessageDigest.getInstance("SHA-256")
        .digest("$server\n$userId".toByteArray()).joinToString("") { "%02x".format(it) }
    fun json() = JSONObject().put("server", server).put("userId", userId).put("token", token).put("deviceId", deviceId)
}

object BluefinRuntime {
    @Volatile private var client: CoreClient? = null
    fun core(context: Context): CoreClient = client ?: synchronized(this) {
        client ?: CoreClient(context.applicationContext).also { client = it }
    }
}

data class SignOutReport(val localCleanupComplete: Boolean, val serverRevoked: Boolean, val playbackReleaseComplete: Boolean,
    val protectedStateComplete: Boolean = true) {
    val complete get() = localCleanupComplete && serverRevoked && playbackReleaseComplete && protectedStateComplete
}

class CoreClient private constructor(private val context: Context?, store: CleanupStore,
    private val exchange: ((String, JSONObject, Account?) -> JSONObject)?, private val stopReaders: suspend () -> Unit) {
    constructor(context: Context) : this(context, AccountVault(context), null, {
        withContext(Dispatchers.Main.immediate) { foregroundVideoReaders.stopAndJoin() }
        MusicService.stopAndJoin()
    })
    internal constructor(store: CleanupStore, stopReaders: suspend () -> Unit = {}, exchange: (String, JSONObject, Account?) -> JSONObject) : this(null, store, exchange, stopReaders)
    private val mutex = Mutex()
    private val cleanup = CleanupCoordinator(store, ::request)
    val account: Account? get() = cleanup.account
    val cleanupPending: Boolean get() = cleanup.cleanupPending
    val cleanupMessage: String get() = cleanup.cleanupMessage
    // Android's /data/user/0 alias is a platform symlink; resolve the app-owned base
    // before the core performs its strict no-follow traversal of offline entries.
    private fun cacheRoot() = java.io.File(context!!.noBackupFilesDir.canonicalFile, "offline").apply { mkdirs() }.absolutePath
    suspend fun call(operation: String, fields: JSONObject = JSONObject()): JSONObject = mutex.withLock {
        withContext(Dispatchers.IO) { cleanup.call(operation, fields) }
    }
    suspend fun callOwned(owner: Account?, operation: String, fields: JSONObject = JSONObject()): JSONObject = mutex.withLock {
        check(account == owner) { "Account changed. Retry from the current library." }
        withContext(Dispatchers.IO) { cleanup.call(operation, fields, owner) }
    }
    private fun request(op: String, fields: JSONObject, owner: Account?): JSONObject {
        exchange?.let { return it(op, fields, owner) }
        check(NativeCore.available) { "Android native core is not packaged. Demo is available." }
        val input = owner?.json() ?: JSONObject()
        fields.keys().forEach { input.put(it, fields.get(it)) }
        input.put("operation", op).put("clientPlatform", "android").put("cacheRoot", cacheRoot())
        CoreRequestEnvironment.decorate(input)
        val encoded = input.toString()
        check(encoded.toByteArray(Charsets.UTF_8).size <= 65536) { "Request too large" }
        return decodeNativeReply(op, JSONObject(NativeCore.invoke(encoded)))
    }
    suspend fun login(server: String, username: String, password: String) = mutex.withLock {
        withContext(Dispatchers.IO) {
            cleanup.login(server, username, password)
        }
    }
    suspend fun accountRequest(server: String, op: String, fields: JSONObject = JSONObject()): JSONObject = mutex.withLock {
        require(op in setOf("info", "accountOptions", "quickConnectEnabled", "createAccount",
            "quickConnectInitiate", "quickConnectStatus", "passkeyInitiate", "passkeyCancel"))
        check(op == "passkeyCancel" || account == null && !cleanupPending) { "Finish sign out before switching accounts." }
        withContext(Dispatchers.IO) { request(op, fields.put("server", server), null) }
    }

    suspend fun exchangeSignIn(server: String, device: String, op: String, fields: JSONObject): Boolean = mutex.withLock {
        require(op == "passkeyExchange" || op == "quickConnectAuthenticate")
        withContext(Dispatchers.IO) {
            cleanup.loginUsing(server, device) {
                request(op, fields.put("server", server).put("deviceId", device), null)
            }
        }
    }

    suspend fun beginSignOut() = mutex.withLock { withContext(Dispatchers.IO) { cleanup.beginSignOut() } }
    /** Serialized behind in-flight calls. Core logout must cancel and join downloads before returning. */
    suspend fun logout(): SignOutReport {
        beginSignOut()
        // Finalizers need the core mutex. Inactivate/persist first, then join
        // readers outside that mutex before native writer/proxy/cache retirement.
        stopReaders()
        return mutex.withLock {
            withContext(Dispatchers.IO) { cleanup.logout() }
        }
    }
}

data class LibraryItem(val id: String, val name: String, val type: String, val overview: String = "", val ticks: Long = 0, val providerLinks: List<Pair<String, String>> = emptyList()) {
    val playable get() = type in setOf("Movie", "Episode", "Audio", "TvChannel")
    val audio get() = type == "Audio"
    companion object {
        fun parse(v: JSONObject) = LibraryItem(v.getString("Id"), v.optString("Name", "Untitled"), v.optString("Type"),
            v.optString("Overview"), v.optJSONObject("UserData")?.optLong("PlaybackPositionTicks") ?: 0,
            v.optJSONArray("ExternalUrls")?.let { links ->
                (0 until minOf(links.length(), 8)).mapNotNull { index ->
                    val link = links.optJSONObject(index) ?: return@mapNotNull null
                    val name = link.optString("Name").take(80)
                    val url = link.optString("Url")
                    if (name.isNotBlank() && safeProviderUrl(url)) name to url else null
                }
            } ?: emptyList())
    }
}
fun JSONObject.items(): List<LibraryItem> {
    val a = optJSONArray("Items") ?: JSONArray()
    return (0 until a.length()).map { LibraryItem.parse(a.getJSONObject(it)) }
}
object Demo {
    val items = listOf(
        LibraryItem("demo-movie", "The glass harbor", "Movie", "An original synthetic catalog entry. No film is included."),
        LibraryItem("demo-series", "Night observatory", "Series", "A synthetic series for browsing seasons and episodes."),
        LibraryItem("demo-episode", "First light", "Episode", "Season 1, episode 1. Synthetic metadata; no episode media."),
        LibraryItem("demo-audio", "Three notes at sea", "Audio", "An original three-second synthesized audio fixture."),
        LibraryItem("demo-channel", "Bluefin test channel", "TvChannel", "Synthetic guide entry. No broadcast stream is included."))
    fun section(name: String, query: String = "") = items.filter {
        when(name) { "Movies" -> it.type == "Movie"; "TV Shows" -> it.type == "Series"; "Episodes" -> it.type == "Episode"; "Music" -> it.audio; "Live TV" -> it.type == "TvChannel"; else -> true }
    }.filter { it.name.contains(query, true) }
}

/** A provider page is an external handoff, never an entitlement or a playback source. */
fun safeProviderUrl(raw: String): Boolean = runCatching {
    val uri = java.net.URI(raw)
    raw.length <= 4096 && raw.none { it.isISOControl() } && uri.scheme == "https" &&
        !uri.host.isNullOrBlank() && uri.userInfo == null && uri.port in listOf(-1, 443) &&
        uri.rawQuery == null && uri.rawFragment == null
}.getOrDefault(false)
