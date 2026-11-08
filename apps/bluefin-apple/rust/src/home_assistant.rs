//! Explicit, user-configured Home Assistant connection. No media credentials are reused.
use super::{err, response_json, text, Api, Method, Result};
use serde_json::{json, Value};

fn entity_id(id: &str) -> bool {
    id.strip_prefix("media_player.").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

pub(super) fn call(value: &Value, operation: &str) -> Result<Value> {
    let api = Api::new(value)?;
    let token = text(value, "token")?;
    let mut request = if operation == "homeAssistantStates" {
        api.client.get(api.url(&["api", "states"])?)
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
    if operation == "homeAssistantControl" {
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
            Some(json!({"entityId":id,"name":name,"state":status,"title":title,"volume":volume}))
        })
        .take(200)
        .collect::<Vec<_>>();
    Ok(json!(players))
}
