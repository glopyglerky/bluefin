//! Portable policy used by the web-runtime TV clients through the raw WASM ABI.
use crate::{LeaseRequest, MediaKind, PlaybackOptions, VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientState {
    pub queue: Vec<String>,
    pub cursor: usize,
    pub position_ticks: u64,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    Lease {
        item: String,
        kind: MediaKind,
        #[serde(rename = "startTicks")]
        start_ticks: u64,
    },
    Queue {
        state: ClientState,
        action: QueueAction,
        item: Option<String>,
    },
    Guide {
        now: i64,
        programs: Vec<Program>,
    },
    Handoff {
        url: String,
    },
    Seek {
        position: u64,
        delta: i64,
        duration: u64,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QueueAction {
    Add,
    Next,
    Previous,
    Clear,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Program {
    pub id: String,
    pub channel: String,
    pub name: String,
    pub start: i64,
    pub end: i64,
}
pub fn apply(input: &[u8]) -> Result<Value, &'static str> {
    if input.len() > 256 * 1024 {
        return Err("input limit");
    }
    let command: Command = serde_json::from_slice(input).map_err(|_| "invalid command")?;
    match command {
        Command::Lease {
            item,
            kind,
            start_ticks,
        } => {
            let request = LeaseRequest {
                version: VERSION,
                item_id: item.replace('-', "").to_ascii_lowercase(),
                playback: PlaybackOptions {
                    kind,
                    max_bitrate: if kind == MediaKind::Audio {
                        320_000
                    } else {
                        8_000_000
                    },
                    start_ticks,
                    audio_stream_index: None,
                    subtitle_stream_index: None,
                },
            };
            if !request.valid() {
                return Err("invalid playback");
            }
            serde_json::to_value(request).map_err(|_| "encoding")
        }
        Command::Queue {
            mut state,
            action,
            item,
        } => {
            if state.queue.len() > 200
                || (state.queue.is_empty() && state.cursor != 0)
                || (!state.queue.is_empty() && state.cursor >= state.queue.len())
                || state.queue.iter().any(|i| !valid_id(i))
            {
                return Err("invalid queue");
            }
            match action {
                QueueAction::Add => {
                    let item = item.ok_or("missing item")?;
                    if !valid_id(&item) || state.queue.len() == 200 {
                        return Err("queue limit");
                    }
                    state.queue.push(item);
                }
                QueueAction::Next => {
                    if !state.queue.is_empty() {
                        state.cursor = (state.cursor + 1).min(state.queue.len() - 1);
                    }
                    state.position_ticks = 0;
                }
                QueueAction::Previous => {
                    state.cursor = state.cursor.saturating_sub(1);
                    state.position_ticks = 0;
                }
                QueueAction::Clear => state = ClientState::default(),
            }
            serde_json::to_value(state).map_err(|_| "encoding")
        }
        Command::Guide { now, programs } => {
            if programs.len() > 1000 {
                return Err("guide limit");
            }
            let mut current: Vec<_> = programs
                .into_iter()
                .filter(|p| {
                    p.start <= now
                        && now < p.end
                        && p.end > p.start
                        && p.name.len() <= 512
                        && p.channel.len() <= 128
                })
                .collect();
            current.sort_by(|a, b| a.channel.cmp(&b.channel).then(a.start.cmp(&b.start)));
            Ok(json!(current))
        }
        Command::Handoff { url } => {
            // Exact public provider watch URL, no arbitrary scheme, credentials,
            // redirects, fragments, tracking parameters or caller-selected apps.
            let prefix = "https://www.youtube.com/watch?v=";
            let id = url
                .strip_prefix(prefix)
                .ok_or("unsupported provider link")?;
            if id.len() != 11
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            {
                return Err("invalid provider link");
            }
            Ok(json!({"url":url,"provider":"YouTube"}))
        }
        Command::Seek {
            position,
            delta,
            duration,
        } => Ok(json!(position.saturating_add_signed(delta).min(duration))),
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_rejects_unknown_and_urls() {
        assert!(
            apply(br#"{"op":"lease","item":"https://bad","kind":"video","startTicks":0}"#).is_err()
        );
        assert!(apply(br#"{"op":"lease","item":"00000000000000000000000000000001","kind":"video","startTicks":0,"userId":"admin"}"#).is_err());
    }
    #[test]
    fn guide_boundary_and_seek_are_deterministic() {
        assert_eq!(
            apply(br#"{"op":"seek","position":5,"delta":-10,"duration":100}"#).unwrap(),
            json!(0)
        );
        let p = br#"{"op":"guide","now":20,"programs":[{"id":"a","channel":"One","name":"Original fixture","start":10,"end":20},{"id":"b","channel":"One","name":"Current fixture","start":20,"end":30}]}"#;
        assert_eq!(apply(p).unwrap()[0]["id"], "b");
    }
    #[test]
    fn handoffs_cannot_carry_server_credentials() {
        assert!(apply(br#"{"op":"handoff","url":"https://www.youtube.com/watch?v=abcdefghijk&token=secret"}"#).is_err());
        assert!(
            apply(br#"{"op":"handoff","url":"https://www.youtube.com/watch?v=abcdefghijk"}"#)
                .is_ok()
        );
    }
}
