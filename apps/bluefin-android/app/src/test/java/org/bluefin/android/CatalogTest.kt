package org.bluefin.android
import org.junit.Assert.*
import org.junit.Test
import org.json.JSONObject
class CatalogTest {
    @Test fun cleanupRequiresEveryIndependentOutcome() {
        for (local in listOf(false, true)) for (revoked in listOf(false, true)) for (playback in listOf(false, true)) {
            val report = SignOutReport(local, revoked, playback)
            assertEquals(local && revoked && playback, report.complete)
            assertEquals(local, report.localCleanupComplete)
            assertEquals(revoked, report.serverRevoked)
            assertEquals(playback, report.playbackReleaseComplete)
        }
    }
    @Test fun providerHandoffRejectsCredentialsAndNonHttpsRoutes() {
        assertTrue(safeProviderUrl("https://example.com/title/one"))
        listOf("javascript:alert(1)", "http://example.com/", "https://secret@example.com/", "https://example.com/?token=secret", "file:///private/data").forEach { assertFalse(safeProviderUrl(it)) }
    }
    @Test fun searchAndTypeFiltersKeepDistinctMediaKinds() {
        assertEquals(listOf("Audio"), Demo.section("Music").map { it.type })
        assertEquals(listOf("TvChannel"), Demo.section("Live TV").map { it.type })
        assertEquals(listOf("demo-episode"), Demo.section("Search", "FIRST LIGHT").map { it.id })
        assertTrue(Demo.section("Movies", "missing").isEmpty())
    }
    @Test fun resumeAndAccountScopesDoNotCrossUsersOrServers() {
        val item = LibraryItem.parse(JSONObject("""{"Id":"movie","Name":"Harbor","Type":"Movie","UserData":{"PlaybackPositionTicks":90000000}}"""))
        assertTrue(item.playable); assertEquals(90000000L, item.ticks)
        val a = Account("https://example.invalid", "one", "a", "device")
        assertNotEquals(a.scope, a.copy(userId="two").scope)
        assertNotEquals(a.scope, a.copy(server="https://other.invalid").scope)
        assertEquals(a.scope, a.copy(token="rotated").scope)
        assertFalse(LibraryItem("series", "Series", "Series").playable)
    }
}
