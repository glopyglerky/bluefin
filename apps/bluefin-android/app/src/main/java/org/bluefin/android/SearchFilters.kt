package org.bluefin.android

import org.json.JSONArray
import org.json.JSONObject

enum class MediaSearchScope(val title: String, val wire: String, val types: Set<String>) {
    ALL("All media", "all", emptySet()),
    MOVIES("Movies", "movies", setOf("Movie")),
    SHOWS("TV Shows", "tvShows", setOf("Series", "Season", "Episode")),
    MUSIC("Music", "music", setOf("Audio", "MusicAlbum", "MusicArtist", "MusicVideo", "Playlist")),
    LIVE("Live TV", "liveTv", setOf("TvChannel", "LiveTvProgram"));
}
data class NamedSearchFilter(val id: String, val name: String)
data class MediaSearchFilters(
    val scope: MediaSearchScope = MediaSearchScope.ALL,
    val studio: NamedSearchFilter? = null, val tag: String? = null,
    val collection: NamedSearchFilter? = null
) {
    fun fields(term: String, start: Int) = JSONObject()
        .put("searchTerm", term.trim().take(128)).put("searchScope", scope.wire)
        .put("limit", 50).put("startIndex", start).apply {
            studio?.let { put("studioIds", JSONArray().put(it.id)) }
            tag?.let { put("tags", JSONArray().put(it)) }
            collection?.let { put("parentId", it.id) }
        }
}
data class MediaSearchFacets(
    val studios: List<NamedSearchFilter> = emptyList(), val tags: List<String> = emptyList(),
    val collections: List<NamedSearchFilter> = emptyList(), val truncated: Boolean = false
) {
    companion object {
        fun parse(value: JSONObject): MediaSearchFacets {
            fun named(key: String): List<NamedSearchFilter> {
                val array = value.optJSONArray(key) ?: JSONArray()
                return (0 until minOf(array.length(), 200)).mapNotNull {
                    val item = array.optJSONObject(it) ?: return@mapNotNull null
                    val id = item.optString("id"); val name = item.optString("name")
                    if (id.matches(Regex("[A-Za-z0-9-]{1,128}")) && name.isNotBlank())
                        NamedSearchFilter(id, name.take(256)) else null
                }
            }
            val tags = value.optJSONArray("tags") ?: JSONArray()
            return MediaSearchFacets(named("studios"),
                (0 until minOf(tags.length(), 512)).map { tags.optString(it) }.filter { it.isNotBlank() },
                named("collections"), value.optBoolean("truncated"))
        }
    }
}
data class HomeShelf(val title: String, val items: List<LibraryItem>)
