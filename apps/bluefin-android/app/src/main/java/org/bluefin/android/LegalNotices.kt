package org.bluefin.android

import android.annotation.SuppressLint
import android.content.res.AssetManager
import android.os.Build
import android.os.Process
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.ui.input.key.*
import kotlinx.coroutines.launch
import androidx.compose.foundation.border
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import org.json.JSONObject
import java.io.ByteArrayInputStream

/** Paths are relative to the containing resource, confined to the legal asset tree. */
internal fun legalPath(base: String, reference: String): String {
    require(reference.isNotBlank() && !reference.startsWith('/') &&
        reference.none { it == ':' || it == '\\' || it == '%' || it == '?' || it == '#' || it.code < 32 })
    val parts = base.substringBeforeLast('/', "").split('/').filter { it.isNotEmpty() }.toMutableList()
    for (part in reference.split('/')) when (part) {
        "", "." -> Unit
        ".." -> { require(parts.isNotEmpty()); parts.removeAt(parts.lastIndex) }
        else -> parts.add(part)
    }
    require(parts.isNotEmpty())
    return parts.joinToString("/")
}

internal data class LegalEntry(val title: String, val paths: List<String>, val reference: String = "")
internal fun legalEntries(json: String, index: String): List<LegalEntry> {
    val root = JSONObject(json)
    require(root.getString("platform") == "android")
    val result = mutableListOf<LegalEntry>()
    val first = root.getJSONArray("first_party")
    require(first.length() > 0)
    result += LegalEntry("Bluefin license and attribution", (0 until first.length()).map { legalPath(index, first.getString(it)) })
    result += LegalEntry("Standard library notices", listOf(legalPath(index, root.getString("standard_library_html"))))
    val packages = root.getJSONArray("packages")
    require(packages.length() > 0)
    for (i in 0 until packages.length()) {
        val item = packages.getJSONObject(i)
        val texts = item.getJSONArray("texts")
        result += LegalEntry(listOf(item.getString("package"), if (item.isNull("version")) "" else item.getString("version")).filter { it.isNotEmpty() }.joinToString(" "),
            (0 until texts.length()).map { legalPath(index, texts.getString(it)) }, item.getString("reference"))
    }
    return result
}

// Provenance names describe the original document, without inferring a license grant.
private fun documentLabel(entry: LegalEntry, resource: String, provenance: JSONObject?): String {
    when (resource) {
        "first-party/LICENSE" -> return "License"
        "first-party/README.md" -> return "About Bluefin"
    }
    if (resource.endsWith("/COPYRIGHT-library.html")) return "Standard-library copyright notices"
    val filename = resource.substringAfterLast('/')
    if (!filename.matches(Regex("[0-9a-f]{64}\\.txt"))) return filename
    val origins = provenance?.optJSONArray(filename.removeSuffix(".txt"))
    val prefix = "${entry.reference} : "
    val names = (0 until (origins?.length() ?: 0)).mapNotNull { index ->
        origins?.optString(index)?.takeIf { entry.reference.isNotEmpty() && it.startsWith(prefix) }
            ?.removePrefix(prefix)?.takeIf { name ->
                name.isNotBlank() && name.length <= 100 && name.none { it == '/' || it == '\\' || it.code < 32 }
            }
    }.distinct()
    return names.singleOrNull() ?: "License text ${entry.paths.indexOf(resource) + 1}"
}

private fun noticeIndex(): String {
    val abis = if (Process.is64Bit()) Build.SUPPORTED_64_BIT_ABIS else Build.SUPPORTED_32_BIT_ABIS
    val target = abis.firstNotNullOfOrNull { abi -> when (abi) {
        "arm64-v8a" -> "aarch64-linux-android"
        "armeabi-v7a" -> "armv7-linux-androideabi"
        "x86_64" -> "x86_64-linux-android"
        else -> null
    } } ?: error("No bundled license index for this architecture")
    return "indexes/android-$target.json"
}
private fun AssetManager.legalText(path: String) = open("legal/$path").bufferedReader(Charsets.UTF_8).use { it.readText() }

@Composable private fun NoticeButton(title: String, click: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    OutlinedButton(onClick = click, modifier = Modifier.fillMaxWidth().onFocusChanged { focused = it.isFocused }
        .border(if (focused) 3.dp else 0.dp, if (focused) MaterialTheme.colorScheme.primary else Color.Transparent)) { Text(title) }
}

@Composable internal fun LegalNotices(done: () -> Unit) {
    val assets = LocalContext.current.assets
    var selected by rememberSaveable { mutableStateOf<Int?>(null) }
    var document by rememberSaveable { mutableStateOf<String?>(null) }
    val entries = remember { runCatching { val index = noticeIndex(); legalEntries(assets.legalText(index), index) } }
    val provenance = remember { runCatching { JSONObject(assets.legalText("third-party/text-provenance.json")) }.getOrNull() }
    val back = { if (document != null) document = null else if (selected != null) selected = null else done() }
    BackHandler(onBack = back)
    Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Software licenses", style = MaterialTheme.typography.headlineMedium)
        NoticeButton("Back", back)
        val list = entries.getOrNull()
        val entry = selected?.let { list?.getOrNull(it) }
        val path = document
        when {
            list == null -> Text("License resources could not be opened. Use Back to return.")
            path != null -> key(path) {
                val text = remember(path) { runCatching { assets.legalText(path) } }
                if (text.isFailure) Text("This local notice could not be opened. Use Back to choose another notice.")
                else if (path.endsWith(".html")) LocalLegalHtml(path, assets, Modifier.weight(1f))
                else {
                    val scroll = rememberLazyListState()
                    val scope = rememberCoroutineScope()
                    LazyColumn(Modifier.weight(1f).onPreviewKeyEvent { event ->
                        val direction = when (event.key) { Key.DirectionDown -> 1; Key.DirectionUp -> -1; else -> 0 }
                        if (direction == 0) false else {
                            if (event.type == KeyEventType.KeyDown) scope.launch {
                                scroll.scrollBy(direction * scroll.layoutInfo.viewportSize.height * 0.65f)
                            }
                            true
                        }
                    }.focusable(), state = scroll, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    // Paragraphs keep long notices accessible without one enormous text layout.
                    items(text.getOrThrow().chunked(4000)) { Text(it, Modifier.fillMaxWidth(), style = MaterialTheme.typography.bodyLarge) }
                    }
                }
            }
            entry != null -> LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                item { Text(entry.title, style = MaterialTheme.typography.titleLarge) }
                if (entry.reference.isNotEmpty()) item { Text("Source reference: ${entry.reference}", style = MaterialTheme.typography.bodyLarge) }
                items(entry.paths) { resource -> NoticeButton(documentLabel(entry, resource, provenance)) { document = resource } }
                if (entry.paths.isEmpty()) item { Text("No local text is available for this component.") }
            }
            else -> LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(list.indices.toList()) { index -> NoticeButton(list[index].title) { selected = index } }
            }
        }
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable private fun LocalLegalHtml(path: String, assets: AssetManager, modifier: Modifier) {
    var failed by remember { mutableStateOf(false) }
    if (failed) { Text("This local notice could not be opened. Use Back to return."); return }
    AndroidView(modifier = modifier.fillMaxWidth(), factory = { context ->
        WebView(context).apply {
            settings.javaScriptEnabled = false
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.blockNetworkLoads = true
            settings.domStorageEnabled = false
            settings.builtInZoomControls = true
            settings.displayZoomControls = false
            settings.textZoom = (100 * context.resources.configuration.fontScale).toInt().coerceAtLeast(100)
            webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    // Only explicit in-document anchors may navigate. Source URLs remain displayed text.
                    val uri = request.url
                    return !(uri.scheme == "https" && uri.host == "legal.invalid" &&
                        uri.encodedPath == "/$path" && uri.fragment != null)
                }
                override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse {
                    val uri = request.url
                    val data = runCatching {
                        require(uri.scheme == "https" && uri.host == "legal.invalid" && uri.port == -1 && uri.query == null)
                        val resource = legalPath("", uri.encodedPath!!.removePrefix("/"))
                        assets.open("legal/$resource").use { it.readBytes() }
                    }.getOrElse { ByteArray(0) }
                    val mime = when { uri.path.orEmpty().endsWith(".css") -> "text/css"; uri.path.orEmpty().endsWith(".html") -> "text/html"; else -> "text/plain" }
                    return WebResourceResponse(mime, "UTF-8", ByteArrayInputStream(data))
                }
            }
            runCatching { loadDataWithBaseURL("https://legal.invalid/$path", assets.legalText(path), "text/html", "UTF-8", null) }
                .onFailure { failed = true }
        }
    }, onRelease = { it.stopLoading(); it.destroy() })
}
