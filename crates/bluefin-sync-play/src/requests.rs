//! SyncPlay controls for the independently authenticated player session.
//! Requests, including the post-join admission read, use the existing transport.

use alloc::{boxed::Box, format, string::String, vec::Vec};
use core::{future::Future, pin::Pin};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

type Result<T> = core::result::Result<T, String>;

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
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RoomRequest {
    List,
    Current,
    Clock,
    Create {
        name: String,
        item: Uuid,
        position_ticks: u64,
    },
    Join {
        group: Uuid,
    },
    Leave {
        group: Uuid,
    },
    Pause {
        group: Uuid,
    },
    Resume {
        group: Uuid,
    },
    Seek {
        group: Uuid,
        position_ticks: u64,
    },
    Ready {
        group: Uuid,
        playlist_item: Uuid,
        position_ticks: u64,
        when: String,
        playing: bool,
    },
    Buffering {
        group: Uuid,
        playlist_item: Uuid,
        position_ticks: u64,
        when: String,
        playing: bool,
    },
}

pub type Requester<'a> = dyn FnMut(&'static str, String, Option<Value>) -> Pin<Box<dyn Future<Output = Result<Value>>>>
    + 'a;

async fn current(send: &mut Requester<'_>) -> Result<Option<Timeline>> {
    let value = send("GET", "Bluefin/SyncPlay/Current".into(), None).await?;
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

async fn joined(send: &mut Requester<'_>, group: Uuid) -> Result<Timeline> {
    // Join can return 204 and still withhold admission when the server denies
    // library access. Only the caller's own joined-session timeline proves it.
    current(send)
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

async fn post(send: &mut Requester<'_>, resource: &str, body: Option<Value>) -> Result<Value> {
    send("POST", format!("SyncPlay/{resource}"), body).await
}

async fn post_control(
    send: &mut Requester<'_>,
    group: Uuid,
    action: &str,
    data: Option<Value>,
) -> Result<Value> {
    if group.is_nil() {
        return Err("Invalid room code.".into());
    }
    let mut body = json!({ "Action": action });
    if let Some(data) = data {
        body["Data"] = data;
    }
    send(
        "POST",
        format!("Bluefin/SyncPlay/{group}/Control"),
        Some(body),
    )
    .await
}

async fn report(
    send: &mut Requester<'_>,
    ready: bool,
    group: Uuid,
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
    post_control(send, group, resource, Some(body)).await
}

pub async fn request_using(operation: RoomRequest, send: &mut Requester<'_>) -> Result<Value> {
    match operation {
        RoomRequest::List => {
            let result = send("GET", "SyncPlay/List".into(), None).await?;
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
        RoomRequest::Current => Ok(json!(current(send).await?)),
        RoomRequest::Clock => send("GET", "GetUtcTime".into(), None).await,
        RoomRequest::Join { group } => {
            if group.is_nil() {
                return Err("Invalid room code.".into());
            }
            post(send, "Join", Some(json!({ "GroupId": group }))).await?;
            Ok(json!(joined(send, group).await?))
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
            let reply = post(send, "New", Some(json!({ "GroupName": group_name }))).await?;
            let group: RoomInfo =
                serde_json::from_value(reply).map_err(|_| "Invalid room response.")?;
            let _ = joined(send, group.group_id).await?;
            post_control(
                send,
                group.group_id,
                "SetNewQueue",
                Some(json!({
                    "PlayingQueue": [item],
                    "PlayingItemPosition": 0,
                    "StartPositionTicks": position,
                })),
            )
            .await?;
            let room = joined(send, group.group_id).await?;
            if room
                .item
                .as_ref()
                .is_none_or(|current| current.item_id != item)
            {
                return Err("The server did not admit this item to the room.".into());
            }
            Ok(json!(room))
        }
        RoomRequest::Pause { group } => post_control(send, group, "Pause", None).await,
        RoomRequest::Resume { group } => post_control(send, group, "Unpause", None).await,
        RoomRequest::Leave { group } => post_control(send, group, "Leave", None).await,
        RoomRequest::Seek {
            group,
            position_ticks,
        } => {
            let body = json!({ "PositionTicks": tick_range(position_ticks)? });
            post_control(send, group, "Seek", Some(body)).await
        }
        RoomRequest::Ready {
            group,
            playlist_item,
            position_ticks,
            when,
            playing,
        } => {
            report(
                send,
                true,
                group,
                playlist_item,
                position_ticks,
                when,
                playing,
            )
            .await
        }
        RoomRequest::Buffering {
            group,
            playlist_item,
            position_ticks,
            when,
            playing,
        } => {
            report(
                send,
                false,
                group,
                playlist_item,
                position_ticks,
                when,
                playing,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::{
        future::ready,
        task::{Context, Poll, Waker},
    };

    type Exchange = (&'static str, String, Option<Value>, Result<Value>);

    fn run(operation: RoomRequest, exchanges: Vec<Exchange>) -> Result<Value> {
        let mut exchanges = exchanges.into_iter();
        let result = {
            let mut send = |method: &'static str, path: String, body: Option<Value>| {
                let (expected_method, expected_path, expected_body, reply) =
                    exchanges.next().expect("Unexpected room request");
                assert_eq!(
                    (method, path, body),
                    (expected_method, expected_path, expected_body)
                );
                Box::pin(ready(reply)) as Pin<Box<dyn Future<Output = Result<Value>>>>
            };
            let mut future = Box::pin(request_using(operation, &mut send));
            match future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("Unexpected deferred callback"),
            }
        };
        assert!(
            exchanges.next().is_none(),
            "Expected room request was not sent"
        );
        result
    }

    fn timeline(group: Uuid, item: Option<Uuid>) -> Value {
        json!({
            "GroupId": group, "State": "Idle", "PositionTicks": 0,
            "AnchorTime": "2026-11-27T00:00:00Z",
            "Item": item.map(|id| json!({
                "ItemId": id, "PlaylistItemId": Uuid::from_u128(4),
            })),
        })
    }

    #[test]
    fn join_requires_admission_to_the_requested_room() {
        let group = Uuid::from_u128(1);
        for value in [Value::Null, timeline(Uuid::from_u128(2), None)] {
            let exchanges = vec![
                (
                    "POST",
                    "SyncPlay/Join".into(),
                    Some(json!({"GroupId": group})),
                    Ok(Value::Null),
                ),
                ("GET", "Bluefin/SyncPlay/Current".into(), None, Ok(value)),
            ];
            assert!(run(RoomRequest::Join { group }, exchanges).is_err());
        }
    }

    #[test]
    fn create_requires_the_exact_authorized_item() {
        let group = Uuid::from_u128(1);
        let item = Uuid::from_u128(2);
        for approved in [item, Uuid::from_u128(3)] {
            let created = json!({"GroupId": group, "GroupName": "Room", "State": "Idle"});
            let exchanges = vec![
                (
                    "POST",
                    "SyncPlay/New".into(),
                    Some(json!({"GroupName":"Room"})),
                    Ok(created),
                ),
                (
                    "GET",
                    "Bluefin/SyncPlay/Current".into(),
                    None,
                    Ok(timeline(group, None)),
                ),
                (
                    "POST",
                    format!("Bluefin/SyncPlay/{group}/Control"),
                    Some(json!({
                        "Action":"SetNewQueue",
                        "Data":{"PlayingQueue":[item],"PlayingItemPosition":0,"StartPositionTicks":0},
                    })),
                    Ok(Value::Null),
                ),
                (
                    "GET",
                    "Bluefin/SyncPlay/Current".into(),
                    None,
                    Ok(timeline(group, Some(approved))),
                ),
            ];
            let command = RoomRequest::Create {
                name: "Room".into(),
                item,
                position_ticks: 0,
            };
            assert_eq!(run(command, exchanges).is_ok(), approved == item);
        }
    }

    #[test]
    fn scoped_commands_preserve_a_stale_membership_denial() {
        let group = Uuid::from_u128(1);
        for (command, action) in [
            (RoomRequest::Pause { group }, "Pause"),
            (RoomRequest::Leave { group }, "Leave"),
        ] {
            let exchanges = vec![(
                "POST",
                format!("Bluefin/SyncPlay/{group}/Control"),
                Some(json!({"Action":action})),
                Err("Room membership changed.".into()),
            )];
            assert_eq!(
                run(command, exchanges),
                Err("Room membership changed.".into())
            );
        }
        assert!(run(RoomRequest::Pause { group: Uuid::nil() }, vec![]).is_err());
    }
}
