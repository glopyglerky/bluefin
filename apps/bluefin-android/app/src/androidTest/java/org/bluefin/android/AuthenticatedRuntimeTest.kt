package org.bluefin.android

import android.content.ComponentName
import androidx.activity.compose.setContent
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.TimeUnit

/** Original loopback HTTP fixture; actual account capture, JNI, CoreClient and MusicService. */
class AuthenticatedRuntimeTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext
    private val core get() = BluefinRuntime.core(context)
    private var offlineWindowOpened = false
    private fun main(block: () -> Unit) = instrumentation.runOnMainSync(block)
    private fun controller(): MediaController {
        lateinit var f: com.google.common.util.concurrent.ListenableFuture<MediaController>
        main { f = MediaController.Builder(context, SessionToken(context, ComponentName(context, MusicService::class.java))).buildAsync() }
        return f.get(15, TimeUnit.SECONDS)
    }
    private fun await(message: String, seconds: Int = 15, condition: () -> Boolean) {
        val end = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds.toLong())
        while (System.nanoTime() < end) { var ok = false; main { ok = condition() }; if (ok) return; Thread.sleep(50) }
        fail(message)
    }
    private fun signedIn(block: () -> Unit) {
        assertNull("Empty synthetic installation required", core.account)
        CoreRequestEnvironment.loopbackFixture = true
        var failure: Throwable? = null
        try {
            runBlocking { core.login("http://127.0.0.1:18765", "original-fixture-user", "original-fixture-password") }
            assertEquals("fixture-user", core.account!!.userId)
            block()
        } catch (error: Throwable) {
            failure = error
            throw error
        } finally {
            try {
                // End the bounded network outage before remote sign-out. Test
                // ordering must not strand cleanup authority for later cases.
                if (offlineWindowOpened) restoreFixtureTransport()
                runBlocking { MusicService.stopAndJoin() }
                if (core.account != null || core.cleanupPending) {
                    val report = runBlocking { core.logout() }
                    assertTrue("Synthetic sign-out must finish: $report", report.complete)
                }
                assertFalse("Synthetic sign-out left pending cleanup", core.cleanupPending)
            } catch (cleanup: Throwable) {
                if (failure == null) throw cleanup
                failure.addSuppressed(cleanup)
            } finally {
                CoreRequestEnvironment.loopbackFixture = false
            }
        }
    }
    private fun fixtureReachable(): Boolean = runCatching {
        val probe = java.net.URL("http://127.0.0.1:18765/").openConnection() as java.net.HttpURLConnection
        probe.connectTimeout = 500; probe.readTimeout = 500
        // A denial proves connectivity, never authentication or entitlement.
        try { check(probe.responseCode > 0) } finally { probe.disconnect() }
    }.isSuccess
    private fun restoreFixtureTransport() {
        println("OFFLINE_WINDOW_COMPLETE")
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        while (!fixtureReachable() && System.nanoTime() < deadline) Thread.sleep(100)
        assertTrue("Host must restore the fixture route before sign-out", fixtureReachable())
        offlineWindowOpened = false
    }
    @Test fun authenticatedServiceDecodesSeeksAdvancesAndReports() = signedIn {
        val c = controller()
        try {
            val item = LibraryItem("song", "Authenticated original audio", "Audio")
            main { MusicService.current()!!.play(listOf(item, item.copy(id="second", name="Second authenticated")), false, null) }
            await("Authenticated decoded playback time") { c.isPlaying && c.currentPosition > 500 }
            main { c.pause(); c.seekTo(1200) }
            await("Authenticated seek acknowledgement") { c.currentPosition >= 1200 }
            main { c.seekToNext() }
            await("Captured-owner next") { c.mediaMetadata.title.toString() == "Second authenticated" && c.isPlaying }
            main { c.seekToPrevious() }
            await("Captured-owner previous") { c.mediaMetadata.title.toString() == item.name && c.isPlaying }
            main { c.seekTo(c.duration.coerceAtLeast(1000) - 300) }
            await("Automatic advance") { c.mediaMetadata.title.toString() == "Second authenticated" && c.isPlaying }
        } finally { main { c.release() } }
    }
    @Test fun completedDownloadVerifiedPlayedWithoutNetworkThenRemoved() = signedIn {
        val owner = core.account!!
        runBlocking { core.callOwned(owner, "downloadStart", JSONObject().put("itemId","song").put("maxBytes", 16000000).put("quotaBytes",32000000)) }
        var record = JSONObject()
        val end = System.nanoTime() + TimeUnit.SECONDS.toNanos(20)
        while (System.nanoTime() < end) {
            val rows = runBlocking { core.callOwned(owner,"downloads") }.getJSONArray("Items")
            if (rows.length()>0) record=rows.getJSONObject(0)
            if (record.optString("state")=="completed") break
            Thread.sleep(100)
        }
        assertEquals(record.toString(), "completed", record.optString("state"))
        val file = java.io.File(java.net.URI(record.getString("localUrl")))
        assertTrue(file.length()>44)
        // Host test runner removes adb reverse for the explicit offline window.
        offlineWindowOpened = true
        println("OFFLINE_WINDOW_READY")
        val offlineDeadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        var unavailable = false
        while (!unavailable && System.nanoTime() < offlineDeadline) {
            unavailable = !fixtureReachable()
            if (!unavailable) Thread.sleep(100)
        }
        assertTrue("Host must remove the fixture reverse before local playback", unavailable)
        val c=controller()
        try {
            main { MusicService.current()!!.play(listOf(LibraryItem("song","Verified offline original","Audio")),true,null) }
            await("Verified local Media3 decode") { c.isPlaying && c.currentPosition > 500 }
            main { c.pause(); c.seekTo(1200) }
            await("Offline local seek acknowledgement") { c.currentPosition >= 1200 }
            main { c.play() }
            await("Offline local playback after seek") { c.isPlaying && c.currentPosition > 1500 }
        } finally { runBlocking(Dispatchers.Main) { MusicService.stopAndJoin() }; main { c.release() } }
        runBlocking { core.callOwned(owner,"downloadCancel",JSONObject().put("itemId","song")); core.callOwned(owner,"downloadRemove",JSONObject().put("itemId","song")) }
        assertFalse(file.exists())
        assertEquals(0,runBlocking { core.callOwned(owner,"downloads") }.getJSONArray("Items").length())
    }
    @Test fun delayedPreparationBeyondFiveSecondsAndCapturedCleanup() = signedIn {
        val c=controller()
        try {
            val start=System.nanoTime()
            main { MusicService.current()!!.play(listOf(LibraryItem("delayed","Delayed authenticated","Audio")),false,null) }
            Thread.sleep(5500)
            main { assertFalse("Must still be server-preparing",c.isPlaying) }
            await("Delayed actual server preparation must eventually decode") { c.isPlaying && c.currentPosition>250 }
            assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime()-start)>6500)
        } finally { runBlocking(Dispatchers.Main) { MusicService.stopAndJoin() }; main { c.release() } }
    }
    @Test fun productionScreenBackgroundDuringPreparation() = productionPreparation(false)
    @Test fun productionNotificationCancelsPendingPreparation() = productionPreparation(true)
    private fun productionPreparation(cancel: Boolean) = signedIn {
        var activity: MainActivity? = null
        await("Open Bluefin through T3 after instrumentation startup", 30) {
            activity = androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).filterIsInstance<MainActivity>().singleOrNull()
            activity != null
        }
        val c = controller()
        try {
            main {
                val model = LibraryModel(context.applicationContext as android.app.Application)
                activity!!.setContent {
                    PlaybackScreen(LibraryItem("delayed", "Delayed production screen", "Audio"),
                        model) { }
                }
            }
            await("Production screen must promote service during native preparation", 3) {
                (context.getSystemService(android.content.Context.ACTIVITY_SERVICE) as android.app.ActivityManager)
                    .getRunningServices(30).any { it.service.className == MusicService::class.java.name && it.foreground }
            }
            println("BACKGROUND_PREPARATION_READY")
            await("Move Bluefin Home during actual preparation through T3", 5) {
                androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                    .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).isEmpty()
            }
            main { assertFalse("Still native preparing at background transition", c.isPlaying) }
            if (cancel) {
                println("PREPARING_NOTIFICATION_STOP_READY")
                await("Press the preparing notification Stop through T3", 5) { MusicService.current()?.status?.startsWith("Stopping") == true }
                main { assertFalse(c.isPlaying) }
                runBlocking(Dispatchers.Main) { MusicService.stopAndJoin() }
                main { assertFalse("No playback after native join", c.isPlaying) }
            } else {
                await("Background preparation eventually plays with audio focus", 15) { c.isPlaying && c.currentPosition > 500 }
                Thread.sleep(1000)
            }
        } finally {
            runBlocking(Dispatchers.Main) { MusicService.stopAndJoin() }
            main { c.release(); activity?.setContent { } }
        }
        await("Only owned playback notifications must be gone after cleanup") {
            context.getSystemService(android.app.NotificationManager::class.java).activeNotifications.isEmpty()
        }
    }
    @Test fun cancelledDownloadCannotPublishAndSiblingAccountIsUntouched() = signedIn {
        val owner = core.account!!
        runBlocking { core.callOwned(owner,"downloadStart",JSONObject().put("itemId","slow").put("maxBytes",16000000).put("quotaBytes",32000000)) }
        Thread.sleep(500)
        runBlocking { core.callOwned(owner,"downloadCancel",JSONObject().put("itemId","slow")) }
        val rows = runBlocking { core.callOwned(owner,"downloads") }.getJSONArray("Items")
        assertEquals(1, rows.length())
        assertEquals("cancelled", rows.getJSONObject(0).getString("state"))
        assertFalse(rows.getJSONObject(0).has("localUrl"))
        runBlocking { core.callOwned(owner,"downloadRemove",JSONObject().put("itemId","slow")) }
        assertEquals(0,runBlocking { core.callOwned(owner,"downloads") }.getJSONArray("Items").length())
        // Actual separate authenticated namespace, without sharing credentials in source or Intents.
        val request=JSONObject().put("operation","downloads").put("clientPlatform","android")
            .put("server",owner.server).put("userId","sibling").put("token","synthetic")
            .put("deviceId","synthetic").put("allowLocalHttp",true)
            .put("cacheRoot",java.io.File(context.noBackupFilesDir.canonicalFile,"offline").absolutePath)
        assertTrue(JSONObject(NativeCore.invoke(request.toString())).getBoolean("ok"))
    }
    @Test fun stopWhilePreparingDoesNotStartLatePlayback() = signedIn {
        val c=controller()
        try {
            main { MusicService.current()!!.play(listOf(LibraryItem("delayed","Cancelled preparation","Audio")),false,null) }
            Thread.sleep(1000)
            val start=System.nanoTime()
            main { MusicService.requestStop() }
            val elapsed=TimeUnit.NANOSECONDS.toMillis(System.nanoTime()-start)
            main { assertFalse(c.isPlaying) }
            assertTrue("Stop request blocked on preparation for ${elapsed}ms", elapsed<2000)
            runBlocking(Dispatchers.Main) { MusicService.stopAndJoin() }
            main { assertFalse("Cancelled preparation must never start late",c.isPlaying) }
        } finally { main { c.release() } }
    }
    @Test fun logoutReportsIndependentCleanupResults() = signedIn {
        val report=runBlocking { core.logout() }
        assertTrue(report.localCleanupComplete)
        assertTrue(report.playbackReleaseComplete)
        assertTrue(report.serverRevoked)
        assertNull(core.account)
    }
    @Test fun authenticatedVideoCoreProxyActuallyRendersAndReleases() = signedIn {
        val owner=core.account!!
        val caps=JSONObject()
        main { caps.put("observed",decoderObservations()) }
        val plan=runBlocking { core.callOwned(owner,"playback",JSONObject().put("itemId","video").put("mediaType","Video").put("androidCapabilities",caps.getJSONObject("observed"))) }
        assertFalse(plan.getString("url").contains(owner.token))
        lateinit var player: androidx.media3.exoplayer.ExoPlayer
        var cleanupPlayer: androidx.media3.exoplayer.ExoPlayer? = null
        var cleanupSurface: android.view.Surface? = null
        try {
            main {
                val surface=androidx.media3.exoplayer.video.PlaceholderSurface.newInstance(context,false)
                cleanupSurface=surface
                player=androidx.media3.exoplayer.ExoPlayer.Builder(context).build()
                cleanupPlayer=player
                player.setVideoSurface(surface)
                player.setMediaItem(androidx.media3.common.MediaItem.fromUri(plan.getString("url")))
                player.prepare();player.play()
            }
            await("Actual decoded/rendered video frames",20) { player.currentPosition>500 && (player.videoDecoderCounters?.renderedOutputBufferCount ?: 0)>0 }
            main { player.pause();player.seekTo(1500) }
            await("Video seek") { player.currentPosition>=1500 }
        } finally {
            main { cleanupPlayer?.release(); cleanupSurface?.release() }
            val f=JSONObject().put("itemId","video").put("playSessionId",plan.getString("playSessionId")).put("playMethod",plan.getString("playMethod"))
            runBlocking { core.callOwned(owner,"releasePlayback",f); core.callOwned(owner,"stopPlaybackProxy",JSONObject().put("proxyId",plan.getString("proxyId"))) }
        }
        assertTrue("Released proxy must reject new readers",runCatching {
            val c=java.net.URL(plan.getString("url")).openConnection() as java.net.HttpURLConnection
            c.connectTimeout=1000;c.readTimeout=1000
            try { c.inputStream.close() } finally { c.disconnect() }
        }.isFailure)
    }
    @Test fun nativeLogoutBooleansRemainIndependentAndForeignFilesSurvive() {
        val root=java.io.File(context.noBackupFilesDir.canonicalFile,"logout-fixture-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        val v=JSONObject().put("server","http://127.0.0.1:18765").put("allowLocalHttp",true).put("clientPlatform","android")
            .put("userId","fixture-user").put("token","invalid-synthetic-token").put("deviceId","fixture").put("cacheRoot",root.absolutePath)
        try {
            v.put("operation","androidSignOut")
            val first=JSONObject(NativeCore.invoke(v.toString())).getJSONObject("value")
            assertTrue(first.getBoolean("localCleanupComplete"));assertTrue(first.getBoolean("playbackReleaseComplete"));assertFalse(first.getBoolean("serverRevoked"))
            val foreign=java.io.File(root.listFiles()!!.single(),"foreign-original.txt").apply { writeText("preserve") }
            val second=JSONObject(NativeCore.invoke(v.toString())).getJSONObject("value")
            assertFalse(second.getBoolean("localCleanupComplete"));assertTrue(second.getBoolean("playbackReleaseComplete"));assertFalse(second.getBoolean("serverRevoked"))
            assertEquals("preserve",foreign.readText())
        } finally { root.deleteRecursively() }
    }
    @Test fun accountChangeRequestedDuringPreparationNeverPlaysLate() = signedIn {
        val owner=core.account!!
        val c=controller()
        try {
            main { MusicService.current()!!.play(listOf(LibraryItem("delayed","Account-change pending","Audio")),false,null) }
            Thread.sleep(1000)
            main { MusicService.requestStop() }
            val report=runBlocking { core.logout() }
            runBlocking(Dispatchers.Main) { MusicService.stopAndJoin() }
            assertTrue(report.playbackReleaseComplete);assertTrue(report.localCleanupComplete);assertTrue(report.serverRevoked)
            main { assertFalse(c.isPlaying) }
            assertNull(core.account)
            assertTrue(runCatching { runBlocking { core.callOwned(owner,"downloads") } }.isFailure)
        } finally { main { c.release() } }
    }
    @Test fun secureStoreCommitFailuresDoNotPublishOrForgetAccount() {
        var failCommit=false
        val prefix="secure-fixture-${java.util.UUID.randomUUID()}-"
        val isolatedFiles=java.io.File(context.noBackupFilesDir.canonicalFile,prefix).apply { mkdirs() }
        val wrapped=object:android.content.ContextWrapper(context) {
            override fun getNoBackupFilesDir()=isolatedFiles
            override fun getSharedPreferences(name:String,mode:Int):android.content.SharedPreferences {
                val prefs=super.getSharedPreferences(prefix+name,mode)
                return object:android.content.SharedPreferences by prefs {
                    override fun edit():android.content.SharedPreferences.Editor {
                        val editor=prefs.edit()
                        return object:android.content.SharedPreferences.Editor by editor {
                            override fun putString(key:String,value:String?):android.content.SharedPreferences.Editor {editor.putString(key,value);return this}
                            override fun clear():android.content.SharedPreferences.Editor {editor.clear();return this}
                            override fun commit()=if(failCommit) false else editor.commit()
                        }
                    }
                }
            }
        }
        val isolated=CoreClient(wrapped)
        CoreRequestEnvironment.loopbackFixture=true
        try {
            runBlocking { isolated.login("http://127.0.0.1:18765","original-fixture-user","original-fixture-password") }
            val owner=isolated.account
            failCommit=true
            assertTrue(runCatching { runBlocking { isolated.logout() } }.isFailure)
            assertNull(isolated.account)
            assertTrue(isolated.cleanupPending)
            failCommit=false
            val recreated=CoreClient(wrapped)
            assertNull(recreated.account)
            assertTrue(recreated.cleanupPending)
            assertTrue(runCatching { runBlocking { recreated.callOwned(owner,"items") } }.isFailure)
            assertTrue(runBlocking { recreated.logout() }.complete)
            assertFalse(recreated.cleanupPending)
        } finally {
            failCommit=false
            if(isolated.account!=null || isolated.cleanupPending) runBlocking { isolated.logout() }
            context.deleteSharedPreferences(prefix+"account-v1")
            isolatedFiles.deleteRecursively()
            CoreRequestEnvironment.loopbackFixture=false
        }
    }
}
