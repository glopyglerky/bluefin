//! SyncPlay controls for the independently authenticated player session.
//! Requests, including the post-join admission read, use the existing transport.

use crate::protocol::{self, Result, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RoomInfo {
    pub group_id: Uuid,
    pub group_name: String,
    pub state: RoomPhase,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum RoomPhase {
    Idle,
    Waiting,
    Paused,
    Playing,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RoomItem {
    pub item_id: Uuid,
    pub playlist_item_id: Uuid,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Timeline {
    pub group_id: Uuid,
    pub state: RoomPhase,
    pub item: Option<RoomItem>,
    pub position_ticks: u64,
    pub anchor_time: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RoomRequest {
    List,
    Current,
    Create {
        name: String,
        item: Uuid,
        position_ticks: u64,
    },
    Join {
        group: Uuid,
    },
    Leave,
    Pause,
    Resume,
    Seek {
        position_ticks: u64,
    },
    Ready {
        playlist_item: Uuid,
        position_ticks: u64,
        when: String,
        playing: bool,
    },
    Buffering {
        playlist_item: Uuid,
        position_ticks: u64,
        when: String,
        playing: bool,
    },
}

async fn call(session: &Session, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    protocol::request(&session.base, &session.token, method, path, &[], body).await
}

async fn current(session: &Session) -> Result<Option<Timeline>> {
    let value = call(session, "GET", "Bluefin/SyncPlay/Current", None).await?;
    let state: Option<Timeline> =
        serde_json::from_value(value).map_err(|_| "Invalid room timeline.")?;
    if let Some(timeline) = &state {
        if timeline.group_id.is_nil()
            || timeline.anchor_time.len() > 40
            || timeline
                .item
                .as_ref()
                .is_some_and(|item| item.item_id.is_nil() || item.playlist_item_id.is_nil())
        {
            return Err("Invalid room timeline.".into());
        }
    }
    Ok(state)
}

async fn joined(session: &Session, group: Uuid) -> Result<Timeline> {
    // Join can return 204 and still withhold admission when the server denies
    // library access. Only the caller's own joined-session timeline proves it.
    current(session)
        .await?
        .filter(|room| room.group_id == group)
        .ok_or_else(|| "This account could not join the room.".into())
}

fn tick_range(ticks: u64) -> Result<u64> {
    if ticks > i64::MAX as u64 {
        return Err("Playback position is out of range.".into());
    }
    Ok(ticks)
}

async fn post(session: &Session, resource: &str, body: Option<Value>) -> Result<Value> {
    call(session, "POST", &format!("SyncPlay/{resource}"), body).await
}

async fn report(
    session: &Session,
    ready: bool,
    playlist_item: Uuid,
    position_ticks: u64,
    when: String,
    playing: bool,
) -> Result<Value> {
    if playlist_item.is_nil() || !(20..=40).contains(&when.len()) || !when.is_ascii() {
        return Err("Invalid room playback observation.".into());
    }
    let resource = if ready { "Ready" } else { "Buffering" };
    let body = json!({
        "PlaylistItemId": playlist_item,
        "PositionTicks": tick_range(position_ticks)?,
        "When": when,
        "IsPlaying": playing,
    });
    post(session, resource, Some(body)).await
}

pub async fn request(session: &Session, operation: RoomRequest) -> Result<Value> {
    match operation {
        RoomRequest::List => {
            let result = call(session, "GET", "SyncPlay/List", None).await?;
            let rooms: Vec<RoomInfo> =
                serde_json::from_value(result).map_err(|_| "Invalid room list.")?;
            if rooms.len() > 200
                || rooms
                    .iter()
                    .any(|room| room.group_id.is_nil() || room.group_name.len() > 800)
            {
                return Err("Invalid room list.".into());
            }
            Ok(json!(rooms))
        }
        RoomRequest::Current => Ok(json!(current(session).await?)),
        RoomRequest::Join { group } => {
            if group.is_nil() {
                return Err("Invalid room code.".into());
            }
            post(session, "Join", Some(json!({ "GroupId": group }))).await?;
            Ok(json!(joined(session, group).await?))
        }
        RoomRequest::Create {
            name,
            item,
            position_ticks,
        } => {
            let group_name = name.trim();
            if group_name.is_empty() || group_name.encode_utf16().count() > 200 || item.is_nil() {
                return Err("Choose an authorized item and enter a room name.".into());
            }
            let position = tick_range(position_ticks)?;
            let reply = post(session, "New", Some(json!({ "GroupName": group_name }))).await?;
            let group: RoomInfo =
                serde_json::from_value(reply).map_err(|_| "Invalid room response.")?;
            let _ = joined(session, group.group_id).await?;
            post(
                session,
                "SetNewQueue",
                Some(json!({
                    "PlayingQueue": [item],
                    "PlayingItemPosition": 0,
                    "StartPositionTicks": position,
                })),
            )
            .await?;
            let room = joined(session, group.group_id).await?;
            if room
                .item
                .as_ref()
                .is_none_or(|current| current.item_id != item)
            {
                return Err("The server did not admit this item to the room.".into());
            }
            Ok(json!(room))
        }
        RoomRequest::Pause => post(session, "Pause", None).await,
        RoomRequest::Resume => post(session, "Unpause", None).await,
        RoomRequest::Leave => post(session, "Leave", None).await,
        RoomRequest::Seek { position_ticks } => {
            let body = json!({ "PositionTicks": tick_range(position_ticks)? });
            post(session, "Seek", Some(body)).await
        }
        RoomRequest::Ready {
            playlist_item,
            position_ticks,
            when,
            playing,
        } => report(session, true, playlist_item, position_ticks, when, playing).await,
        RoomRequest::Buffering {
            playlist_item,
            position_ticks,
            when,
            playing,
        } => report(session, false, playlist_item, position_ticks, when, playing).await,
    }
}
