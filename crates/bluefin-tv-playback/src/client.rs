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
            let (provider, kind) = direct_destination(&url).ok_or("unsupported provider link")?;
            Ok(json!({"url":url,"provider":provider,"kind":kind}))
        }
        Command::Seek {
            position,
            delta,
            duration,
        } => Ok(json!(position.saturating_add_signed(delta).min(duration))),
    }
}

fn direct_destination(url: &str) -> Option<(&'static str, &'static str)> {
    let alphanumeric = |id: &str| id.bytes().all(|ch| ch.is_ascii_alphanumeric());
    let number = |id: &str| (6..=8).contains(&id.len()) && id.bytes().all(|ch| ch.is_ascii_digit());
    let slug = |id: &str| {
        (1..=200).contains(&id.len())
            && id.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
            })
    };
    let uuid = |id: &str| {
        id.len() == 36
            && id.bytes().enumerate().all(|(i, ch)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    ch == b'-'
                } else {
                    ch.is_ascii_hexdigit()
                }
            })
    };
    let opaque = |id: &str| {
        id.bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
    };
    let routes: [(&str, &str, &str, &dyn Fn(&str) -> bool); 17] = [
        (
            "https://www.youtube.com/watch?v=",
            "YouTube",
            "player",
            &|id| id.len() == 11 && opaque(id),
        ),
        (
            "https://www.netflix.com/watch/",
            "Netflix",
            "player",
            &number,
        ),
        (
            "https://www.netflix.com/title/",
            "Netflix",
            "title",
            &number,
        ),
        (
            "https://www.disneyplus.com/browse/entity-",
            "Disney+",
            "title",
            &uuid,
        ),
        (
            "https://www.disneyplus.com/browse/page-",
            "Disney+",
            "title",
            &uuid,
        ),
        (
            "https://www.disneyplus.com/movies/wd/",
            "Disney+",
            "title",
            &|id| id.len() == 12 && alphanumeric(id),
        ),
        (
            "https://www.disneyplus.com/series/wp/",
            "Disney+",
            "title",
            &|id| id.len() == 12 && alphanumeric(id),
        ),
        ("https://www.hulu.com/movie/", "Hulu", "title", &uuid),
        ("https://www.hulu.com/series/", "Hulu", "title", &uuid),
        (
            "https://www.paramountplus.com/shows/video/",
            "Paramount+",
            "player",
            &|id| {
                id.strip_suffix('/')
                    .is_some_and(|id| (8..=80).contains(&id.len()) && opaque(id))
            },
        ),
        (
            "https://www.peacocktv.com/stream-tv/",
            "Peacock",
            "title",
            &slug,
        ),
        (
            "https://www.peacocktv.com/stream-movies/",
            "Peacock",
            "title",
            &slug,
        ),
        (
            "https://www.amazon.com/gp/video/detail/",
            "Prime Video",
            "title",
            &|id| {
                id.len() == 10
                    && id.starts_with('B')
                    && id
                        .bytes()
                        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
            },
        ),
        ("https://www.hbomax.com/movie/", "HBO Max", "title", &uuid),
        ("https://www.hbomax.com/show/", "HBO Max", "title", &uuid),
        ("https://tubitv.com/movies/", "Tubi", "title", &|id| {
            (1..=12).contains(&id.len())
                && !id.starts_with('0')
                && id.bytes().all(|ch| ch.is_ascii_digit())
        }),
        ("https://tubitv.com/series/", "Tubi", "title", &|id| {
            (1..=12).contains(&id.len())
                && !id.starts_with('0')
                && id.bytes().all(|ch| ch.is_ascii_digit())
        }),
    ];
    routes
        .into_iter()
        .find_map(|(prefix, provider, kind, valid)| {
            url.strip_prefix(prefix)
                .filter(|id| valid(id))
                .map(|_| (provider, kind))
        })
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
        assert_eq!(
            apply(br#"{"op":"handoff","url":"https://www.netflix.com/watch/80234304"}"#).unwrap()["kind"],
            "player"
        );
        assert_eq!(
            apply(br#"{"op":"handoff","url":"https://www.peacocktv.com/stream-movies/psych-3-this-is-gus"}"#).unwrap()["provider"],
            "Peacock"
        );
        for invalid in [
            "https://www.netflix.com/watch/80234304?token=secret",
            "https://www.netflix.com@unrelated.test/watch/80234304",
            "https://www.paramountplus.com/shows/video/abc12345/extra/",
            "https://tv.apple.com/movie/umc.cmc.intermediary",
        ] {
            assert_eq!(direct_destination(invalid), None);
        }
    }
}
