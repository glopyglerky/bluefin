package org.bluefin.android

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.compose.foundation.Image
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import android.util.LruCache
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/** Account/version scoped memory cache. Encoded bytes, decoded pixels and concurrent loads are bounded. */
object Artwork {
    private val slots = Semaphore(2)
    private val work = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val monitor = Any()
    private var generation = 0L
    private val pending = mutableMapOf<String, Deferred<Bitmap?>>()
    private val memory = object : LruCache<String, Bitmap>(24 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.allocationByteCount
    }
    fun clear() = synchronized(monitor) {
        generation++
        pending.values.forEach { it.cancel() }
        pending.clear()
        memory.evictAll()
    }
    suspend fun fetch(core: CoreClient, owner: Account, sources: List<CatalogArtwork>): Bitmap? {
        for (source in sources) {
            if (core.account != owner) return null
            val version = source.tag ?: "untagged-" + (System.currentTimeMillis() / 900_000)
            val key = owner.scope + "/" + source.id + "/" + version
            val cached = synchronized(monitor) { memory.get(key) }
            if (cached != null) return cached
            val epoch: Long
            val request: Deferred<Bitmap?>
            synchronized(monitor) {
                epoch = generation
                request = pending.getOrPut(key) { work.async { download(core, owner, source) } }
            }
            val result = try { request.await() } catch (e: CancellationException) { throw e }
            finally { synchronized(monitor) { if (pending[key] === request && request.isCompleted) pending.remove(key) } }
            if (core.account != owner) return null
            synchronized(monitor) {
                if (epoch != generation) return null
                if (result != null) memory.put(key, result)
            }
            if (result != null) return result
        }
        return null
    }
    private suspend fun download(core: CoreClient, owner: Account, source: CatalogArtwork): Bitmap? = slots.withPermit {
        withContext(Dispatchers.IO) {
            runCatching {
                check(core.account == owner)
                val base = Uri.parse(owner.server)
                check(base.scheme == "https" && !base.host.isNullOrBlank() && base.userInfo == null && base.query == null && base.fragment == null)
                fun headerValue(value: String): String {
                    check(value.isNotEmpty() && value.all { it.code in 32..126 && it != '"' && it != '\\' })
                    return value
                }
                val url = base.buildUpon().appendPath("Items").appendPath(source.id).appendPath("Images").appendPath("Primary")
                    .appendQueryParameter("maxWidth", "512").appendQueryParameter("maxHeight", "512")
                    .apply { source.tag?.let { appendQueryParameter("tag", it) } }.build()
                val connection = URL(url.toString()).openConnection() as HttpURLConnection
                try {
                    connection.instanceFollowRedirects = false
                    connection.connectTimeout = 10000; connection.readTimeout = 10000
                    connection.setRequestProperty("Authorization", "MediaBrowser Client=\"Bluefin Android\", Device=\"Android\", DeviceId=\"${headerValue(owner.deviceId)}\", Version=\"0.1.0\", Token=\"${headerValue(owner.token)}\"")
                    val bitmap = decode(connection)
                    if (core.account == owner) bitmap else { bitmap?.recycle(); null }
                } finally { connection.disconnect() }
            }.getOrNull()
        }
    }

    suspend fun publicPortrait(url: String?): Bitmap? {
        if (url == null || !PublicTvTitle.isPortrait(url) && !isCommonsArtwork(url)) return null
        return slots.withPermit {
            withContext(Dispatchers.IO) {
                runCatching {
                    val connection = URL(url).openConnection() as HttpURLConnection
                    try {
                        connection.instanceFollowRedirects = false
                        connection.connectTimeout = 6000
                        connection.readTimeout = 6000
                        decode(connection)
                    } finally { connection.disconnect() }
                }.getOrNull()
            }
        }
    }

    private fun decode(connection: HttpURLConnection): Bitmap? {
        check(connection.responseCode == 200 && connection.contentType?.startsWith("image/") == true)
        check(connection.contentLengthLong <= 2 * 1024 * 1024)
        val bytes = connection.inputStream.use { input ->
            val out = java.io.ByteArrayOutputStream()
            val buffer = ByteArray(16384)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                check(out.size() + count <= 2 * 1024 * 1024)
                out.write(buffer, 0, count)
            }
            out.toByteArray()
        }
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        check(bounds.outWidth in 1..16384 && bounds.outHeight in 1..16384)
        var sample = 1
        while (bounds.outWidth / sample > 512 || bounds.outHeight / sample > 512) sample *= 2
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
    }
}

@Composable
fun ItemArtwork(item: LibraryItem, model: LibraryModel, modifier: Modifier) {
    val owner = model.core.account
    var bitmap by remember(item.artworkSources, owner) { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(item.artworkSources, owner) {
        if (owner != null) bitmap = Artwork.fetch(model.core, owner, item.artworkSources)
    }
    bitmap?.let { Image(it.asImageBitmap(), contentDescription = null, modifier = modifier, contentScale = ContentScale.Crop) }
}

@Composable fun PublicTvArtwork(item: PublicTvTitle, modifier: Modifier) {
    PublicCatalogArtwork(item.artwork, modifier)
}

@Composable fun PublicCatalogArtwork(artwork: String?, modifier: Modifier) {
    var bitmap by remember(artwork) { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(artwork) { bitmap = Artwork.publicPortrait(artwork) }
    bitmap?.let { Image(it.asImageBitmap(), contentDescription = null, modifier = modifier, contentScale = ContentScale.Crop) }
}
