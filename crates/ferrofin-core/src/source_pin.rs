//! An opened local generation for a private playback manager. No writer locks.
//!
//! The caller must bind the returned manager into its private planner and every
//! media route. A marker guard beside a router using the original manager does
//! not pin dispatch. Linux proc-descriptor paths are server-only capabilities.
use std::collections::HashMap;

use async_trait::async_trait;
use ferrofin_db::{entities::base_items::BaseItemEntity, source_pin::OpenedSourceGeneration};
use ferrofin_model::{dto::MediaSourceInfo, entities::VideoType, media_info::MediaProtocol};
use ferrofin_traits::error::ServiceError;
use uuid::Uuid;

use crate::video_versions::{VersionRowReader, VersionRows};

mod authorization;

pub(crate) struct PinnedInput {
    pub captured: OpenedSourceGeneration,
    pub source: MediaSourceInfo,
    pub item_id: Uuid,
    pub source_id: Uuid,
    pub viewer: Uuid,
    anchors: Vec<authorization::LibraryAnchor>,
}

impl PinnedInput {
    pub fn accepts(&self, id: Uuid) -> bool {
        id == self.item_id || id == self.source_id
    }
}

/// Compose the same production version-group rules over one database snapshot.
pub(crate) async fn prepare(
    captured: OpenedSourceGeneration,
    item_id: Uuid,
    source_id: Uuid,
    viewer: Uuid,
    mut source: MediaSourceInfo,
) -> Result<PinnedInput, ServiceError> {
    let versions =
        VersionRows::load(&CapturedVersions(&captured.versions), &[&captured.item]).await?;
    if !versions
        .items_for_media_sources(&captured.item)
        .iter()
        .any(|(row, _)| Uuid::parse_str(&row.id).ok() == Some(source_id))
    {
        return Err(ServiceError::conflict(
            "source is outside the captured version group",
        ));
    }
    // Ordinary audio lyric sidecars are metadata read through the separately
    // authorized lyrics route, never decoder inputs. Keep every captured stream
    // row for authority/generation checks; omit only lyrics from execution.
    if captured.source.media_type.as_deref() == Some("Audio") {
        source.media_streams.retain(|stream| {
            stream.stream_type != ferrofin_model::entities::MediaStreamType::Lyric
        });
    }
    let path = captured.source.path.as_deref().unwrap_or_default();
    let data = crate::item_data::parse_data(captured.source.data.as_deref());
    if source.protocol != MediaProtocol::File
        || source.is_remote
        || source.live_stream_id.is_some()
        || source.open_token.is_some()
        || source.is_infinite_stream
        || source.requires_opening
        || !std::path::Path::new(path).is_absolute()
        || crate::shortcut::is_strm(path)
        || crate::shortcut::path_protocol(path) != MediaProtocol::File
        || !matches!(
            captured.source.media_type.as_deref(),
            Some("Audio" | "Video")
        )
        || crate::item_type_lookup::kind_from_type_name(&captured.source.type_).is_none_or(|kind| {
            !matches!(
                kind,
                ferrofin_model::data::BaseItemKind::Movie
                    | ferrofin_model::data::BaseItemKind::Episode
                    | ferrofin_model::data::BaseItemKind::Video
                    | ferrofin_model::data::BaseItemKind::Audio
                    | ferrofin_model::data::BaseItemKind::MusicVideo
                    | ferrofin_model::data::BaseItemKind::Trailer
            )
        })
        || source.run_time_ticks.is_none_or(|ticks| ticks <= 0)
        || source
            .video_type
            .is_some_and(|kind| kind != VideoType::VideoFile)
        || data.get("IsShortcut").and_then(serde_json::Value::as_bool) == Some(true)
        || !crate::video_versions::additional_parts(captured.source.data.as_deref()).is_empty()
        || source
            .media_streams
            .iter()
            .any(|stream| stream.is_external || stream.path.is_some())
        || !source.media_streams.iter().any(|stream| {
            matches!(
                (captured.source.media_type.as_deref(), stream.stream_type),
                (
                    Some("Audio"),
                    ferrofin_model::entities::MediaStreamType::Audio
                ) | (
                    Some("Video"),
                    ferrofin_model::entities::MediaStreamType::Video
                )
            )
        })
    {
        return Err(ServiceError::unavailable(
            "source is not a supported finite local input",
        ));
    }
    let anchors = authorization::anchors(&captured)?;
    source.path = Some(captured.input_path()?);
    // There is exactly one opened input. Never probe or open an auxiliary URL.
    source.supports_probing = false;
    source.path_is_private = true;
    Ok(PinnedInput {
        captured,
        source,
        item_id,
        source_id,
        viewer,
        anchors,
    })
}

struct CapturedVersions<'a>(&'a [BaseItemEntity]);

#[async_trait]
impl VersionRowReader for CapturedVersions<'_> {
    async fn rows_by_id(&self, ids: &[Uuid]) -> Result<Vec<BaseItemEntity>, ServiceError> {
        Ok(self
            .0
            .iter()
            .filter(|row| Uuid::parse_str(&row.id).is_ok_and(|id| ids.contains(&id)))
            .cloned()
            .collect())
    }

    async fn rows_by_primary(
        &self,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BaseItemEntity>>, ServiceError> {
        let mut map: HashMap<Uuid, Vec<BaseItemEntity>> = HashMap::new();
        for row in self.0 {
            if let Some(primary) = row
                .primary_version_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok())
                && ids.contains(&primary)
                && Uuid::parse_str(&row.id).ok() != Some(primary)
            {
                map.entry(primary).or_default().push(row.clone());
            }
        }
        Ok(map)
    }
}
