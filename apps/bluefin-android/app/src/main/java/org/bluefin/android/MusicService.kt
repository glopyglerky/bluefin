package org.bluefin.android

import android.content.Intent
import android.net.Uri
import androidx.media3.common.*
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject

/** Owns the player, account, reporting and proxy lease independently of any Activity. */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
class MusicService : MediaSessionService() {
    private lateinit var player: ExoPlayer
    private lateinit var session: MediaSession
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val playbackMutex = Mutex()
    private var work: Job? = null
    private var requestKey: String? = null
    private var destroyed = false
    private var foregroundOwned = false
    private var preparing = false
    private val stopCommand = androidx.media3.session.SessionCommand(STOP, android.os.Bundle.EMPTY)
    private fun preparingNotification(): android.app.Notification {
        val manager = requireNotNull(getSystemService(android.app.NotificationManager::class.java)) { "Notifications are unavailable" }
        manager.createNotificationChannel(android.app.NotificationChannel("preparing", "Music preparation", android.app.NotificationManager.IMPORTANCE_LOW))
        val stop = android.app.PendingIntent.getService(this, 0, Intent(this, MusicService::class.java).setAction(STOP),
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT)
        return androidx.core.app.NotificationCompat.Builder(this, "preparing")
            .setSmallIcon(android.R.drawable.ic_media_play).setContentTitle("Bluefin")
            .setContentText(if (work?.isCancelled == true) "Stopping; completing cleanup" else "Preparing music")
            .setOngoing(true).addAction(android.R.drawable.ic_media_pause, "Stop", stop)
            .setStyle(androidx.media3.session.MediaStyleNotificationHelper.MediaStyle(session)).build()
    }
    override fun onUpdateNotification(session: MediaSession, startInForegroundRequired: Boolean) {
        // No fabricated player timeline while native admission is pending. Keep the
        // real user-started lease until playback or captured-account cleanup finishes.
        if (foregroundOwned && (preparing || work?.isCancelled == true)) return
        super.onUpdateNotification(session, startInForegroundRequired)
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == START) {
            val request = pending
            if (request == null || request.id != intent.getStringExtra("request")) {
                if (work?.isActive != true) stopSelf(startId)
                return START_NOT_STICKY
            }
            pending = null
            if (request.owner != BluefinRuntime.core(this).account) {
                if (work?.isActive != true) stopSelf(startId)
                return START_NOT_STICKY
            }
            val key = "${request.owner?.scope}:${request.items.map { it.id }}:${request.offline}:${request.iroh}"
            if (requestKey == key && work?.isActive == true) return START_NOT_STICKY
            preparing = true
            foregroundOwned = true
            androidx.core.app.ServiceCompat.startForeground(this, PREPARING_NOTIFICATION, preparingNotification(),
                android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
            play(request.items, request.offline, request.iroh)
        } else if (intent?.action == STOP) {
            requestStop()
            scope.launch { stopAndJoin() }
        }
        return START_NOT_STICKY
    }
    private var queueIndex = 0
    private var queueSize = 0
    private var requestedIndex: Int? = null
    private fun navigate(delta: Int) {
        val next = queueIndex + delta
        if (work?.isActive == true && next in 0 until queueSize) requestedIndex = next
    }
    var status = "Ready"; private set
    override fun onCreate() {
        super.onCreate()
        instance = this
        player = ExoPlayer.Builder(this).build().apply {
            setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(), true)
            setHandleAudioBecomingNoisy(true)
            setWakeMode(C.WAKE_MODE_LOCAL)
        }
        val controls = object : ForwardingSimpleBasePlayer(player) {
            override fun handleStop(): com.google.common.util.concurrent.ListenableFuture<*> {
                requestStop()
                return com.google.common.util.concurrent.Futures.immediateVoidFuture()
            }
            override fun getState(): State {
                val state = super.getState()
                return state.buildUpon().setAvailableCommands(state.availableCommands.buildUpon()
                    .addAll(Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                        Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM).build()).build()
            }
            override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): com.google.common.util.concurrent.ListenableFuture<*> {
                when (seekCommand) {
                    Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> navigate(1)
                    Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> navigate(-1)
                    else -> return super.handleSeek(mediaItemIndex, positionMs, seekCommand)
                }
                return com.google.common.util.concurrent.Futures.immediateVoidFuture()
            }
        }
        session = MediaSession.Builder(this, controls)
            .setCustomLayout(listOf(androidx.media3.session.CommandButton.Builder()
                .setDisplayName("Stop").setIconResId(android.R.drawable.ic_media_pause)
                .setSessionCommand(stopCommand).build()))
            .setCallback(object : MediaSession.Callback {
            override fun onConnect(session: MediaSession, controller: MediaSession.ControllerInfo): MediaSession.ConnectionResult {
                if (controller.uid != android.os.Process.myUid() && !controller.isTrusted)
                    return MediaSession.ConnectionResult.reject()
                // Session permission is stable; Media3 intersects it with the
                // player's changing commands as each admitted item becomes ready.
                val commands = Player.Commands.Builder().addAllCommands()
                    .remove(Player.COMMAND_SET_MEDIA_ITEM).remove(Player.COMMAND_CHANGE_MEDIA_ITEMS)
                // External controls need transport state and display metadata, not
                // local file/proxy capabilities or the private playback timeline.
                if (controller.uid != android.os.Process.myUid()) {
                    commands.remove(Player.COMMAND_GET_CURRENT_MEDIA_ITEM).remove(Player.COMMAND_GET_TIMELINE)
                }
                return MediaSession.ConnectionResult.AcceptedResultBuilder(session)
                    .setAvailableSessionCommands(MediaSession.ConnectionResult.DEFAULT_SESSION_COMMANDS
                        .buildUpon().add(stopCommand).build())
                    .setAvailablePlayerCommands(commands.build()).build()
            }
            override fun onCustomCommand(session: MediaSession, controller: MediaSession.ControllerInfo,
                customCommand: androidx.media3.session.SessionCommand, args: android.os.Bundle):
                com.google.common.util.concurrent.ListenableFuture<androidx.media3.session.SessionResult> {
                if (customCommand.customAction != STOP)
                    return com.google.common.util.concurrent.Futures.immediateFuture(
                        androidx.media3.session.SessionResult(androidx.media3.session.SessionError.ERROR_NOT_SUPPORTED))
                requestStop()
                scope.launch { stopAndJoin() }
                return com.google.common.util.concurrent.Futures.immediateFuture(
                    androidx.media3.session.SessionResult(androidx.media3.session.SessionResult.RESULT_SUCCESS))
            }
            override fun onAddMediaItems(session: MediaSession, controller: MediaSession.ControllerInfo,
                mediaItems: List<MediaItem>): com.google.common.util.concurrent.ListenableFuture<List<MediaItem>> =
                com.google.common.util.concurrent.Futures.immediateFailedFuture(UnsupportedOperationException("Use the admitted queue"))
        }).build()
    }
    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession = session
    fun play(items: List<LibraryItem>, offline: Boolean, iroh: JSONObject?) {
        val core = BluefinRuntime.core(this)
        val owner = core.account
        val key = "${owner?.scope}:${items.map { it.id }}:$offline:$iroh"
        if (requestKey == key && work?.isActive == true) return
        requestKey = key
        work?.cancel()
        work = scope.launch { playbackMutex.withLock {
            val queue = items.take(200).toList()
            queueIndex = 0; queueSize = queue.size; requestedIndex = null
            while (queueIndex in queue.indices) {
                val item = queue[queueIndex]
                ensureActive()
                check(item.audio) { "Music queue accepts audio only" }
                var plan: JSONObject? = null
                var started = false
                var position = 0L
                fun fields() = JSONObject().put("itemId", item.id)
                    .put("mediaSourceId", plan?.optString("mediaSourceId"))
                    .put("playSessionId", plan?.optString("playSessionId"))
                    .put("playMethod", plan?.optString("playMethod"))
                    .put("positionTicks", position * 10000 + (plan?.optLong("startOffsetTicks") ?: 0))
                try {
                    preparing = foregroundOwned
                    status = "Preparing ${item.name}"
                    val uri = if (owner == null) {
                        check(item.id == "demo-audio" && !offline)
                        Uri.parse("asset:///bluefin-tone.wav")
                    } else if (offline) {
                        val records = core.callOwned(owner, "downloads").getJSONArray("Items")
                        val record = (0 until records.length()).map { records.getJSONObject(it) }
                            .single { it.optString("itemId") == item.id && it.optString("state") == "completed" }
                        // Only core verification supplies localUrl after bounded size/hash and no-follow checks.
                        val local = Uri.parse(record.getString("localUrl"))
                        check(local.scheme == "file" && local.authority.isNullOrEmpty())
                        local
                    } else {
                        withContext(NonCancellable) {
                            plan = core.callOwned(owner, "playback", JSONObject().put("itemId", item.id)
                                .put("startTicks", item.ticks).put("mediaType", "Audio")
                                .put("androidCapabilities", decoderObservations()).apply { if (iroh != null) put("iroh", iroh) })
                        }
                        val p = requireNotNull(plan)
                        val local = Uri.parse(p.getString("url"))
                        check(local.scheme == "http" && local.host == "127.0.0.1" && local.userInfo == null && local.fragment == null)
                        check(p.optJSONObject("headers")?.length() in listOf(null, 0))
                        local
                    }
                    ensureActive()
                    player.setMediaItem(MediaItem.Builder().setUri(uri).setMediaId(item.id)
                        .setMediaMetadata(MediaMetadata.Builder().setTitle(item.name).build()).build())
                    player.seekTo((item.ticks - (plan?.optLong("startOffsetTicks") ?: 0)).coerceAtLeast(0) / 10000); player.prepare(); player.play()
                    preparing = false
                    onUpdateNotification(session, true)
                    getSystemService(android.app.NotificationManager::class.java)?.cancel(PREPARING_NOTIFICATION)
                    if (plan != null) { core.callOwned(owner, "started", fields()); started = true }
                    status = if (offline) "Verified downloaded source · output depends on device" else "${plan?.optString("playMethod") ?: "Original synthetic audio"} · output depends on device"
                    var seconds = 0
                    while (player.playbackState != Player.STATE_ENDED && requestedIndex == null) {
                        delay(250)
                        position = player.currentPosition.coerceAtLeast(0)
                        if (player.playbackState == Player.STATE_IDLE) return@withLock
                        player.playerError?.let { throw IllegalStateException(it.errorCodeName) }
                        if (++seconds % 40 == 0 && plan != null) core.callOwned(owner, "progress", fields().put("isPaused", !player.isPlaying))
                    }
                } catch (e: CancellationException) { throw e }
                catch (_: Exception) { status = "Playback unavailable. Check account, download or pinned route."; break }
                finally {
                    // Stop readers before releasing the proxy or allowing removal/sign-out.
                    val report = fields()
                    if (!destroyed) { player.stop(); player.clearMediaItems() }
                    withContext(NonCancellable) {
                        if (plan != null) {
                            if (started) runCatching { core.callOwned(owner, "stopped", report) }
                            val release = runCatching { core.callOwned(owner, "releasePlayback", report) }
                            val stop = runCatching { core.callOwned(owner, "stopPlaybackProxy", JSONObject().put("proxyId", plan!!.getString("proxyId"))) }
                            if (release.isFailure || stop.isFailure) status = "Local playback stopped; server cleanup unconfirmed."
                        }
                    }
                }
                queueIndex = requestedIndex ?: (queueIndex + 1)
                requestedIndex = null
            }
        } }
        val admitted = work
        admitted?.invokeOnCompletion {
            scope.launch {
                if (work === admitted && foregroundOwned) {
                    foregroundOwned = false; preparing = false
                    if (!destroyed) super@MusicService.onUpdateNotification(session, false)
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    getSystemService(android.app.NotificationManager::class.java)?.cancel(PREPARING_NOTIFICATION)
                    stopSelf()
                }
            }
        }
    }
    override fun onDestroy() {
        destroyed = true
        work?.cancel()
        player.stop()
        session.release(); player.release()
        // Cleanup coroutine retains the captured account. Do not cancel it during its finalizer.
        work?.invokeOnCompletion { scope.cancel() } ?: scope.cancel()
        if (instance === this) instance = null
        super.onDestroy()
    }
    companion object {
        private const val START = "org.bluefin.android.START_MUSIC"
        private const val STOP = "org.bluefin.android.STOP_MUSIC"
        private const val PREPARING_NOTIFICATION = 2001
        private data class Pending(val id: String, val owner: Account?, val items: List<LibraryItem>, val offline: Boolean, val iroh: JSONObject?)
        private var pending: Pending? = null
        private var instance: MusicService? = null
        /** Called by the visible production music screen; no account or queue in Intents. */
        fun start(context: android.content.Context, items: List<LibraryItem>, offline: Boolean, iroh: JSONObject?) {
            check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper())
            require(items.isNotEmpty() && items.size <= 200 && items.all { it.audio })
            val request = Pending(java.util.UUID.randomUUID().toString(), BluefinRuntime.core(context).account,
                items.toList(), offline, iroh?.let { JSONObject(it.toString()) })
            pending = request
            try {
                androidx.core.content.ContextCompat.startForegroundService(context,
                    Intent(context, MusicService::class.java).setAction(START).putExtra("request", request.id))
            } catch (e: Exception) { if (pending === request) pending = null; throw e }
        }
        fun current(): MusicService? = instance
        /** Stop audible work immediately; retained native preparation still owns its cleanup. */
        fun requestStop() {
            pending = null
            val service = instance ?: return
            service.work?.cancel()
            if (!service.destroyed) { service.player.stop(); service.player.clearMediaItems() }
            service.status = "Stopping; waiting for server cleanup."
        }
        suspend fun stopAndJoin() = withContext(Dispatchers.Main.immediate) {
            // ExoPlayer and service state belong to Main even when sign-out is
            // requested by a background caller. Joining suspends this dispatcher.
            val service = instance ?: return@withContext
            requestStop()
            service.work?.cancelAndJoin()
            service.playbackMutex.withLock {
                if (!service.destroyed) { service.player.stop(); service.player.clearMediaItems() }
            }
            service.stopSelf()
        }
    }
}
