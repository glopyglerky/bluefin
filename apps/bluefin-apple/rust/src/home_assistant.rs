//! Explicit, user-configured Home Assistant connection. No media credentials are reused.
use super::{Api, Method, Result, err, response_json, text};
use serde_json::{Value, json};

fn entity_id(id: &str) -> bool {
    id.strip_prefix("media_player.").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

fn music_assistant_track_request(value: &Value) -> Result<Value> {
    let entity = text(value, "entityId")?;
    if !entity_id(entity) {
        return Err(err("Select one Music Assistant speaker"));
    }
    let track = text(value, "track")?.trim();
    if track.is_empty() || track.len() > 256 || track.contains("://") {
        return Err(err("Enter a track name"));
    }
    let mode = match value["mode"].as_str() {
        Some("play") => "play",
        Some("add") => "add",
        _ => return Err(err("Choose Play now or Add to queue")),
    };
    Ok(json!({"entity_id": entity, "media_id": track, "media_type": "track", "enqueue": mode}))
}

pub(super) fn call(value: &Value, operation: &str) -> Result<Value> {
    let api = Api::new(value)?;
    let token = text(value, "token")?;
    let mut request = if operation == "homeAssistantStates" {
        api.client.get(api.url(&["api", "states"])?)
    } else if operation == "musicAssistantPlayTrack" {
        api.client
            .post(api.url(&["api", "services", "music_assistant", "play_media"])?)
            .json(&music_assistant_track_request(value)?)
    } else {
        let service = text(value, "service")?;
        if !matches!(
            service,
            "media_play_pause" | "media_next_track" | "media_stop"
        ) {
            return Err(err("Unsupported Home Assistant media action"));
        }
        let entity = text(value, "entityId")?;
        if !entity_id(entity) {
            return Err(err("Select one Home Assistant media player"));
        }
        api.client
            .request(
                Method::POST,
                api.url(&["api", "services", "media_player", service])?,
            )
            .json(&json!({"entity_id":entity}))
    };
    request = request.bearer_auth(token);
    let response = request
        .send()
        .map_err(|_| err("Home Assistant request failed or timed out"))?;
    let result = response_json(response)?;
    if operation != "homeAssistantStates" {
        return Ok(Value::Null);
    }
    let states = result
        .as_array()
        .ok_or_else(|| err("Invalid Home Assistant state response"))?;
    let players = states
        .iter()
        .filter_map(|state| {
            let id = state["entity_id"].as_str()?;
            if !entity_id(id) {
                return None;
            }
            let attributes = &state["attributes"];
            let name: String = attributes["friendly_name"]
                .as_str()
                .unwrap_or(id)
                .chars()
                .take(200)
                .collect();
            let title = attributes["media_title"]
                .as_str()
                .map(|s| s.chars().take(500).collect::<String>());
            let status: String = state["state"]
                .as_str()
                .unwrap_or("unknown")
                .chars()
                .take(40)
                .collect();
            let volume = attributes["volume_level"]
                .as_f64()
                .filter(|v| v.is_finite() && (0.0..=1.0).contains(v));
            let music_assistant_speaker = attributes["device_class"] == "speaker"
                && attributes["mass_player_type"].as_str().is_some();
            Some(json!({
                "entityId": id,
                "name": name,
                "state": status,
                "title": title,
                "volume": volume,
                "musicAssistantSpeaker": music_assistant_speaker,
            }))
        })
        .take(200)
        .collect::<Vec<_>>();
    Ok(json!(players))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::mock;

    #[test]
    fn track_request_targets_one_speaker_with_an_explicit_queue_mode() {
        let mut input = json!({
            "entityId": "media_player.kitchen", "track": "  Original song  ", "mode": "add"
        });
        assert_eq!(
            music_assistant_track_request(&input).unwrap(),
            json!({
                "entity_id": "media_player.kitchen", "media_id": "Original song",
                "media_type": "track", "enqueue": "add"
            })
        );
        input["mode"] = json!("play");
        assert_eq!(
            music_assistant_track_request(&input).unwrap()["enqueue"],
            "play"
        );
        input["mode"] = json!("replace");
        assert!(music_assistant_track_request(&input).is_err());
    }

    #[test]
    fn track_request_rejects_broad_targets_and_urls() {
        let mut input = json!({"entityId": "all", "track": "Original song", "mode": "play"});
        assert!(music_assistant_track_request(&input).is_err());
        input["entityId"] = json!("media_player.kitchen");
        for invalid in [" ", "https://example.test/track", "music\ntrack"] {
            input["track"] = json!(invalid);
            assert!(music_assistant_track_request(&input).is_err());
        }
    }

    #[test]
    fn track_uses_the_configured_home_assistant_action() {
        let (server, worker) = mock("200 OK", "[]");
        let response = crate::call(&json!({
            "operation": "musicAssistantPlayTrack", "server": server, "allowLocalHttp": true,
            "token": "fixture-home-token", "entityId": "media_player.kitchen",
            "track": "Original song", "mode": "add"
        }))
        .unwrap();
        assert!(response.is_null());
        let request = worker.join().unwrap();
        let (header, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(header.starts_with("POST /jellyfin/api/services/music_assistant/play_media "));
        assert!(
            header
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-home-token")
        );
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap()["entity_id"],
            "media_player.kitchen"
        );
    }
}
