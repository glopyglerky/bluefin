package org.bluefin.android

import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import javax.crypto.KeyGenerator

/** Production coordinator/codec on JVM; no Android Keystore/JNI/device claim. */
class CleanupOwnershipTest {
    private val a = Account("https://fixture.invalid", "owner-a", "token-a", "device-a")
    private val b = a.copy(userId = "owner-b", token = "token-b", deviceId = "device-b")
    private val media = CleanupResource("opaque-proxy", "session-a", "Transcode", "live-a")
    private class Store(initial: CleanupState) : CleanupStore {
        val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
        var envelope = ProtectedCleanupCodec.seal(initial, key)
        var interrupted = false
        var failWrite = false
        var failEnd = false
        var failBegin = false
        var unreadable = false
        var failActive = false
        override fun load(): CleanupState {
            check(!unreadable)
            val value = CleanupState.parse(ProtectedCleanupCodec.open(envelope, key))
            return if (interrupted) value.copy(pending = true) else value
        }
        override fun begin() { check(!failBegin); interrupted = true }
        override fun write(state: CleanupState) { check(!failWrite && !(failActive && !state.pending && state.owner != null)); envelope = ProtectedCleanupCodec.seal(state, key) }
        override fun end() { check(!failEnd); interrupted = false }
    }
    private fun reply(local: Boolean = true, revoked: Boolean = true, release: Boolean = true, resources: List<CleanupResource> = emptyList()) = JSONObject()
        .put("localCleanupComplete", local).put("serverRevoked", revoked).put("playbackReleaseComplete", release).put("cleanupResources", resourceJson(resources))
    @Test fun failedLogoutRetainsInactiveAForAnActualRetry() = runBlocking {
        val storage = Store(CleanupState(a))
        val requests = mutableListOf<Account?>()
        val core = CoreClient(storage) { op, _, owner ->
            assertEquals("androidSignOut", op); requests += owner
            reply(revoked = requests.size > 1)
        }
        assertFalse(core.logout().serverRevoked)
        assertNull("A must be inactive during cleanup", core.account)
        val retry = core.logout()
        assertEquals("Retry must actually use A, not synthesize success from a null active account", listOf(a, a), requests)
        assertTrue(retry.complete)
        assertFalse(core.cleanupPending); assertNull(storage.load().owner)
    }
    @Test fun localDebtBlocksBAndNullActiveAccountCannotEraseIt() = runBlocking {
        val store = Store(CleanupState(a)); var attempts = 0
        val core = CoreClient(store) { op, fields, owner ->
            assertEquals("androidSignOut", op); assertEquals(a, owner)
            if (++attempts > 1) assertTrue(fields.getBoolean("cleanupServerRevoked"))
            reply(local = attempts > 2)
        }
        assertFalse(core.logout().localCleanupComplete); assertNull(core.account)
        assertTrue(runCatching { core.login(b.server, b.userId, "synthetic-password") }.isFailure)
        assertFalse(core.logout().localCleanupComplete); assertTrue(core.cleanupPending)
        assertTrue(core.logout().complete); assertEquals(3, attempts)
    }
    @Test fun failedOwnedReleaseSurvivesRestartAndRetriesOnlyA() = runBlocking {
        val store = Store(CleanupState(a, resources = listOf(media)))
        val first = CoreClient(store) { op, _, owner ->
            assertEquals(a, owner)
            if (op == "releasePlayback") error("Synthetic HTTP 503")
            reply(revoked = false, release = false, resources = listOf(media))
        }
        assertTrue(runCatching { first.callOwned(a, "releasePlayback", media.json()) }.isFailure)
        assertFalse(first.logout().playbackReleaseComplete)
        assertEquals(listOf(media), store.load().resources)
        val retried = mutableListOf<Account?>()
        val second = CoreClient(store) { op, fields, owner ->
            assertEquals("androidSignOut", op); retried += owner
            assertEquals(listOf(media), readResources(fields.getJSONObject("cleanupResources"))); reply()
        }
        assertNull(second.account); assertTrue(second.cleanupPending)
        assertTrue(runCatching { second.callOwned(a, "items") }.isFailure)
        assertTrue(second.logout().complete); assertEquals(listOf(a), retried)
        assertNull(store.load().owner); assertTrue(store.load().resources.isEmpty())
    }
    @Test fun partialOrdinaryReleaseIsProtectedBeforeTheFailureIsReturned() = runBlocking {
        for (progress in listOf(media.copy(encodingReleased = true), media.copy(liveClosed = true),
            media.copy(encodingReleased = true, liveClosed = true))) {
            val store = Store(CleanupState(a, resources = listOf(media)))
            val first = CoreClient(store) { op, _, owner ->
                assertEquals(a, owner); assertEquals("releasePlayback", op)
                decodeNativeReply(op, JSONObject().put("ok", false).put("error", "Synthetic incomplete cleanup")
                    .put("cleanupResources", resourceJson(listOf(progress))))
            }
            assertTrue(runCatching { first.callOwned(a, "releasePlayback", media.json()) }.isFailure)
            assertEquals("A failed release retains the receipt and its independent acknowledgments", listOf(progress), store.load().resources)
            first.beginSignOut()
            val second = CoreClient(store) { op, fields, owner ->
                assertEquals(a, owner); assertEquals("androidSignOut", op)
                assertEquals(listOf(progress), readResources(fields.getJSONObject("cleanupResources")))
                reply()
            }
            assertNull(second.account)
            assertTrue(runCatching { second.login(b.server, b.userId, "synthetic") }.isFailure)
            assertTrue(second.logout().complete)
        }
    }
    @Test fun staticReleaseKeepsMissingAndJsonNullLiveIdentityConsistent() = runBlocking {
        val static = media.copy(method = "DirectPlay", liveId = null, encodingReleased = true, liveClosed = true)
        for (live in listOf(null, JSONObject.NULL, "", "null")) {
            val store = Store(CleanupState(a, resources = listOf(static)))
            val core = CoreClient(store) { op, fields, owner ->
                assertEquals(a, owner); assertEquals("releasePlayback", op)
                assertTrue(fields.isNull("liveStreamId")); JSONObject()
            }
            val fields = static.json()
            if (live == null) fields.remove("liveStreamId") else fields.put("liveStreamId", live)
            core.callOwned(a, "releasePlayback", fields)
            assertTrue(store.load().resources.isEmpty())
        }
    }
    @Test fun partialReleaseWriteFailureKeepsAInactiveAndProgressAvailableForRetry() = runBlocking {
        val progress = media.copy(encodingReleased = true)
        val store = Store(CleanupState(a, resources = listOf(media)))
        val core = CoreClient(store) { op, fields, owner ->
            assertEquals(a, owner)
            if (op == "releasePlayback") return@CoreClient decodeNativeReply(op,
                JSONObject().put("ok", false).put("error", "Synthetic partial release")
                    .put("cleanupResources", resourceJson(listOf(progress))))
            assertEquals("androidSignOut", op)
            assertEquals(listOf(progress), readResources(fields.getJSONObject("cleanupResources")))
            reply()
        }
        store.failWrite = true
        assertTrue(runCatching { core.callOwned(a, "releasePlayback", media.json()) }.isFailure)
        assertNull(core.account); assertTrue(core.cleanupPending)
        assertTrue(runCatching { core.login(b.server, b.userId, "synthetic") }.isFailure)
        store.failWrite = false
        assertTrue(core.logout().complete)
    }
    @Test fun remoteOnlyDebtAlsoBlocksNewAdmissionUntilConfirmed() = runBlocking {
        val store = Store(CleanupState(a, pending = true, report = SignOutReport(true, false, true)))
        val owners = mutableListOf<Account?>()
        val core = CoreClient(store) { op, fields, owner ->
            owners += owner
            if (op == "androidSignOut") reply() else JSONObject().put("User", JSONObject().put("Id", b.userId)).put("AccessToken", b.token).also { assertEquals(b.userId, fields.getString("username")) }
        }
        assertTrue(runCatching { core.login(b.server, b.userId, "synthetic") }.isFailure)
        assertTrue(owners.isEmpty()); assertTrue(core.logout().complete)
        core.login(b.server, b.userId, "synthetic"); assertEquals(b.userId, core.account!!.userId)
        assertEquals(listOf(a, null), owners)
    }
    @Test fun writeFailureInactivatesAAndDurableMarkerPreventsActiveReload() = runBlocking {
        val store = Store(CleanupState(a)); var calls = 0
        val core = CoreClient(store) { _, _, owner -> calls++; assertEquals(a, owner); reply() }
        store.failWrite = true
        assertTrue(runCatching { core.logout() }.isFailure)
        assertNull(core.account); assertTrue(core.cleanupPending); assertEquals(0, calls)
        val restarted = CoreClient(store) { _, _, owner -> calls++; assertEquals(a, owner); reply() }
        assertNull(restarted.account); assertTrue(restarted.cleanupPending)
        store.failWrite = false; assertTrue(restarted.logout().complete); assertEquals(1, calls)
    }
    @Test fun clearingFailureKeepsConfirmedRevocationAcrossReload() = runBlocking {
        val store = Store(CleanupState(a)); var calls = 0
        val core = CoreClient(store) { _, _, _ -> calls++; reply() }
        store.failEnd = true
        assertFalse(core.logout().complete); assertNull(core.account); assertTrue(core.cleanupPending)
        val restarted = CoreClient(store) { _, _, _ -> error("No remote request for settled tombstone") }
        assertNull(restarted.account); assertTrue(restarted.cleanupPending)
        store.failEnd = false
        assertTrue(restarted.logout().complete); assertFalse(restarted.cleanupPending); assertEquals(1, calls)
    }
    @Test fun loginSecureSaveFailurePreservesIssuedAuthorityRatherThanSwallowingDebt() = runBlocking {
        val store = Store(CleanupState()).apply { failActive = true }
        var issued: Account? = null; var cleanups = 0
        val core = CoreClient(store) { op, fields, owner ->
            if (op == "login") {
                issued = a.copy(deviceId = fields.getString("deviceId"))
                JSONObject().put("User", JSONObject().put("Id", a.userId)).put("AccessToken", a.token)
            } else { assertEquals(issued, owner); cleanups++; reply(revoked = cleanups > 1) }
        }
        assertTrue(runCatching { core.login(a.server, "synthetic-user", "synthetic-password") }.isFailure)
        assertNull(core.account); assertTrue(core.cleanupPending)
        assertEquals(issued, store.load().owner); assertTrue(store.load().pending)
        assertTrue(core.logout().complete); assertEquals(2, cleanups)
    }
    @Test fun completelyUnavailableSecureWritesRetainMemoryOwnerAndFailClosedOnReload() = runBlocking {
        val store = Store(CleanupState()); var issued: Account? = null; var calls = 0
        val core = CoreClient(store) { op, fields, owner ->
            calls++
            if (op == "login") {
                issued = a.copy(deviceId = fields.getString("deviceId")); store.failWrite = true
                JSONObject().put("User", JSONObject().put("Id", a.userId)).put("AccessToken", a.token)
            } else { assertEquals(issued, owner); reply(revoked = false) }
        }
        assertTrue(runCatching { core.login(a.server, "synthetic", "synthetic") }.isFailure)
        assertNull(core.account); assertTrue(core.cleanupPending)
        val restarted = CoreClient(store) { _, _, _ -> error("No lost-authority success") }
        assertNull(restarted.account); assertTrue(runCatching { restarted.logout() }.isFailure)
        store.failWrite = false
        assertFalse(core.logout().serverRevoked); assertEquals(3, calls)
    }
    @Test fun unreadableProtectedStateIsNeverReplacedWithEmptySuccess() = runBlocking {
        val store = Store(CleanupState(a, pending = true, resources = listOf(media))).apply { unreadable = true }
        val before = store.envelope
        val core = CoreClient(store) { _, _, _ -> reply() }
        assertNull(core.account); assertTrue(core.cleanupPending)
        assertTrue(runCatching { core.logout() }.isFailure); assertEquals(before, store.envelope)
        store.unreadable = false; assertTrue(core.logout().complete)
    }
    @Test fun capturedAuthorityCannotBeReplacedByCallerFieldsOrAnotherOwner() = runBlocking {
        val core = CoreClient(Store(CleanupState(a))) { _, _, owner -> assertEquals(a, owner); JSONObject() }
        assertTrue(runCatching { core.callOwned(b, "items") }.isFailure)
        for (field in listOf("server", "token", "userId", "deviceId", "cleanupResources", "cleanupServerRevoked")) {
            assertTrue(runCatching { core.callOwned(a, "items", JSONObject().put(field, "replacement")) }.isFailure)
        }
        core.callOwned(a, "items"); Unit
    }
    @Test fun playbackReceiptPersistsBeforeAdmissionAndOnlySuccessfulReleaseRemovesIt() = runBlocking {
        val store = Store(CleanupState(a))
        val core = CoreClient(store) { op, _, _ ->
            if (op == "playback") JSONObject().put("proxyId", media.proxyId).put("playSessionId", media.sessionId)
                .put("playMethod", media.method).put("source", JSONObject().put("LiveStreamId", media.liveId)) else JSONObject()
        }
        core.callOwned(a, "playback"); assertEquals(listOf(media), store.load().resources); assertEquals(a, core.account)
        core.callOwned(a, "releasePlayback", media.json()); assertTrue(store.load().resources.isEmpty())
    }
    @Test fun interruptedPreparationDoesNotClaimUnknownProducerCleanupComplete() = runBlocking {
        val store = Store(CleanupState(a))
        val core = CoreClient(store) { op, _, _ -> if (op == "playback") error("Lost preparation reply") else reply() }
        assertTrue(runCatching { core.callOwned(a, "playback") }.isFailure)
        val restarted = CoreClient(store) { _, _, owner -> assertEquals(a, owner); reply() }
        assertNull(restarted.account); assertFalse(restarted.logout().playbackReleaseComplete); assertTrue(restarted.cleanupPending)
    }
    @Test fun protectedCodecEncryptsAuthenticatesAndBoundsPendingState() {
        val store = Store(CleanupState(a, pending = true, resources = listOf(media)))
        assertFalse(store.envelope.body.contains(a.token)); assertFalse(store.envelope.body.contains(a.server)); assertEquals(a, store.load().owner)
        val bytes = java.util.Base64.getDecoder().decode(store.envelope.body); bytes[0] = (bytes[0].toInt() xor 1).toByte()
        val tampered = store.envelope.copy(body = java.util.Base64.getEncoder().encodeToString(bytes))
        assertTrue(runCatching { ProtectedCleanupCodec.open(tampered, store.key) }.isFailure)
        assertTrue(runCatching { CleanupState.parse(CleanupState(a).json().put("version", 999).toString()) }.isFailure)
        assertTrue(runCatching { readResources(resourceJson(List(33) { media.copy(proxyId = "id-$it") })) }.isFailure)
        assertTrue(runCatching { readResources(resourceJson(listOf(media, media))) }.isFailure)
        assertTrue(runCatching { CleanupResource.parse(media.json().put("playSessionId", "../arbitrary?route")) }.isFailure)
        assertTrue(runCatching { CleanupResource.parse(media.json().put("cleanupUrl", "https://other.invalid")) }.isFailure)
    }
    @Test fun readerJoinPrecedesNativeRetirementAndFailedJoinKeepsAInactive() = runBlocking {
        val store = Store(CleanupState(a))
        var failJoin = true; var nativeCalls = 0
        lateinit var core: CoreClient
        core = CoreClient(store, stopReaders = {
            assertNull(core.account); assertTrue(core.cleanupPending)
            check(!failJoin) { "Synthetic reader join failed" }
        }) { _, _, owner -> nativeCalls++; assertEquals(a, owner); reply() }
        assertTrue(runCatching { core.logout() }.isFailure)
        assertEquals(0, nativeCalls); assertNull(core.account); assertTrue(core.cleanupPending)
        assertTrue(runCatching { core.login(b.server, b.userId, "synthetic") }.isFailure)
        failJoin = false; assertTrue(core.logout().complete); assertEquals(1, nativeCalls)
    }
    @Test fun admissionStaysBlockedWhileReadersAreStillJoining() = runBlocking {
        val entered = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
        val store = Store(CleanupState(a)); var nativeCalls = 0
        val core = CoreClient(store, stopReaders = { entered.complete(Unit); finish.await() }) { _, _, owner -> nativeCalls++; assertEquals(a, owner); reply() }
        val cleanup = launch { assertTrue(core.logout().complete) }
        entered.await(); assertNull(core.account); assertEquals(0, nativeCalls)
        assertTrue(runCatching { core.login(b.server, b.userId, "synthetic") }.isFailure)
        finish.complete(Unit); cleanup.join(); assertEquals(1, nativeCalls)
    }
    @Test fun markerFailureUsesEncryptedPendingFallbackAndNeverAdmitsB() = runBlocking {
        val store = Store(CleanupState(a)).apply { failBegin = true }
        val core = CoreClient(store) { _, _, _ -> error("No native work before protected custody") }
        assertTrue(runCatching { core.logout() }.isFailure); assertNull(core.account)
        assertTrue(store.load().pending); assertEquals(a, store.load().owner)
        val restored = CoreClient(store) { _, _, owner -> assertEquals(a, owner); reply() }
        assertNull(restored.account); store.failBegin = false; assertTrue(restored.logout().complete)
    }
    @Test fun explicitLoginRejectionCanRetryButUnknownLoginOutcomeCannotBecomeSuccess() = runBlocking {
        val rejected = CoreClient(Store(CleanupState())) { _, _, _ -> error("Server returned HTTP 401") }
        assertTrue(runCatching { rejected.login(a.server, "synthetic", "wrong-password") }.isFailure)
        assertFalse(rejected.cleanupPending); assertNull(rejected.account)
        assertTrue(rejected.logout().complete)
        val store = Store(CleanupState())
        val unknown = CoreClient(store) { _, _, _ -> error("Server request failed or timed out") }
        assertTrue(runCatching { unknown.login(a.server, "synthetic", "synthetic") }.isFailure)
        assertTrue(unknown.cleanupPending)
        val restored = CoreClient(store) { _, _, _ -> error("No fabricated cleanup") }
        assertNull(restored.account); assertTrue(restored.cleanupPending)
        assertTrue(runCatching { restored.logout() }.isFailure)
    }
}
