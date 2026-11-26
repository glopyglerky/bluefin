package org.bluefin.android

import android.content.ComponentName
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.TimeUnit

/** Real packaged JNI and Media3, synthetic asset only. Never uses an existing account. */
@RunWith(AndroidJUnit4::class)
class NativePlaybackTest {
    @Test fun finalAndAncestorSymlinksAreRejectedWithoutTouchingSibling() {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val base=java.io.File(context.noBackupFilesDir.canonicalFile,"links-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        val actual=java.io.File(base,"actual").apply { mkdirs() }
        val sentinel=java.io.File(actual,"foreign.txt").apply { writeText("preserve") }
        val link=java.io.File(base,"link")
        java.nio.file.Files.createSymbolicLink(link.toPath(),actual.toPath())
        try {
            for (path in listOf(link,java.io.File(link,"descendant"))) {
                val v=JSONObject().put("operation","downloads").put("clientPlatform","android").put("server","https://example.invalid")
                    .put("userId","one").put("token","synthetic").put("deviceId","synthetic").put("cacheRoot",path.absolutePath)
                assertFalse(JSONObject(NativeCore.invoke(v.toString())).getBoolean("ok"))
                assertEquals("preserve",sentinel.readText())
                assertEquals(1,actual.listFiles()!!.size)
            }
        } finally { java.nio.file.Files.delete(link.toPath());base.deleteRecursively() }
    }
    @Test fun missingCatalogRegressionDiagnostic() {
        val context=InstrumentationRegistry.getInstrumentation().targetContext
        val root=java.io.File(context.noBackupFilesDir.canonicalFile,"missing-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        val v=JSONObject().put("operation","downloads").put("clientPlatform","android").put("server","https://example.invalid")
            .put("userId","one").put("token","synthetic").put("deviceId","synthetic").put("cacheRoot",root.absolutePath)
        try {
            val missing=JSONObject(NativeCore.invoke(v.toString()))
            val directory=root.listFiles()!!.single()
            // Diagnostic only: existing metadata isolates missing-file detection.
            java.io.File(directory,"catalog.json").writeText("[]")
            val existing=JSONObject(NativeCore.invoke(v.toString()))
            println("CACHE_DIAGNOSTIC missing=$missing existing=$existing")
            assertTrue("Existing catalog must read through repaired traversal: $existing",existing.getBoolean("ok"))
            assertTrue("Fresh empty namespace must work: $missing",missing.getBoolean("ok"))
        } finally { root.deleteRecursively() }
    }
    @Test fun packagedBridgeAdmitsAndroidAndRejectsIdentityOverride() {
        assertTrue(NativeCore.available)
        val result = JSONObject(NativeCore.invoke("""{"operation":"androidBridgeStatus"}"""))
        assertTrue(result.getJSONObject("value").getBoolean("qualified"))
        assertFalse(JSONObject(NativeCore.invoke("""{"operation":"info","clientPlatform":"apple"}""")).getBoolean("ok"))
    }
    @Test fun actualCoreRejectsSymlinkCacheAndIsolatesAccounts() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = java.io.File(context.noBackupFilesDir.canonicalFile, "instrumentation-offline-${java.util.UUID.randomUUID()}").apply { mkdirs() }
        fun request(user: String) = JSONObject().put("operation", "downloads").put("clientPlatform", "android")
            .put("server", "https://example.invalid").put("userId", user).put("token", "synthetic")
            .put("deviceId", "synthetic").put("cacheRoot", root.absolutePath)
        fun invoke(user: String) = JSONObject(NativeCore.invoke(request(user).toString()))
        try {
            val initial = invoke("one")
            assertTrue(initial.toString(), initial.getBoolean("ok"))
            val first = root.listFiles()!!.single()
            assertTrue(invoke("two").getBoolean("ok"))
            assertEquals(2, root.listFiles()!!.size)
            val outside = java.io.File(root, "fixture.json").apply { writeText("[]") }
            java.nio.file.Files.createSymbolicLink(java.io.File(first, "catalog.json").toPath(), outside.toPath())
            assertFalse(invoke("one").getBoolean("ok"))
            assertTrue(invoke("two").getBoolean("ok"))
            assertEquals("[]", outside.readText())
        } finally { root.deleteRecursively() }
    }
    @Test fun serviceOwnsSyntheticAudioAcrossControllerDisconnectAndSeeks() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        check(BluefinRuntime.core(context).account == null) { "Synthetic test requires an empty test installation" }
        // Android 15 requires a resumed app or an existing foreground service for
        // initial audio focus. The device operator opens the app through T3.
        var resumed = false
        val foregroundDeadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30)
        while (!resumed && System.nanoTime() < foregroundDeadline) {
            instrumentation.runOnMainSync {
                resumed = androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                    .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).isNotEmpty()
            }
            if (!resumed) Thread.sleep(100)
        }
        assertTrue("Open the empty app through the T3 device route before audio qualification", resumed)
        lateinit var future: com.google.common.util.concurrent.ListenableFuture<MediaController>
        instrumentation.runOnMainSync {
            future = MediaController.Builder(context, SessionToken(context, ComponentName(context, MusicService::class.java))).buildAsync()
        }
        val controller = future.get(15, TimeUnit.SECONDS)
        val first = Demo.items.single { it.audio }
        instrumentation.runOnMainSync {
            MusicService.current()!!.play(listOf(first, first.copy(name = "Queue second")) + List(6) { first }, false, null)
        }
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15)
        var advanced = false
        while (!advanced && System.nanoTime() < deadline) {
            instrumentation.runOnMainSync { advanced = controller.currentPosition > 250 }
            Thread.sleep(50)
        }
        assertTrue("Actual Media3 time must advance", advanced)
        instrumentation.runOnMainSync {
            assertFalse(controller.isCommandAvailable(androidx.media3.common.Player.COMMAND_SET_MEDIA_ITEM))
            assertFalse(controller.isCommandAvailable(androidx.media3.common.Player.COMMAND_CHANGE_MEDIA_ITEMS))
            // This controller is the app's own UID; direct service access from an
            // unrelated UID is blocked by Android before session admission.
            assertFalse(context.packageManager.getServiceInfo(ComponentName(context, MusicService::class.java), 0).exported)
            assertTrue(controller.isCommandAvailable(androidx.media3.common.Player.COMMAND_SEEK_TO_NEXT))
            controller.seekToNext()
        }
        fun awaitTitle(title: String) {
            val until = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
            var matches = false
            while (!matches && System.nanoTime() < until) {
                instrumentation.runOnMainSync { matches = controller.mediaMetadata.title.toString() == title }
                if (!matches) Thread.sleep(50)
            }
            assertTrue("Native queue must reach $title", matches)
        }
        awaitTitle("Queue second")
        instrumentation.runOnMainSync { controller.seekToPrevious() }
        awaitTitle(first.name)
        var seekable = false
        val readyDeadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (!seekable && System.nanoTime() < readyDeadline) {
            instrumentation.runOnMainSync {
                seekable = controller.playbackState == androidx.media3.common.Player.STATE_READY &&
                    controller.isCommandAvailable(androidx.media3.common.Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM)
            }
            if (!seekable) Thread.sleep(50)
        }
        assertTrue("Previous entry must finish preparation and become seekable", seekable)
        instrumentation.runOnMainSync { controller.pause(); controller.seekTo(1000) }
        var sought = false
        val seekDeadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (!sought && System.nanoTime() < seekDeadline) {
            instrumentation.runOnMainSync { sought = controller.currentPosition >= 1000 }
            if (!sought) Thread.sleep(50)
        }
        assertTrue("Media3 must acknowledge the seek", sought)
        instrumentation.runOnMainSync { controller.play(); controller.release() }
        // Operator backgrounds the app through T3 during this window. Several
        // real three-second assets must advance without an attached controller.
        Thread.sleep(8000)
        instrumentation.runOnMainSync { assertNotNull(MusicService.current()) }
        runBlocking(kotlinx.coroutines.Dispatchers.Main) { MusicService.stopAndJoin() }
    }
}
