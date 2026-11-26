package org.bluefin.android

import org.json.JSONObject

data class CatalogArtwork(val id: String, val tag: String? = null)
fun catalogArtwork(value: JSONObject): List<CatalogArtwork> {
    val sources = mutableListOf<CatalogArtwork>()
    fun add(id: String?, tag: String?) {
        if (id != null && id.matches(Regex("[A-Za-z0-9_-]{1,128}")) &&
            tag != null && tag.isNotBlank() && tag.length <= 128 && tag.none { it.isISOControl() }) {
            val source = CatalogArtwork(id, tag)
            if (source !in sources) sources += source
        }
    }
    if (value.optString("Type") in setOf("Episode", "Season"))
        add(value.optString("SeriesId"), value.optString("SeriesPrimaryImageTag"))
    add(value.optString("Id"), value.optJSONObject("ImageTags")?.optString("Primary"))
    if (value.optString("Type") == "Audio") add(value.optString("AlbumId"), value.optString("AlbumPrimaryImageTag"))
    add(value.optString("ParentPrimaryImageItemId"), value.optString("ParentPrimaryImageTag"))
    if (sources.isEmpty()) {
        val id = value.optString("Id")
        if (id.matches(Regex("[A-Za-z0-9_-]{1,128}"))) sources += CatalogArtwork(id)
    }
    return sources
}
