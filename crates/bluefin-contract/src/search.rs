//! Search categories shared by Bluefin clients and server discovery.
use serde::{Deserialize, Serialize};

/// Media-type refinement for an authenticated account search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScope {
    #[default]
    /// Every media type visible to the account.
    All,
    /// Feature films.
    Movies,
    /// Series, seasons, and episodes.
    TvShows,
    /// Music tracks, artists, albums, videos, and playlists.
    Music,
    /// Authorized channels and guide programs.
    LiveTv,
}

impl SearchScope {
    /// Item types sent to the server; no type restriction for a broad search.
    #[must_use]
    pub const fn library_item_types(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Movies => Some("Movie"),
            Self::TvShows => Some("Series,Season,Episode"),
            Self::Music => Some("Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist"),
            Self::LiveTv => Some("TvChannel,LiveTvProgram"),
        }
    }
    /// Stable wire value used by connected catalog search.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Movies => "movies",
            Self::TvShows => "tvShows",
            Self::Music => "music",
            Self::LiveTv => "liveTv",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_round_trip_and_cover_live_channels() {
        for scope in [SearchScope::All, SearchScope::Movies, SearchScope::TvShows,
                      SearchScope::Music, SearchScope::LiveTv] {
            assert_eq!(serde_json::to_value(scope).unwrap(), scope.wire());
            assert_eq!(serde_json::from_value::<SearchScope>(serde_json::json!(scope.wire())).unwrap(), scope);
        }
        assert!(SearchScope::LiveTv.library_item_types().unwrap().contains("TvChannel"));
        assert!(SearchScope::Music.library_item_types().unwrap().contains("Playlist"));
    }
}
