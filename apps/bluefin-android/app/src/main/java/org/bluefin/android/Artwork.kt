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
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.net.HttpURLConnection
import java.net.URL

/** No disk cache, token URL or redirects. Encoded bytes and decoded pixels are both bounded. */
object Artwork {
    private val slots = Semaphore(2)
    suspend fun fetch(core: CoreClient, owner: Account, id: String): Bitmap? = slots.withPermit {
        withContext(Dispatchers.IO) {
            runCatching {
                check(core.account == owner)
                val base = Uri.parse(owner.server)
                check(base.scheme == "https" && !base.host.isNullOrBlank() && base.userInfo == null && base.query == null && base.fragment == null)
                fun headerValue(value: String): String {
                    check(value.isNotEmpty() && value.all { it.code in 32..126 && it != '"' && it != '\\' })
                    return value
                }
                val url = base.buildUpon().appendPath("Items").appendPath(id).appendPath("Images").appendPath("Primary")
                    .appendQueryParameter("maxWidth", "512").appendQueryParameter("maxHeight", "512").build()
                val connection = URL(url.toString()).openConnection() as HttpURLConnection
                try {
                    connection.instanceFollowRedirects = false
                    connection.connectTimeout = 10000; connection.readTimeout = 10000
                    connection.setRequestProperty("Authorization", "MediaBrowser Client=\"Bluefin Android\", Device=\"Android\", DeviceId=\"${headerValue(owner.deviceId)}\", Version=\"0.1.0\", Token=\"${headerValue(owner.token)}\"")
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
                    val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
                    if (core.account == owner) bitmap else { bitmap?.recycle(); null }
                } finally { connection.disconnect() }
            }.getOrNull()
        }
    }
}

@Composable
fun ItemArtwork(item: LibraryItem, model: LibraryModel, modifier: Modifier) {
    val owner = model.core.account
    var bitmap by remember(item.id, owner) { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(item.id, owner) {
        if (owner != null) bitmap = Artwork.fetch(model.core, owner, item.id)
    }
    bitmap?.let { Image(it.asImageBitmap(), contentDescription = null, modifier = modifier, contentScale = ContentScale.Crop) }
}
