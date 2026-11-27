package org.bluefin.android
import org.junit.Assert.*
import org.junit.Test
import org.json.JSONObject

class SearchAndArtworkTest {
    @Test fun publicTvLinksUseProviderIdentityAndKnownArtwork() {
        val page = JSONObject("""{"results":[
            {"id":47,"title":"A Stage","year":"2025","sourceUrl":"https://unrelated.test",
             "artwork":"https://static.tvmaze.com/uploads/images/medium_portrait/2/47.jpg"},
            {"id":47,"title":"Duplicate"},
            {"id":48,"title":"No poster","artwork":"https://unrelated.test/poster.jpg"}
        ]}""")
        val titles = PublicTvTitle.parse(page)
        assertEquals(listOf(47L, 48L), titles.map { it.id })
        assertEquals("https://www.tvmaze.com/shows/47", titles.first().url)
        assertNotNull(titles.first().artwork)
        assertNull(titles.last().artwork)
    }

    @Test fun laterSearchPagesRetainAllServerFacets() {
        val filters = MediaSearchFilters(MediaSearchScope.SHOWS, NamedSearchFilter("studio", "Studio"),
            "Service tag", NamedSearchFilter("collection", "Collection"))
        val page = filters.fields("  space  ", 50)
        assertEquals("tvShows", page.getString("searchScope"))
        assertEquals("space", page.getString("searchTerm"))
        assertEquals(50, page.getInt("startIndex"))
        assertEquals("studio", page.getJSONArray("studioIds").getString(0))
        assertEquals("Service tag", page.getJSONArray("tags").getString(0))
        assertEquals("collection", page.getString("parentId"))
        assertEquals("all", MediaSearchFilters().fields("anything", 0).getString("searchScope"))
    }
    @Test fun episodePosterUsesSeriesAndSongsInheritAlbumOnce() {
        val episode = JSONObject("""{"Id":"episode","Type":"Episode","SeriesId":"series","SeriesPrimaryImageTag":"poster","ImageTags":{"Primary":"still"}}""")
        assertEquals(listOf(CatalogArtwork("series", "poster"), CatalogArtwork("episode", "still")), catalogArtwork(episode))
        val song = JSONObject("""{"Id":"song","Type":"Audio","AlbumId":"album","AlbumPrimaryImageTag":"cover","ParentPrimaryImageItemId":"album","ParentPrimaryImageTag":"cover"}""")
        assertEquals(listOf(CatalogArtwork("album", "cover")), catalogArtwork(song))
        episode.put("SeriesId", "../other")
        assertEquals(CatalogArtwork("episode", "still"), catalogArtwork(episode).first())
    }
}
