package org.bluefin.android

import android.view.View
import android.view.ViewGroup
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import org.junit.Assert.*
import org.junit.Test
import java.io.File

/** Host drives the ordinary UI. This observer neither authenticates nor replaces content. */
class OrdinaryUiRuntimeTest {
    @Test fun ordinaryUiJourney() {
        val i = InstrumentationRegistry.getInstrumentation()
        val context = i.targetContext
        val end = System.nanoTime() + 1_200_000_000_000L
        val done = File(context.filesDir, "ordinary-ui-finish")
        done.delete()
        assertNull("Requires disposed synthetic account state", BluefinRuntime.core(context).account)
        assertFalse(BluefinRuntime.core(context).cleanupPending)
        CoreRequestEnvironment.loopbackFixture = true
        println("ORDINARY_UI_READY transport=debug-in-process-http-loopback")
        var previous = ""
        fun players(v: View): List<ExoPlayer> = when(v) {
            is PlayerView -> listOfNotNull(v.player as? ExoPlayer)
            is ViewGroup -> (0 until v.childCount).flatMap { players(v.getChildAt(it)) }
            else -> emptyList()
        }
        try {
            while (!done.exists() && System.nanoTime() < end) {
                i.runOnMainSync {
                    val core = BluefinRuntime.core(context)
                    val a = ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(Stage.RESUMED)
                        .filterIsInstance<MainActivity>().singleOrNull()
                    val video = a?.window?.decorView?.let { players(it) }.orEmpty()
                    val music = MusicService.current()?.let { service ->
                        MusicService::class.java.getDeclaredField("player").apply { isAccessible = true }.get(service) as? ExoPlayer
                    }
                    val telemetry = (video.map { "video" to it } + listOfNotNull(music?.let { "audio" to it })).joinToString { (kind,p) ->
                        "$kind time=${p.currentPosition} playing=${p.isPlaying} state=${p.playbackState} rendered=${p.videoDecoderCounters?.renderedOutputBufferCount} audioDecoded=${p.audioDecoderCounters?.renderedOutputBufferCount} queue=${p.mediaItemCount}"
                    }
                    val s = "account=${core.account?.userId} pending=${core.cleanupPending} resumed=${a != null} $telemetry"
                    if (s != previous) { println("ORDINARY_UI_STATE $s"); previous = s }
                }
                Thread.sleep(1000)
            }
            assertTrue("Host must explicitly complete bounded journey", done.exists())
            assertEquals("Host verdict", "PASS", done.readText().trim())
            assertNull("Ordinary UI must sign out", BluefinRuntime.core(context).account)
            assertFalse("Ordinary retry must finish cleanup", BluefinRuntime.core(context).cleanupPending)
        } finally { done.delete(); CoreRequestEnvironment.loopbackFixture = false }
    }
}
