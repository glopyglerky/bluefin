//! Portable PlaybackInfo planning, reporting and queue policy. No transport credentials in plans.
use crate::protocol::{self, Item, Result, Session};
use bluefin_contract::{QueueState, RepeatMode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub audio: Option<i32>,
    pub subtitle: Option<i32>,
    pub position: f64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Track {
    pub index: i32,
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub codec: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub item: Item,
    pub url: Url,
    pub prefix: String,
    pub play_session: String,
    pub media_source: String,
    pub live_stream: Option<String>,
    pub method: String,
    pub hls: bool,
    pub live: bool,
    pub offset: f64,
    pub selection: Selection,
    pub tracks: Vec<Track>,
}
#[derive(Clone, Copy)]
pub enum Platform {
    Native,
    Browser,
}

pub fn ticks(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds < 0.0 {
        return 0;
    }
    (seconds * 10_000_000.0).min(i64::MAX as f64) as u64
}
fn id(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 512
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
        .map(str::to_owned)
        .ok_or_else(|| format!("Missing or invalid {key}"))
}
/// A negotiated item prefix is the only upstream media authority. Strip legacy
/// credentials emitted by compatible servers; never use them as authentication.
pub fn pinned_url(base: &Url, prefix: &str, raw: &str) -> Result<Url> {
    if raw.len() > 16384 || raw.contains('\\') || raw.chars().any(char::is_control) {
        return Err("Invalid media URL".into());
    }
    let mut url = base.join(raw).map_err(|_| "Invalid media URL")?;
    if url.origin() != base.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url
            .path()
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
        || url.path().contains('%')
    {
        return Err("Media URL escaped the negotiated origin/item path".into());
    }
    let query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !protocol::credential(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url)
}
fn profile(platform: Platform) -> Value {
    #[cfg(target_arch = "wasm32")]
    if matches!(platform, Platform::Browser) {
        return browser_profile();
    }
    // Restrict browser claims to common HTML/MSE codecs. Decode failure is surfaced.
    let video = match platform {
        Platform::Native => "h264,hevc,vp8,vp9,av1,mpeg2video,mpeg4",
        Platform::Browser => "h264,vp8,vp9,av1",
    };
    let audio = match platform {
        Platform::Native => "aac,mp3,opus,vorbis,flac,alac,pcm_s16le,ac3,eac3",
        Platform::Browser => "aac,mp3,opus,vorbis,flac,pcm_s16le",
    };
    json!({"Name":"Bluefin Universal", "MaxStreamingBitrate":40000000,
        "DirectPlayProfiles":[{"Container": if matches!(platform,Platform::Native) {"mp4,m4v,mkv,webm,ts"}else{"mp4,m4v,webm"},"Type":"Video","VideoCodec":video,"AudioCodec":audio},
          {"Container":"mp3,aac,m4a,flac,wav,ogg,opus,webm","Type":"Audio","AudioCodec":audio}],
        "TranscodingProfiles":[{"Container":"ts","Type":"Video","VideoCodec":"h264","AudioCodec":"aac","Protocol":"hls","Context":"Streaming","MinSegments":2,"BreakOnNonKeyFrames":false},
          {"Container":"aac","Type":"Audio","AudioCodec":"aac","Protocol":"hls","Context":"Streaming"}],
        "SubtitleProfiles":[{"Format":"srt","Method":"Encode"},{"Format":"ass","Method":"Encode"},{"Format":"subrip","Method":"Encode"},{"Format":"pgssub","Method":"Encode"}]})
}
#[cfg(target_arch = "wasm32")]
fn browser_profile() -> Value {
    use wasm_bindgen::JsCast;
    let media = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.create_element("video").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlMediaElement>().ok());
    let supports = |mime: &str| {
        media
            .as_ref()
            .is_some_and(|m| !m.can_play_type(mime).is_empty())
    };
    let mut direct = Vec::new();
    for (container, mime, codec) in [
        ("mp4,m4v", "video/mp4; codecs=\"avc1.42E01E\"", "h264"),
        ("mp4,m4v", "video/mp4; codecs=\"hvc1.1.6.L93.B0\"", "hevc"),
        ("webm", "video/webm; codecs=\"vp8\"", "vp8"),
        ("webm", "video/webm; codecs=\"vp09.00.10.08\"", "vp9"),
        ("mp4", "video/mp4; codecs=\"av01.0.04M.08\"", "av1"),
    ] {
        if supports(mime) {
            let audio = if container == "webm" {
                [
                    ("opus", "audio/webm; codecs=\"opus\""),
                    ("vorbis", "audio/webm; codecs=\"vorbis\""),
                ]
                .into_iter()
                .filter(|(_, m)| supports(m))
                .map(|(c, _)| c)
                .collect::<Vec<_>>()
                .join(",")
            } else if supports("audio/mp4; codecs=\"mp4a.40.2\"") {
                "aac".into()
            } else {
                String::new()
            };
            direct.push(
                json!({"Container":container,"Type":"Video","VideoCodec":codec,"AudioCodec":audio}),
            );
        }
    }
    for (container, mime, codec) in [
        ("mp3", "audio/mpeg", "mp3"),
        ("aac", "audio/aac", "aac"),
        ("m4a,mp4", "audio/mp4; codecs=\"mp4a.40.2\"", "aac"),
        ("flac", "audio/flac", "flac"),
        ("wav", "audio/wav; codecs=\"1\"", "pcm_s16le"),
        ("ogg,opus", "audio/ogg; codecs=\"opus\"", "opus"),
    ] {
        if supports(mime) {
            direct.push(json!({"Container":container,"Type":"Audio","AudioCodec":codec}));
        }
    }
    let hls = supports("application/vnd.apple.mpegurl")
        || js_sys::Reflect::get(&js_sys::global(), &"Hls".into())
            .ok()
            .and_then(|h| {
                js_sys::Reflect::get(&h, &"isSupported".into())
                    .ok()
                    .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
                    .and_then(|f| f.call0(&h).ok())
            })
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let transcode = if hls {
        json!([
            {"Container":"ts","Type":"Video","VideoCodec":"h264","AudioCodec":"aac","Protocol":"hls","Context":"Streaming","MinSegments":2,"BreakOnNonKeyFrames":false},
            {"Container":"aac","Type":"Audio","AudioCodec":"aac","Protocol":"hls","Context":"Streaming"}
        ])
    } else {
        json!([])
    };
    json!({"Name":"Bluefin Browser","MaxStreamingBitrate":40000000,"DirectPlayProfiles":direct,"TranscodingProfiles":transcode,
        "SubtitleProfiles":[{"Format":"srt","Method":"Encode"},{"Format":"ass","Method":"Encode"},{"Format":"subrip","Method":"Encode"},{"Format":"pgssub","Method":"Encode"}]})
}
pub async fn negotiate(
    session: &Session,
    item: Item,
    selection: Selection,
    platform: Platform,
) -> Result<Plan> {
    if !item.playable() {
        return Err("Select a movie, episode, track or live channel".into());
    }
    if selection.audio.is_some_and(|n| !(0..=1024).contains(&n))
        || selection
            .subtitle
            .is_some_and(|n| !(-1..=1024).contains(&n))
    {
        return Err("Invalid stream selection".into());
    }
    let force = selection.audio.is_some() || selection.subtitle.is_some_and(|s| s >= 0);
    let info = protocol::request(&session.base, &session.token, "POST", &format!("Items/{}/PlaybackInfo",item.id), &[], Some(json!({"UserId":session.user,"IsPlayback":true,"AutoOpenLiveStream":true,
        "EnableDirectPlay":!force,"EnableDirectStream":false,"EnableTranscoding":true,"AudioStreamIndex":selection.audio,"SubtitleStreamIndex":selection.subtitle,
        "StartTimeTicks":ticks(selection.position),"MaxStreamingBitrate":40000000,"DeviceProfile":profile(platform)}))).await?;
    match plan(session, item, selection, &info) {
        Ok(plan) => {
            // AutoOpenLiveStream may prepare more than one source. Release only
            // unselected live resources, retaining the chosen transcode session.
            let mut failures = Vec::new();
            for source in info["MediaSources"]
                .as_array()
                .into_iter()
                .flatten()
                .take(32)
            {
                if source["LiveStreamId"].as_str() != plan.live_stream.as_deref()
                    && let Some(live) = source["LiveStreamId"].as_str()
                    && let Err(e) = close_live(session, live).await
                {
                    failures.push(e);
                }
            }
            if !failures.is_empty() {
                let _ = release(session, &plan).await;
                return Err("Unselected live-source cleanup was not acknowledged".into());
            }
            Ok(plan)
        }
        Err(error) => {
            let result = release_info(session, &info).await;
            Err(if result.is_ok() {
                error
            } else {
                format!("{error}; prepared playback cleanup was not acknowledged")
            })
        }
    }
}
pub fn plan(session: &Session, item: Item, selection: Selection, info: &Value) -> Result<Plan> {
    if info.get("ErrorCode").is_some_and(|v| !v.is_null()) {
        return Err("Server could not prepare playback".into());
    }
    let play_session = id(info, "PlaySessionId")?;
    let sources = info["MediaSources"]
        .as_array()
        .filter(|s| s.len() <= 32)
        .ok_or("Missing or excessive media sources")?;
    let source = sources
        .iter()
        .find(|s| s["SupportsDirectPlay"] == true || s["TranscodingUrl"].is_string())
        .ok_or("No negotiated playable source")?;
    let media_source = id(source, "Id")?;
    let live_stream = source["LiveStreamId"].as_str().map(str::to_owned);
    let live = item.kind == "TvChannel" || live_stream.is_some();
    let prefix = format!(
        "{}{}/{}/",
        session.base.path(),
        if item.kind == "Audio" {
            "Audio"
        } else {
            "Videos"
        },
        item.id.simple()
    );
    let force = selection.audio.is_some() || selection.subtitle.is_some_and(|s| s >= 0);
    let (url, method) = if !live && !force && source["SupportsDirectPlay"] == true {
        let mut url = session
            .base
            .join(&format!(
                "{}/{}/stream",
                if item.kind == "Audio" {
                    "Audio"
                } else {
                    "Videos"
                },
                item.id.simple()
            ))
            .map_err(|_| "Invalid stream URL")?;
        url.query_pairs_mut()
            .append_pair("Static", "true")
            .append_pair("MediaSourceId", &media_source)
            .append_pair("PlaySessionId", &play_session);
        (url, "DirectPlay")
    } else {
        let raw = source["TranscodingUrl"]
            .as_str()
            .ok_or("Server did not provide a stream for this track/live selection")?;
        (pinned_url(&session.base, &prefix, raw)?, "Transcode")
    };
    let hls = url.path().ends_with(".m3u8");
    let offset = if hls {
        url.query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case("StartTimeTicks"))
            .map(|(_, v)| v.parse::<u64>().map(|n| n as f64 / 10_000_000.0))
            .transpose()
            .map_err(|_| "Invalid HLS offset")?
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let tracks: Vec<Track> =
        serde_json::from_value(source.get("MediaStreams").cloned().unwrap_or(json!([])))
            .map_err(|_| "Invalid stream list")?;
    if tracks.len() > 256 {
        return Err("Too many media streams".into());
    }
    for (kind, selected) in [("Audio", selection.audio), ("Subtitle", selection.subtitle)] {
        if let Some(index) = selected
            && index >= 0
            && !tracks.iter().any(|s| s.kind == kind && s.index == index)
        {
            return Err("Selected stream was absent from negotiated source".into());
        }
    }
    Ok(Plan {
        item,
        url,
        prefix,
        play_session,
        media_source,
        live_stream,
        method: method.into(),
        hls,
        live,
        offset,
        selection,
        tracks,
    })
}
pub async fn report(
    session: &Session,
    plan: &Plan,
    phase: &str,
    position: f64,
    paused: bool,
) -> Result<()> {
    let path = match phase {
        "started" => "Sessions/Playing",
        "progress" => "Sessions/Playing/Progress",
        "stopped" => "Sessions/Playing/Stopped",
        _ => return Err("Invalid playback event".into()),
    };
    protocol::request(&session.base,&session.token,"POST",path,&[],Some(json!({"ItemId":plan.item.id,"MediaSourceId":plan.media_source,"PlaySessionId":plan.play_session,"PositionTicks":ticks(position),"IsPaused":paused,"CanSeek":!plan.live,"PlayMethod":plan.method,"AudioStreamIndex":plan.selection.audio,"SubtitleStreamIndex":plan.selection.subtitle}))).await.map(|_|())
}
async fn close_live(session: &Session, live: &str) -> Result<()> {
    protocol::request(
        &session.base,
        &session.token,
        "POST",
        "LiveStreams/Close",
        &[("liveStreamId", live.into())],
        None,
    )
    .await
    .map(|_| ())
}
pub async fn release(session: &Session, plan: &Plan) -> Result<()> {
    let mut errors = Vec::new();
    if let Some(live) = &plan.live_stream
        && let Err(e) = close_live(session, live).await
    {
        errors.push(e);
    }
    if plan.method == "Transcode"
        && let Err(e) = protocol::request(
            &session.base,
            &session.token,
            "DELETE",
            "Videos/ActiveEncodings",
            &[
                ("deviceId", protocol::device_id()),
                ("playSessionId", plan.play_session.clone()),
            ],
            None,
        )
        .await
    {
        errors.push(e);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err("Playback resource release was not acknowledged".into())
    }
}
async fn release_info(session: &Session, info: &Value) -> Result<()> {
    let mut failed = false;
    for source in info["MediaSources"]
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
    {
        if let Some(live) = source["LiveStreamId"].as_str() {
            failed |= close_live(session, live).await.is_err();
        }
    }
    if let Some(play) = info["PlaySessionId"].as_str() {
        failed |= protocol::request(
            &session.base,
            &session.token,
            "DELETE",
            "Videos/ActiveEncodings",
            &[
                ("deviceId", protocol::device_id()),
                ("playSessionId", play.into()),
            ],
            None,
        )
        .await
        .is_err();
    }
    if failed {
        Err("Prepared playback release not acknowledged".into())
    } else {
        Ok(())
    }
}
/// A play run owns one order of queue indices, including duplicates. Repeat
/// Off exhausts it; Repeat All creates a fresh cycle. No account state is saved.
#[derive(Clone)]
pub struct QueueRun {
    pub state: QueueState,
    order: Vec<usize>,
    position: usize,
}
fn shuffle(order: &mut [usize], mut seed: u64) {
    if seed == 0 {
        seed = 0x9e3779b97f4a7c15;
    }
    for i in (1..order.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        order.swap(i, seed as usize % (i + 1));
    }
}
impl QueueRun {
    pub fn new(state: QueueState, current: usize, seed: u64) -> Option<Self> {
        if !state.is_valid() || current >= state.item_ids.len() {
            return None;
        }
        let mut order: Vec<_> = (0..state.item_ids.len()).collect();
        let position = if state.shuffle {
            order.retain(|n| *n != current);
            shuffle(&mut order, seed);
            order.insert(0, current);
            0
        } else {
            current
        };
        Some(Self {
            state,
            order,
            position,
        })
    }
    pub fn current(&self) -> usize {
        self.order[self.position]
    }
    pub fn advance(&mut self, seed: u64) -> Option<usize> {
        if self.state.repeat == RepeatMode::One {
            return Some(self.current());
        }
        if self.position + 1 < self.order.len() {
            self.position += 1;
            return Some(self.current());
        }
        if self.state.repeat != RepeatMode::All {
            return None;
        }
        let previous = self.current();
        self.order = (0..self.state.item_ids.len()).collect();
        if self.state.shuffle {
            shuffle(&mut self.order, seed);
            if self.order.len() > 1 && self.order[0] == previous {
                self.order.swap(0, 1);
            }
        }
        self.position = 0;
        Some(self.current())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_pins_and_header_credentials() {
        let base = Url::parse("https://example.test/root/").unwrap();
        let prefix = "/root/Videos/abc/";
        let url = pinned_url(
            &base,
            prefix,
            "Videos/abc/master.m3u8?api_key=discard&AudioStreamIndex=1",
        )
        .unwrap();
        assert_eq!(url.query(), Some("AudioStreamIndex=1"));
        for raw in [
            "https://evil.test/root/Videos/abc/a.ts",
            "../Videos/abc/a.ts",
            "Videos/other/a.ts",
            "Videos/abc/%2fsecret",
        ] {
            assert!(pinned_url(&base, prefix, raw).is_err());
        }
        assert!(protocol::authorization("x\r\nInjected:yes").is_err());
        assert!(
            protocol::authorization("quote\"slash\\")
                .unwrap()
                .contains("Token=\"quote\\\"slash\\\\\"")
        );
    }
    #[test]
    fn queue_duplicates_eof_repeat_and_shuffle() {
        let mut q = QueueState {
            item_ids: vec![uuid::Uuid::from_u128(1); 3],
            ..Default::default()
        };
        let mut run = QueueRun::new(q.clone(), 0, 1).unwrap();
        assert_eq!(run.advance(2), Some(1));
        assert_eq!(run.advance(3), Some(2));
        assert_eq!(run.advance(4), None);
        q.repeat = RepeatMode::All;
        let mut run = QueueRun::new(q.clone(), 2, 1).unwrap();
        assert_eq!(run.advance(2), Some(0));
        q.repeat = RepeatMode::One;
        let mut run = QueueRun::new(q, 1, 1).unwrap();
        assert_eq!(run.advance(2), Some(1));
    }
    #[test]
    fn shuffle_repeat_off_is_finite() {
        let mut q = QueueState {
            item_ids: vec![uuid::Uuid::from_u128(1); 12],
            shuffle: true,
            ..Default::default()
        };
        for seed in 0..24 {
            let mut run = QueueRun::new(q.clone(), 4, seed).unwrap();
            let mut seen = std::collections::BTreeSet::from([run.current()]);
            while let Some(index) = run.advance(seed + 1) {
                assert!(
                    seen.insert(index),
                    "shuffled index repeated before exhaustion"
                );
            }
            assert_eq!(seen.len(), q.item_ids.len());
        }
        q.repeat = RepeatMode::All;
        let mut run = QueueRun::new(q, 0, 7).unwrap();
        for _ in 0..24 {
            assert!(run.advance(8).is_some());
        }
    }
}
