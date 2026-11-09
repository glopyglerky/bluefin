//! Bluefin native client core. Upstream attribution and GPL license remain at repository root.
use reqwest::{blocking::Client, Method};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, CString},
    io::Read,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use url::Url;
mod home_assistant;
mod iroh_transport;
mod native_server;
mod offline;
mod premium;
#[cfg(feature = "iroh-probe")]
mod probe;
mod sports;
mod stream;
const INPUT_MAX: usize = 65536;
const RESPONSE_MAX: usize = 8 * 1024 * 1024;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
type Result<T> = std::result::Result<T, String>;
fn err(s: &str) -> String {
    s.to_owned()
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    let s = v
        .get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing or invalid {k}"))?;
    if s.is_empty() || s.len() > 4096 || s.chars().any(char::is_control) {
        return Err(format!("Invalid {k}"));
    }
    Ok(s)
}
fn local(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".local")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
        })
}
fn server(v: &Value) -> Result<Url> {
    let s = text(v, "server")?;
    if s.contains('\\') {
        return Err(err("Invalid server URL"));
    }
    let mut u = Url::parse(s).map_err(|_| err("Invalid server URL"))?;
    let path = u.path().to_ascii_lowercase();
    if u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || path.contains("%2e")
        || path.contains("%2f")
        || path.contains("%5c")
    {
        return Err(err(
            "Server URL must have no credentials, query, fragment or escaped path separators",
        ));
    }
    if u.scheme() != "https"
        && !(u.scheme() == "http"
            && v["allowLocalHttp"] == true
            && local(u.host_str().unwrap_or("").trim_matches(['[', ']'])))
    {
        return Err(err("HTTPS required; local HTTP requires allowLocalHttp"));
    }
    if !u.path().ends_with('/') {
        u.set_path(&format!("{}/", u.path()));
    }
    Ok(u)
}
struct Api<'a> {
    v: &'a Value,
    base: Url,
    client: Client,
}
impl<'a> Api<'a> {
    fn new(v: &'a Value) -> Result<Self> {
        Ok(Self {
            v,
            base: server(v)?,
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|_| err("HTTP client unavailable"))?,
        })
    }
    fn url(&self, parts: &[&str]) -> Result<Url> {
        let mut u = self.base.clone();
        {
            let mut p = u
                .path_segments_mut()
                .map_err(|_| err("Invalid server path"))?;
            p.pop_if_empty();
            for part in parts {
                if part.is_empty()
                    || *part == "."
                    || *part == ".."
                    || part.contains(['/', '\\', '%', '?', '#'])
                {
                    return Err(err("Invalid path identifier"));
                }
                p.push(part);
            }
        }
        Ok(u)
    }
    fn auth(&self, required: bool) -> Result<String> {
        let device = self
            .v
            .get("deviceId")
            .and_then(Value::as_str)
            .unwrap_or("bluefin-apple");
        if device.len() > 128
            || !device
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(err("Invalid deviceId"));
        }
        let mut a = format!("MediaBrowser Client=\"Bluefin Apple\", Device=\"Apple\", DeviceId=\"{device}\", Version=\"0.1.0\"");
        if required {
            let token = text(self.v, "token")?;
            if !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(err("Invalid token"));
            }
            a.push_str(&format!(", Token=\"{token}\""));
        }
        Ok(a)
    }
    fn request(
        &self,
        method: Method,
        parts: &[&str],
        query: Vec<(&str, String)>,
        body: Option<Value>,
        auth: bool,
    ) -> Result<Value> {
        let mut r = self
            .client
            .request(method, self.url(parts)?)
            .header("Authorization", self.auth(auth)?)
            .query(&query);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let response = r
            .send()
            .map_err(|_| err("Server request failed or timed out"))?;
        response_json(response)
    }
    fn get(&self, p: &[&str], q: Vec<(&str, String)>) -> Result<Value> {
        self.request(Method::GET, p, q, None, true)
    }
    fn post(&self, p: &[&str], b: Value) -> Result<Value> {
        self.request(Method::POST, p, vec![], Some(b), true)
    }
    fn cleanup_request(
        &self,
        method: Method,
        parts: &[&str],
        query: Vec<(&str, String)>,
    ) -> Result<()> {
        let response = self
            .client
            .request(method, self.url(parts)?)
            .header("Authorization", self.auth(true)?)
            .query(&query)
            .timeout(Duration::from_secs(3))
            .send()
            .map_err(|_| err("Playback cleanup request failed or timed out"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "Playback cleanup returned HTTP {}",
                response.status().as_u16()
            ))
        }
    }
}
fn response_json(response: reqwest::blocking::Response) -> Result<Value> {
    if !response.status().is_success() {
        return Err(format!(
            "Server returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > RESPONSE_MAX as u64)
    {
        return Err(err("Server response too large"));
    }
    let mut bytes = Vec::new();
    response
        .take(RESPONSE_MAX as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| err("Server response failed or timed out"))?;
    if bytes.len() > RESPONSE_MAX {
        return Err(err("Server response too large"));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| err("Invalid server JSON"))
}
fn count(v: &Value, k: &str, default: u64, max: u64) -> Result<u64> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(x) => x
            .as_u64()
            .filter(|n| *n <= max)
            .ok_or_else(|| format!("Invalid {k}")),
    }
}
fn index(v: &Value, k: &str) -> Result<Option<i64>> {
    v.get(k)
        .filter(|v| !v.is_null())
        .map(|x| {
            x.as_i64()
                .filter(|n| (-1..=128).contains(n))
                .ok_or_else(|| format!("Invalid {k}"))
        })
        .transpose()
}
fn profile() -> Value {
    json!({"Name":"Bluefin Apple conservative SDR", "MaxStreamingBitrate":20000000,
    "DirectPlayProfiles":[{"Container":"mp4,m4v","Type":"Video","VideoCodec":"h264","AudioCodec":"aac,ac3,eac3"},{"Container":"mp3,aac,m4a,mp4,flac,wav,aiff","Type":"Audio","AudioCodec":"mp3,aac,flac,alac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_s16be,pcm_s24be,pcm_s32be"}],
    "TranscodingProfiles":[{"Container":"ts","Type":"Video","Protocol":"hls","VideoCodec":"h264","AudioCodec":"aac","MaxAudioChannels":"2","MinSegments":2,"BreakOnNonKeyFrames":false},{"Container":"ts","Type":"Audio","Protocol":"hls","AudioCodec":"aac","MaxAudioChannels":"2"}],
    "CodecProfiles":[{"Type":"Video","Codec":"h264","Conditions":[{"Condition":"EqualsAny","Property":"VideoProfile","Value":"high|main|baseline|constrained baseline","IsRequired":true},{"Condition":"LessThanEqual","Property":"VideoLevel","Value":"41","IsRequired":true},{"Condition":"EqualsAny","Property":"VideoRangeType","Value":"SDR","IsRequired":true},{"Condition":"LessThanEqual","Property":"VideoBitDepth","Value":"8","IsRequired":true},{"Condition":"LessThanEqual","Property":"Width","Value":"1920","IsRequired":true},{"Condition":"LessThanEqual","Property":"Height","Value":"1080","IsRequired":true}]}]})
}
fn playback_bitrate(v: &Value) -> Result<u64> {
    let quality = match v.get("quality").filter(|value| !value.is_null()) {
        None => "automatic",
        Some(value) => value
            .as_str()
            .ok_or_else(|| err("Invalid playback quality"))?,
    };
    match quality {
        "automatic" => Ok(80_000_000),
        "original" => Ok(200_000_000),
        "balanced" => Ok(12_000_000),
        "dataSaver" => Ok(3_000_000),
        _ => Err(err("Invalid playback quality")),
    }
}
fn selected_source(v: &Value) -> Result<Option<&str>> {
    v.get("mediaSourceId")
        .filter(|value| !value.is_null())
        .map(|_| text(v, "mediaSourceId"))
        .transpose()
}
fn native_audio(source: &Value) -> bool {
    source["SupportsDirectPlay"] == true
        && matches!(
            source["Container"].as_str(),
            Some("mp3" | "aac" | "m4a" | "mp4" | "flac" | "wav" | "aiff")
        )
        && source["MediaStreams"].as_array().is_some_and(|streams| {
            streams.iter().any(|s| s["Type"] == "Audio")
                && !streams.iter().any(|s| s["Type"] == "Video")
                && streams.iter().filter(|s| s["Type"] == "Audio").all(|s| {
                    matches!(
                        s["Codec"].as_str(),
                        Some(
                            "mp3"
                                | "aac"
                                | "flac"
                                | "alac"
                                | "pcm_s16le"
                                | "pcm_s24le"
                                | "pcm_s32le"
                                | "pcm_s16be"
                                | "pcm_s24be"
                                | "pcm_s32be"
                        )
                    )
                })
        })
}
fn playback_url(base: &Url, raw: &str) -> Result<Url> {
    if raw.len() > 16384 || raw.contains('\\') || raw.chars().any(char::is_control) {
        return Err(err("Invalid playback URL"));
    }
    let u = base.join(raw).map_err(|_| err("Invalid playback URL"))?;
    if u.origin() != base.origin()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err(err(
            "Playback URL must use server origin without credentials",
        ));
    }
    Ok(u)
}
fn playback(api: &Api<'_>) -> Result<Value> {
    let v = api.v;
    let item = text(v, "itemId")?;
    let user = text(v, "userId")?;
    let ticks = count(v, "startTicks", 0, i64::MAX as u64)?;
    let audio = index(v, "audioStreamIndex")?;
    let subtitle = index(v, "subtitleStreamIndex")?;
    let bitrate = playback_bitrate(v)?;
    let mut device_profile = profile();
    device_profile["MaxStreamingBitrate"] = json!(bitrate);
    let b = json!({"UserId":user,"MediaSourceId":selected_source(v)?,"MaxStreamingBitrate":bitrate,"StartTimeTicks":ticks,"AudioStreamIndex":audio,"SubtitleStreamIndex":subtitle,"IsPlayback":true,"AutoOpenLiveStream":true,"EnableDirectPlay":audio.is_none_or(|index| index < 0) && subtitle.is_none_or(|index| index < 0),"EnableDirectStream":false,"EnableTranscoding":true,"DeviceProfile":device_profile});
    let info = api.post(&["Items", item, "PlaybackInfo"], b)?;
    let result = plan_playback(api, &info);
    if let Err(error) = &result {
        if release_prepared(api, &info).is_err() {
            return Err(format!("{error}; playback cleanup could not be confirmed"));
        }
    }
    result
}
fn release_prepared(api: &Api<'_>, info: &Value) -> Result<()> {
    let mut failure = None;
    let mut closed = std::collections::HashSet::new();
    for source in info["MediaSources"].as_array().into_iter().flatten() {
        if let Some(id) = source["LiveStreamId"].as_str().filter(|id| !id.is_empty()) {
            if closed.insert(id) {
                if closed.len() > 16 {
                    failure.get_or_insert(err("Too many opened live sources; cleanup incomplete"));
                    break;
                }
                if let Err(error) = api.cleanup_request(
                    Method::POST,
                    &["LiveStreams", "Close"],
                    vec![("liveStreamId", id.into())],
                ) {
                    failure.get_or_insert(error);
                }
            }
        }
    }
    if info["MediaSources"]
        .as_array()
        .is_some_and(|sources| sources.iter().any(|s| s["TranscodingUrl"].is_string()))
    {
        if let Some(session) = info["PlaySessionId"].as_str() {
            let device = api.v["deviceId"].as_str().unwrap_or("bluefin-apple");
            if let Err(error) = api.cleanup_request(
                Method::DELETE,
                &["Videos", "ActiveEncodings"],
                vec![
                    ("deviceId", device.into()),
                    ("playSessionId", session.into()),
                ],
            ) {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}
fn plan_playback(api: &Api<'_>, info: &Value) -> Result<Value> {
    let v = api.v;
    let item = text(v, "itemId")?;
    let ticks = count(v, "startTicks", 0, i64::MAX as u64)?;
    let audio = index(v, "audioStreamIndex")?;
    let subtitle = index(v, "subtitleStreamIndex")?;
    let is_audio = v["mediaType"] == "Audio";
    let selected_source = selected_source(v)?;
    let bitrate = playback_bitrate(v)?;
    if info.get("ErrorCode").is_some_and(|x| !x.is_null()) {
        return Err(err("Server could not prepare playback"));
    }
    text(info, "PlaySessionId")?;
    let mut sources = info["MediaSources"]
        .as_array()
        .ok_or_else(|| err("No playback sources"))?
        .iter()
        .collect::<Vec<_>>();
    if is_audio {
        sources.sort_by_key(|source| !native_audio(source));
    }
    for source in sources {
        let id = source["Id"]
            .as_str()
            .ok_or_else(|| err("Missing media source ID"))?;
        if selected_source.is_some_and(|selected| selected != id) {
            continue;
        }
        let transcode = source["TranscodingUrl"].as_str();
        let streams = source["MediaStreams"].as_array();
        let direct = source["SupportsDirectPlay"] == true
            && audio.is_none_or(|index| index < 0)
            && source["Bitrate"]
                .as_u64()
                .is_none_or(|value| value <= bitrate)
            && (!matches!(v["quality"].as_str(), Some("balanced" | "dataSaver"))
                || source["Bitrate"].is_u64())
            && matches!(source["Container"].as_str(), Some("mp4" | "m4v"))
            && streams.is_some_and(|s| {
                s.iter().any(|s| {
                    s["Type"] == "Video"
                        && s["Codec"] == "h264"
                        && matches!(
                            s["Profile"].as_str(),
                            Some("High" | "Main" | "Baseline" | "Constrained Baseline")
                        )
                        && s["Level"].as_f64().is_some_and(|n| n <= 41.0)
                        && s["VideoRange"].as_str() == Some("SDR")
                        && s["BitDepth"].as_u64().is_some_and(|n| n <= 8)
                        && s["Width"].as_u64().is_some_and(|n| n <= 1920)
                        && s["Height"].as_u64().is_some_and(|n| n <= 1080)
                }) && s
                    .iter()
                    .filter(|s| s["Type"] == "Audio")
                    .all(|s| matches!(s["Codec"].as_str(), Some("aac" | "ac3" | "eac3")))
            })
            && subtitle.unwrap_or(-1) == -1;
        let audio_direct = is_audio && native_audio(source) && audio.is_none_or(|index| index < 0);
        let (mut url, uses_transcode) = if is_audio {
            if !streams.is_some_and(|ss| {
                ss.iter().any(|s| s["Type"] == "Audio") && !ss.iter().any(|s| s["Type"] == "Video")
            }) {
                continue;
            }
            if audio_direct {
                let extension = format!("stream.{}", source["Container"].as_str().unwrap_or("aac"));
                let mut u = api.url(&["Audio", item, &extension])?;
                u.query_pairs_mut()
                    .append_pair("Static", "true")
                    .append_pair("MediaSourceId", id);
                (u, false)
            } else if let Some(raw) = transcode {
                (playback_url(&api.base, raw)?, true)
            } else {
                continue; // No validated audio plan; do not create a second universal session.
            }
        } else if direct {
            let mut u = api.url(&["Videos", item, "stream.mp4"])?;
            u.query_pairs_mut()
                .append_pair("Static", "true")
                .append_pair("MediaSourceId", id);
            (u, false)
        } else if let Some(raw) = transcode {
            (playback_url(&api.base, raw)?, true)
        } else {
            continue;
        };
        // Credentials travel only in a header. Preserve other server-generated stream parameters.
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| !stream::credential(k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.set_query(None);
        if !pairs.is_empty() {
            url.query_pairs_mut().extend_pairs(pairs);
        }
        let prefix = api
            .url(&[if is_audio { "Audio" } else { "Videos" }, item])?
            .path()
            .to_owned()
            + "/";
        let hls = uses_transcode && url.path().ends_with(".m3u8");
        let offset = if hls {
            url.query_pairs()
                .find(|(k, _)| k.eq_ignore_ascii_case("StartTimeTicks"))
                .map(|(_, v)| v.parse::<u64>().map_err(|_| err("Invalid HLS offset")))
                .transpose()?
                .unwrap_or(0)
        } else {
            0
        };
        let (proxy_id, local_url) = stream::start_with_transport(
            url.clone(),
            prefix,
            api.auth(true)?,
            iroh_transport::Tunnel::from_request(api)?,
        )?;
        let result = json!({"playbackInfo":info,"source":source,"url":local_url,"remoteUrl":url.as_str(),"headers":{},"proxyId":proxy_id,"playSessionId":info["PlaySessionId"],"mediaSourceId":id,"startTicks":ticks,"startOffsetTicks":offset,"playMethod":if uses_transcode {"Transcode"} else {"DirectPlay"},"audioStreamIndex":audio,"subtitleStreamIndex":subtitle});
        if result.to_string().len() > RESPONSE_MAX - 64 {
            stream::stop(&proxy_id)?;
            return Err(err("Playback metadata too large"));
        }
        return Ok(result);
    }
    Err(err("No supported Apple playback source"))
}
/// JSON interface, also useful for non-FFI hosts. Synchronous and bounded.
pub fn call(v: &Value) -> Result<Value> {
    let op = text(v, "operation")?;
    if matches!(op, "homeAssistantStates" | "homeAssistantControl") {
        return home_assistant::call(v, op);
    }
    if op == "irohProbe" {
        #[cfg(feature = "iroh-probe")]
        {
            return probe::call(v);
        }
        #[cfg(not(feature = "iroh-probe"))]
        {
            return Err(err("Iroh probe feature not built; media route unqualified"));
        }
    }
    if op == "stopPlaybackProxy" {
        stream::stop(text(v, "proxyId")?)?;
        return Ok(json!({"stopped":true}));
    }
    if matches!(
        op,
        "downloadStart" | "downloads" | "downloadCancel" | "downloadRemove"
    ) {
        return offline::call(v, op);
    }
    if op == "logout" {
        offline::logout(v)?;
    }
    let api = Api::new(v)?;
    if matches!(
        op,
        "bluefinCapabilities"
            | "bluefinMusicQueue"
            | "saveBluefinMusicQueue"
            | "bluefinMusicQueueItems"
    ) {
        return native_server::call(&api, op);
    }
    if matches!(
        op,
        "sportsPreferences" | "saveSportsPreferences" | "sportsHome"
    ) {
        return sports::call(&api, op);
    }
    if matches!(
        op,
        "musicItems"
            | "lyrics"
            | "createPlaylist"
            | "addPlaylistItems"
            | "removePlaylistItems"
            | "renamePlaylist"
            | "deletePlaylist"
            | "liveChannels"
            | "livePrograms"
    ) {
        return premium::call(&api, op);
    }
    match op {
        "info" => api.request(
            Method::GET,
            &["System", "Info", "Public"],
            vec![],
            None,
            false,
        ),
        "login" => api.request(
            Method::POST,
            &["Users", "AuthenticateByName"],
            vec![],
            Some(json!({"Username":text(v,"username")?,"Pw":text(v,"password")?})),
            false,
        ),
        "home" => {
            let user = text(v, "userId")?;
            let q = vec![
                ("Limit", "20".into()),
                ("Fields", "PrimaryImageAspectRatio,Overview".into()),
            ];
            let resume = api.get(&["Users", user, "Items", "Resume"], q.clone())?;
            let latest = api.get(&["Users", user, "Items", "Latest"], q.clone())?;
            let next = api.get(
                &["Shows", "NextUp"],
                vec![("UserId", user.into()), ("Limit", "20".into())],
            )?;
            Ok(
                json!({"resume":array_items(resume)?,"latest":array_items(latest)?,"nextUp":array_items(next)?}),
            )
        }
        "libraries" => api.get(&["Users", text(v, "userId")?, "Views"], vec![]),
        "items" => {
            let user = text(v, "userId")?;
            let mut q = vec![
                (
                    "StartIndex",
                    count(v, "startIndex", 0, 1000000)?.to_string(),
                ),
                ("Limit", count(v, "limit", 50, 200)?.to_string()),
                ("Recursive", "true".into()),
                ("Fields", "PrimaryImageAspectRatio,Overview".into()),
            ];
            for (key, out) in [
                ("parentId", "ParentId"),
                ("searchTerm", "SearchTerm"),
                ("includeItemTypes", "IncludeItemTypes"),
            ] {
                if v.get(key).is_some_and(|x| !x.is_null()) {
                    q.push((out, text(v, key)?.into()));
                }
            }
            let r = api.get(&["Users", user, "Items"], q)?;
            let items = r["Items"]
                .as_array()
                .ok_or_else(|| err("Invalid items response"))?;
            let total = r["TotalRecordCount"]
                .as_u64()
                .ok_or_else(|| err("Invalid item count"))?;
            Ok(json!({"Items":items,"TotalRecordCount":total}))
        }
        "item" => api
            .get(
                &["Users", text(v, "userId")?, "Items", text(v, "itemId")?],
                vec![("fields", premium::FIELDS.into())],
            )
            .map(|x| premium::project(&x)),
        "favorite" => {
            let favorite = v["favorite"]
                .as_bool()
                .ok_or_else(|| err("Invalid favorite state"))?;
            api.request(
                if favorite {
                    Method::POST
                } else {
                    Method::DELETE
                },
                &[
                    "Users",
                    text(v, "userId")?,
                    "FavoriteItems",
                    text(v, "itemId")?,
                ],
                vec![],
                None,
                true,
            )
        }
        "releasePlayback" => {
            let mut failure = None;
            if v["playMethod"] == "Transcode" {
                if let Err(error) = api.cleanup_request(
                    Method::DELETE,
                    &["Videos", "ActiveEncodings"],
                    vec![
                        ("deviceId", text(v, "deviceId")?.into()),
                        ("playSessionId", text(v, "playSessionId")?.into()),
                    ],
                ) {
                    failure = Some(error);
                }
            }
            if let Some(live) = v["liveStreamId"].as_str().filter(|s| !s.is_empty()) {
                if let Err(error) = api.cleanup_request(
                    Method::POST,
                    &["LiveStreams", "Close"],
                    vec![("liveStreamId", live.into())],
                ) {
                    failure.get_or_insert(error);
                }
            }
            failure.map_or(Ok(Value::Null), Err)
        }
        "playback" => playback(&api),
        "started" | "progress" | "stopped" => {
            let endpoint = match op {
                "started" => "Playing",
                "progress" => "Progress",
                _ => "Stopped",
            };
            let mut b = json!({"ItemId":text(v,"itemId")?,"MediaSourceId":text(v,"mediaSourceId")?,"PlaySessionId":text(v,"playSessionId")?,"PositionTicks":count(v,"positionTicks",0,i64::MAX as u64)?,"IsPaused":v["isPaused"].as_bool().unwrap_or(false),"IsMuted":v["isMuted"].as_bool().unwrap_or(false),"CanSeek":true,"PlayMethod":text(v,"playMethod")?});
            if !matches!(
                b["PlayMethod"].as_str(),
                Some("DirectPlay" | "DirectStream" | "Transcode")
            ) {
                return Err(err("Invalid playMethod"));
            }
            for (input, out) in [
                ("audioStreamIndex", "AudioStreamIndex"),
                ("subtitleStreamIndex", "SubtitleStreamIndex"),
            ] {
                if let Some(n) = index(v, input)? {
                    b[out] = json!(n);
                }
            }
            if op == "started" {
                api.post(&["Sessions", endpoint], b)
            } else {
                api.post(&["Sessions", "Playing", endpoint], b)
            }
        }
        "logout" => api.post(&["Sessions", "Logout"], Value::Null),
        "quickConnectInitiate" => api.request(
            Method::POST,
            &["QuickConnect", "Initiate"],
            vec![],
            None,
            false,
        ),
        "quickConnectStatus" => api.request(
            Method::GET,
            &["QuickConnect", "Connect"],
            vec![("Secret", text(v, "secret")?.into())],
            None,
            false,
        ),
        "quickConnectAuthenticate" => api.request(
            Method::POST,
            &["Users", "AuthenticateWithQuickConnect"],
            vec![],
            Some(json!({"Secret":text(v,"secret")?})),
            false,
        ),
        _ => Err(err("Unknown operation")),
    }
}
fn array_items(v: Value) -> Result<Value> {
    if v.is_array() {
        Ok(v)
    } else if v["Items"].is_array() {
        Ok(v["Items"].clone())
    } else {
        Err(err("Invalid home response"))
    }
}
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
#[allow(deprecated)] // fetch_update supports Apple stable and NAS toolchains.
fn envelope(input: &[u8]) -> Value {
    let r = (|| {
        let v: Value = serde_json::from_slice(input).map_err(|_| err("Invalid input JSON"))?;
        // Local cancellation is bounded by the two existing proxy slots and never
        // waits for ordinary network-call admission.
        if matches!(
            v["operation"].as_str(),
            Some("stopPlaybackProxy" | "downloadCancel" | "downloadRemove")
        ) {
            return call(&v);
        }
        if ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 4).then_some(n + 1)
            })
            .is_err()
        {
            return Err(err("Client busy"));
        }
        let _lease = Lease;
        call(&v)
    })();
    match r {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error}),
    }
}
/// # Safety
/// `input` must be readable through its NUL terminator, at most INPUT_MAX+1 bytes.
#[no_mangle]
pub unsafe extern "C" fn bluefin_call(input: *const c_char) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if input.is_null() {
            return json!({"ok":false,"error":"Null input"});
        }
        let mut n = 0;
        while n <= INPUT_MAX {
            if unsafe { *input.add(n) } == 0 {
                break;
            }
            n += 1;
        }
        if n > INPUT_MAX {
            return json!({"ok":false,"error":"Input too large"});
        }
        envelope(unsafe { std::slice::from_raw_parts(input.cast::<u8>(), n) })
    })
    .unwrap_or_else(|_| json!({"ok":false,"error":"Client internal failure"}));
    let mut out = result.to_string();
    if out.len() > RESPONSE_MAX {
        out = "{\"ok\":false,\"error\":\"Response too large\"}".into();
    }
    // serde_json escapes NUL, so construction cannot fail.
    CString::new(out).unwrap_or_default().into_raw()
}
/// # Safety
/// `ptr` must be NULL or an unfreed pointer returned by bluefin_call.
#[no_mangle]
pub unsafe extern "C" fn bluefin_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}
#[cfg(test)]
mod tests;
