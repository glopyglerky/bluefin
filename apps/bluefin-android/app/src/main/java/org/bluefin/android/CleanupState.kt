package org.bluefin.android

import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.util.UUID

private fun opaque(value: String, max: Int = 128): String {
    require(value.length in 1..max && value.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "-_." }) { "Invalid protected cleanup identifier" }
    return value
}

internal fun Account.validate(): Account {
    val uri = URI(server)
    val path = (uri.rawPath ?: "").lowercase()
    require(server.length <= 4096 && server.none { it.isISOControl() || it == '\\' } &&
        uri.scheme in listOf("https", "http") && !uri.host.isNullOrBlank() && uri.userInfo == null &&
        uri.rawQuery == null && uri.rawFragment == null && listOf("%2e", "%2f", "%5c").none { it in path }) { "Invalid protected server" }
    // Native's existing environment-specific HTTPS/local-HTTP policy still applies.
    opaque(userId); opaque(deviceId); opaque(token, 4096)
    return this
}

internal data class CleanupResource(val proxyId: String, val sessionId: String, val method: String, val liveId: String?,
    val encodingReleased: Boolean = method != "Transcode", val liveClosed: Boolean = liveId == null) {
    val remoteReleased get() = encodingReleased && liveClosed
    fun json() = JSONObject().put("proxyId", proxyId).put("playSessionId", sessionId).put("playMethod", method).put("liveStreamId", liveId ?: JSONObject.NULL)
        .put("remoteReleased", remoteReleased).put("encodingReleased", encodingReleased).put("liveClosed", liveClosed)
    companion object {
        fun parse(v: JSONObject): CleanupResource {
            require(v.keys().asSequence().all { it in setOf("proxyId", "playSessionId", "playMethod", "liveStreamId", "remoteReleased", "encodingReleased", "liveClosed") }) { "Invalid cleanup receipt" }
            val method = v.getString("playMethod")
            require(method in setOf("DirectPlay", "DirectStream", "Transcode")) { "Invalid cleanup method" }
            val live = if (v.isNull("liveStreamId")) null else opaque(v.getString("liveStreamId"))
            val remote = v.getBoolean("remoteReleased")
            val encoding = if (v.has("encodingReleased")) v.getBoolean("encodingReleased") else remote || method != "Transcode"
            val closed = if (v.has("liveClosed")) v.getBoolean("liveClosed") else remote || live == null
            require(!remote || encoding && closed) { "Inconsistent remote receipt" }
            return CleanupResource(opaque(v.getString("proxyId")), opaque(v.getString("playSessionId")), method, live, encoding, closed)
        }
        fun plan(v: JSONObject) = parse(JSONObject().put("proxyId", v.getString("proxyId"))
            .put("playSessionId", v.getString("playSessionId")).put("playMethod", v.getString("playMethod"))
            .put("liveStreamId", v.optJSONObject("source")?.optString("LiveStreamId")?.takeIf { it.isNotEmpty() && it != "null" } ?: JSONObject.NULL).put("remoteReleased", false))
    }
}

internal fun resourceJson(resources: List<CleanupResource>) = JSONObject().put("version", 1)
    .put("resources", JSONArray().also { a -> resources.forEach { a.put(it.json()) } })
internal fun readResources(v: JSONObject): List<CleanupResource> {
    require(v.getInt("version") == 1 && v.keys().asSequence().all { it in setOf("version", "resources") }) { "Unsupported cleanup state" }
    val a = v.getJSONArray("resources")
    require(a.length() <= 32) { "Too many pending resources" }
    val list = (0 until a.length()).map { CleanupResource.parse(a.getJSONObject(it)) }
    require(list.map { it.proxyId }.distinct().size == list.size) { "Duplicate pending resource" }
    return list
}

/** Confirmed native progress accompanying an unsuccessful release operation. */
internal class NativeCleanupFailure(message: String, val resources: List<CleanupResource>) : IllegalStateException(message)

/** One owner at most. The pending owner is never exposed as active authentication. */
internal data class CleanupState(val owner: Account? = null, val pending: Boolean = false,
    val resources: List<CleanupResource> = emptyList(), val report: SignOutReport = SignOutReport(false, false, false),
    val uncertainPlayback: Boolean = false) {
    fun json(): JSONObject = JSONObject().put("version", 2).put("owner", owner?.json() ?: JSONObject.NULL)
        .put("pending", pending).put("cleanupResources", resourceJson(resources)).put("uncertainPlayback", uncertainPlayback)
        .put("localCleanupComplete", report.localCleanupComplete).put("serverRevoked", report.serverRevoked)
        .put("playbackReleaseComplete", report.playbackReleaseComplete)
    companion object {
        const val MAX_BYTES = 32768
        fun parse(encoded: String): CleanupState {
            require(encoded.toByteArray(Charsets.UTF_8).size <= MAX_BYTES) { "Protected state too large" }
            val v = JSONObject(encoded)
            require(v.getInt("version") == 2) { "Unsupported protected state" }
            require(v.keys().asSequence().toSet() == setOf("version", "owner", "pending", "cleanupResources", "uncertainPlayback", "localCleanupComplete", "serverRevoked", "playbackReleaseComplete")) { "Invalid protected schema" }
            val owner = v.optJSONObject("owner")?.let { Account(it.getString("server"), it.getString("userId"), it.getString("token"), it.getString("deviceId")).validate() }
            val state = CleanupState(owner, v.getBoolean("pending"), readResources(v.getJSONObject("cleanupResources")),
                SignOutReport(v.getBoolean("localCleanupComplete"), v.getBoolean("serverRevoked"), v.getBoolean("playbackReleaseComplete")), v.getBoolean("uncertainPlayback"))
            require(owner != null || state.resources.isEmpty() && !state.uncertainPlayback) { "Missing cleanup authority" }
            return state
        }
    }
}

internal interface CleanupStore {
    fun load(): CleanupState
    /** Durable, non-secret interruption marker, independent of encrypted preference commits. */
    fun begin()
    fun write(state: CleanupState)
    fun end()
}

/** Production state transitions. CoreClient serializes these with all native calls. */
internal class CleanupCoordinator(private val store: CleanupStore,
    private val request: (String, JSONObject, Account?) -> JSONObject) {
    private var state = CleanupState()
    private var unreadable = false
    private var storageFailed = false
    init { try { state = store.load() } catch (_: Exception) { unreadable = true; state = CleanupState(pending = true) } }
    val account: Account? get() = state.owner.takeUnless { state.pending || unreadable || storageFailed }
    val cleanupPending get() = state.pending || unreadable || storageFailed
    val cleanupMessage: String get() = when {
        unreadable || storageFailed -> "Sign-out state could not be stored securely. Retry finishing sign out."
        state.owner == null && state.pending && !state.report.complete -> "The sign-in outcome is unconfirmed. Keep this installation signed out until its protected cleanup authority can be recovered."
        state.uncertainPlayback -> "Playback cleanup could not be confirmed. Keep this account signed out and retry cleanup."
        !state.report.localCleanupComplete -> "Finish sign out before connecting another account. Retry local cleanup."
        else -> "Signed out locally. Retry server cleanup before connecting another account."
    }
    private fun persist(value: CleanupState, finish: Boolean = false) {
        try {
            store.begin(); store.write(value); if (finish) store.end()
            storageFailed = false
        } catch (failure: Exception) {
            storageFailed = true
            // If the independent marker fails, an encrypted pending write can
            // still prevent an active reload. If both stores fail, retain the
            // owner in memory and report failure; never claim durable custody.
            runCatching { store.write(value.copy(pending = true)) }
            throw IllegalStateException("Sign-out state could not be stored securely. Retry finishing sign out.", failure)
        }
    }
    fun login(server: String, username: String, password: String) {
        val device = UUID.randomUUID().toString()
        loginUsing(server, device) {
            request("login", JSONObject().put("server", server).put("username", username)
                .put("password", password).put("deviceId", device), null)
        }
    }

    fun loginUsing(server: String, device: String, exchange: () -> JSONObject): Boolean {
        check(account == null && !cleanupPending) { "Finish sign out before connecting another account." }
        // A crash after a login may have issued a session: an interrupted empty
        // journal is held, never interpreted as permission to admit another user.
        state = CleanupState(pending = true)
        try { persist(state) } catch (failure: Exception) {
            // No network request was made: this process can finish that empty
            // intent once storage recovers. A restarted unknown intent stays held.
            state = CleanupState(pending = true, report = SignOutReport(true, true, true))
            throw failure
        }
        val reply = try {
            exchange()
        } catch (failure: Exception) {
            // These exact statuses are generated by native response_json from
            // a fresh server response, not from a server-supplied error body.
            if (failure.message in setOf("Server returned HTTP 401", "Server returned HTTP 403")) {
                state = CleanupState(pending = true, report = SignOutReport(true, true, true))
                val settled = state.copy(pending = false)
                try { persist(settled, finish = true); state = settled } catch (_: Exception) { /* Retain the empty pending receipt for retry. */ }
            }
            throw failure
        }
        if (reply.optBoolean("pending")) {
            state = CleanupState(pending = true, report = SignOutReport(true, true, true))
            val settled = state.copy(pending = false)
            persist(settled, finish = true)
            state = settled
            return false
        }
        val next = Account(server, reply.getJSONObject("User").getString("Id"), reply.getString("AccessToken"), device).validate()
        state = CleanupState(next, pending = true)
        try {
            persist(state.copy(pending = false), finish = true)
            state = state.copy(pending = false)
        } catch (failure: Exception) {
            // Keep the actual issued authority in memory even when secure writes
            // fail. The interruption marker prevents an active reload of old data.
            runCatching { cleanupAttempt(persistFirst = false) }
            throw IllegalStateException("Account could not be stored securely. Finish sign out before signing in again.", failure)
        }
        return true
    }
    fun call(operation: String, fields: JSONObject, captured: Account? = account): JSONObject {
        check(captured == account) { "Account changed. Retry from the current library." }
        check(!cleanupPending) { "Finish sign out before using an account." }
        require(operation !in setOf("login", "androidSignOut")) { "Use the account sign-in or sign-out action" }
        val owner = account
        // UI fields cannot replace captured authentication or supply restoration debt.
        require(fields.keys().asSequence().none { it in setOf("token", "userId", "deviceId", "server", "cleanupResources", "cleanupServerRevoked") }) { "Captured account fields cannot be replaced" }
        if (operation == "playback" && owner != null) {
            check(state.resources.size < 32) { "Finish sign out to release pending playback." }
            state = state.copy(pending = true, uncertainPlayback = true)
            persist(state)
            val result = request(operation, fields, owner)
            val receipt = CleanupResource.plan(result)
            val next = state.copy(pending = false, uncertainPlayback = false, resources = state.resources + receipt)
            try { persist(next, finish = true); state = next }
            catch (failure: Exception) {
                state = next.copy(pending = true)
                throw failure
            }
            return result
        }
        // Android JSONObject can stringify JSON null as "null". Cleanup must
        // retain the same optional live identity as the admitted receipt.
        val callFields = if (operation == "releasePlayback") JSONObject(fields.toString()).apply {
            put("liveStreamId", fields.optString("liveStreamId").takeIf { it.isNotEmpty() && it != "null" } ?: JSONObject.NULL)
        } else fields
        val result = try { request(operation, callFields, owner) }
        catch (failure: NativeCleanupFailure) {
            if (operation == "releasePlayback" && owner != null) {
                val progress = failure.resources.associateBy { it.proxyId }
                val retained = state.resources.map { current ->
                    val update = progress[current.proxyId] ?: return@map current
                    require(current.sessionId == update.sessionId && current.method == update.method && current.liveId == update.liveId) { "Cleanup progress does not match captured resource" }
                    current.copy(encodingReleased = current.encodingReleased || update.encodingReleased,
                        liveClosed = current.liveClosed || update.liveClosed)
                }
                val next = state.copy(resources = retained)
                state = next
                try { persist(next, finish = true) }
                catch (storage: Exception) { state = next.copy(pending = true); throw storage }
            }
            throw failure
        }
        if (operation == "releasePlayback" && owner != null) {
            val retained = state.resources.filterNot { it.sessionId == callFields.optString("playSessionId") && it.method == callFields.optString("playMethod") && it.liveId == callFields.optString("liveStreamId").takeIf { live -> live.isNotEmpty() && live != "null" } }
            if (retained != state.resources) {
                val next = state.copy(resources = retained)
                try { persist(next, finish = true); state = next }
                catch (failure: Exception) { state = state.copy(pending = true); throw failure }
            }
        }
        return result
    }
    fun beginSignOut() {
        if (unreadable) {
            // Never overwrite unreadable ciphertext with an empty success record.
            state = store.load().copy(pending = true); unreadable = false
        }
        if (state.owner == null) {
            if (state.pending && state.report.complete) {
                val settled = CleanupState(report = state.report)
                persist(settled, finish = true); state = settled
                return
            }
            check(!cleanupPending) { "Protected cleanup authority is unavailable. Retry secure storage; keep this installation signed out." }
            return
        }
        state = state.copy(pending = true)
        persist(state)
    }
    fun logout(): SignOutReport {
        beginSignOut()
        if (state.owner == null) return SignOutReport(true, true, true)
        return cleanupAttempt(persistFirst = true)
    }
    private fun cleanupAttempt(persistFirst: Boolean): SignOutReport {
        if (persistFirst) persist(state)
        val owner = state.owner ?: error("Missing protected cleanup authority")
        val value = try {
            request("androidSignOut", JSONObject().put("cleanupResources", resourceJson(state.resources))
                .put("cleanupServerRevoked", state.report.serverRevoked), owner)
        } catch (_: Exception) {
            // No reply is no acknowledgement. Keep the exact prior debt.
            persist(state)
            return state.report.copy(protectedStateComplete = false)
        }
        val resources = readResources(value.getJSONObject("cleanupResources"))
        val report = SignOutReport(value.getBoolean("localCleanupComplete"), state.report.serverRevoked || value.getBoolean("serverRevoked"),
            value.getBoolean("playbackReleaseComplete") && !state.uncertainPlayback)
        state = state.copy(resources = resources, report = report)
        // A completed remote report cannot discard outstanding local resources.
        val finished = report.complete && resources.isEmpty()
        try {
            val settled = CleanupState(report = report)
            persist(if (finished) settled else state, finish = finished)
            if (finished) state = settled
        } catch (_: Exception) { return report.copy(protectedStateComplete = false) }
        return report.copy(protectedStateComplete = !report.complete || finished)
    }
}
