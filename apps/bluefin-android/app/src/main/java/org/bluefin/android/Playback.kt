package org.bluefin.android

import android.media.MediaCodecList
import android.net.Uri
import androidx.annotation.OptIn
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.PlaybackException
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.ui.PlayerView
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONArray
import org.json.JSONObject

/** Observations only. Decoder presence does not prove an individual stream is playable. */
fun decoderObservations(): JSONObject {
    val types = runCatching { MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
        .filter { !it.isEncoder }.flatMap { it.supportedTypes.toList() }.distinct().sorted() }.getOrDefault(emptyList())
    return JSONObject().put("schemaVersion", 1).put("decoderMimeTypes", JSONArray(types))
        .put("nativeProbeSupported", false).put("hdrQualified", false).put("passthroughQualified", false)
}

@OptIn(UnstableApi::class)
@Composable
fun PlaybackScreen(item: LibraryItem, model: LibraryModel, offline: Boolean = false, audioQueue: List<LibraryItem> = listOf(item), onFinished: (Boolean) -> Unit) {
    if (item.audio) {
        MusicScreen(item, model, offline, audioQueue) { onFinished(false) }
        return
    }
    val context = LocalContext.current
    val owner = remember(item.id) { model.core.account }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var status by remember(item.id) { mutableStateOf("Preparing playback…") }
    var closing by remember(item.id) { mutableStateOf(false) }
    var ended by remember(item.id) { mutableStateOf(false) }
    var exit by remember(item.id) { mutableStateOf(false) }
    var disposed by remember(item.id) { mutableStateOf(false) }
    val preview = remember(item.id) { model.demo }
    val player = remember(item.id) {
        ExoPlayer.Builder(context).setMediaSourceFactory(DefaultMediaSourceFactory(DefaultDataSource.Factory(context)))
            .build().apply {
                setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build(), true)
                setHandleAudioBecomingNoisy(true)
            }
    }
    val session = remember(player) { MediaSession.Builder(context, player).build() }
    var position by remember(player) { mutableStateOf(0L) }
    val finished = remember(player) { CompletableDeferred<Unit>() }
    val failed = remember(player) { CompletableDeferred<PlaybackException>() }
    var readerLease by remember(player) { mutableStateOf<VideoReaders.Lease?>(null) }
    var readerStarted by remember(player) { mutableStateOf(false) }
    fun stopReader() {
        if (!disposed) {
            if (player.mediaItemCount > 0) position = player.currentPosition.coerceAtLeast(0)
            player.stop()
            player.clearMediaItems()
        }
    }
    fun requestExit() {
        exit = true
        stopReader()
        finished.complete(Unit)
    }
    DisposableEffect(player, lifecycle) {
        val lease = foregroundVideoReaders.register { requestExit() }
        readerLease = lease
        val listener = object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                stopReader()
                failed.complete(error)
            }
            override fun onPlaybackStateChanged(state: Int) { if (state == Player.STATE_ENDED) { ended = true; requestExit() } }
        }
        // Video belongs to this foreground screen. Background music has its own service.
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) requestExit() }
        player.addListener(listener); lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer); player.removeListener(listener)
            requestExit()
            disposed = true
            session.release(); player.release()
            // If the coroutine never started, there can be no captured plan.
            if (!readerStarted) foregroundVideoReaders.finished(lease)
        }
    }
    LaunchedEffect(item.id) {
        readerStarted = true
        val lease = readerLease
        var plan: JSONObject? = null
        var startAttempted = false
        var showError = false
        fun checkAdmission() {
            if (exit || disposed || !lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) || model.core.account != owner) {
                requestExit()
                error("Video playback was closed before preparation completed.")
            }
        }
        fun fields() = JSONObject().put("itemId", item.id)
            .put("mediaSourceId", plan?.optString("mediaSourceId"))
            .put("playSessionId", plan?.optString("playSessionId"))
            .put("playMethod", plan?.optString("playMethod"))
            .put("liveStreamId", plan?.optJSONObject("source")?.optString("LiveStreamId"))
            .put("positionTicks", position * 10000 + (plan?.optLong("startOffsetTicks") ?: 0))
        try {
            check(lease != null) { "Finish closing the previous video before playing another." }
            MusicService.stopAndJoin()
            if (preview) {
                currentCoroutineContext().ensureActive()
                checkAdmission()
                check(item.audio) { "This synthetic catalog entry has no playable media. Choose Three notes at sea." }
                player.setMediaItem(MediaItem.fromUri("asset:///bluefin-tone.wav")); player.prepare(); player.play()
                status = "Original synthetic audio"
            } else {
                checkAdmission()
                // Retrieve any native resource IDs even if the screen is dismissed during a bounded JNI call.
                withContext(NonCancellable) {
                    plan = model.core.callOwned(owner, "playback", JSONObject().put("itemId", item.id).put("startTicks", item.ticks)
                        .put("mediaType", if (item.audio) "Audio" else "Video").put("androidCapabilities", decoderObservations())
                        .apply { model.irohConfig()?.let { put("iroh", it) } })
                }
                currentCoroutineContext().ensureActive()
                checkAdmission()
                val p = requireNotNull(plan)
                val uri = Uri.parse(p.getString("url"))
                check(uri.scheme == "http" && uri.host == "127.0.0.1" && uri.userInfo == null && uri.fragment == null) { "Native core did not return its loopback stream proxy" }
                check(p.optJSONObject("headers")?.length() in listOf(null, 0)) { "Unexpected player credentials" }
                check(!p.optBoolean("nativeProbeRequired")) { "This stream needs an unqualified native asset probe" }
                player.setMediaItem(MediaItem.fromUri(uri))
                player.seekTo(((item.ticks - p.optLong("startOffsetTicks")).coerceAtLeast(0)) / 10000)
                player.prepare(); player.play()
                // A failed response can still mean the server accepted the start report.
                startAttempted = true
                model.core.callOwned(owner, "started", fields())
                currentCoroutineContext().ensureActive()
                checkAdmission()
                status = p.optString("playMethod", "Playing")
            }
            while (!exit) {
                val completed = withTimeoutOrNull(1000) {
                    select {
                        finished.onAwait { true }
                        failed.onAwait { throw it }
                    }
                }
                if (completed == null && !exit) {
                    checkAdmission()
                    position = player.currentPosition.coerceAtLeast(0)
                    if (!preview && position % 10000 < 1000) {
                        runCatching { model.core.callOwned(owner, "progress", fields().put("isPaused", !player.isPlaying)) }
                    }
                }
            }
        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException) throw e
            if (!exit && !disposed) {
                status = e.message ?: "Playback unavailable"
                showError = true
            }
        } finally {
            closing = true
            try {
                stopReader()
                withContext(NonCancellable) {
                    if (plan != null) {
                        if (startAttempted) runCatching { model.core.callOwned(owner, "stopped", fields()) }
                        val release = runCatching { model.core.callOwned(owner, "releasePlayback", fields()) }
                        val stop = runCatching { model.core.callOwned(owner, "stopPlaybackProxy", JSONObject().put("proxyId", plan!!.getString("proxyId"))) }
                        if (release.isFailure || stop.isFailure) model.message("Playback closed locally. Server resource cleanup could not be confirmed.")
                    }
                }
                foregroundVideoReaders.finished(lease)
            } catch (failure: Throwable) {
                foregroundVideoReaders.failed(lease, failure)
                throw failure
            }
        }
        // The error stays visible only after the player and captured plan are retired.
        if (showError && !exit) {
            closing = false
            finished.await()
        }
        onFinished(ended)
    }
    androidx.activity.compose.BackHandler { requestExit() }
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(item.name, style = MaterialTheme.typography.headlineSmall)
        Text(status)
        AndroidView(
            factory = { context ->
                PlayerView(context).apply {
                    this.player = player
                    useController = true
                    controllerShowTimeoutMs = 5000
                    controllerHideOnTouch = true
                    requestFocus()
                }
            },
            modifier = Modifier.weight(1f).fillMaxWidth(),
            update = { it.player = player }
        )
        Button(onClick = { requestExit() }, enabled = !closing) { Text(if (closing) "Closing…" else "Back to library") }
    }
}

@OptIn(UnstableApi::class)
@Composable
fun MusicScreen(item: LibraryItem, model: LibraryModel, offline: Boolean, queue: List<LibraryItem>, done: () -> Unit) {
    val context = LocalContext.current
    var controller by remember { mutableStateOf<androidx.media3.session.MediaController?>(null) }
    var status by remember { mutableStateOf("Connecting music service…") }
    val scope = rememberCoroutineScope()
    DisposableEffect(item.id, offline) {
        val token = androidx.media3.session.SessionToken(context, android.content.ComponentName(context, MusicService::class.java))
        val future = androidx.media3.session.MediaController.Builder(context, token).buildAsync()
        var disposed = false
        future.addListener({
            if (!disposed) runCatching {
                controller = future.get()
                MusicService.start(context, queue, offline, model.irohConfig())
            }.onFailure { status = "Music service unavailable" }
        }, androidx.core.content.ContextCompat.getMainExecutor(context))
        onDispose { disposed = true; androidx.media3.session.MediaController.releaseFuture(future) }
    }
    LaunchedEffect(item.id) {
        while (true) { delay(500); MusicService.current()?.let { status = it.status } }
    }
    androidx.activity.compose.BackHandler { done() }
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(item.name, style = MaterialTheme.typography.headlineSmall)
        Text(status)
        controller?.let { player ->
            AndroidView(factory = { PlayerView(it).apply { this.player = player; useController = true; controllerShowTimeoutMs = 0; requestFocus() } },
                modifier = Modifier.weight(1f).fillMaxWidth(), update = { it.player = player })
        }
        Button(onClick = done) { Text("Library · keep music playing") }
        Button(onClick = { MusicService.requestStop(); scope.launch { MusicService.stopAndJoin(); done() } }) { Text("Stop music") }
    }
}
