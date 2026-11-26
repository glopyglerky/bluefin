//! Host identity and conservative platform admission; never viewer authority.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Platform {
    Apple,
    Android,
}
impl Platform {
    pub fn read(v: &Value) -> Result<Self> {
        match v
            .get("clientPlatform")
            .filter(|v| !v.is_null())
            .and_then(Value::as_str)
        {
            None if v.get("clientPlatform").is_none_or(|v| v.is_null()) => Ok(Self::Apple),
            Some("apple") => Ok(Self::Apple),
            Some("android") => Ok(Self::Android),
            _ => Err(err("Unsupported client platform")),
        }
    }
    pub fn identity(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Apple => ("Bluefin Apple", "Apple", "bluefin-apple"),
            Self::Android => ("Bluefin Android", "Android", "bluefin-android"),
        }
    }
}

pub(super) enum VideoCapabilities {
    Apple(native_video::Capabilities),
    Android(AndroidCapabilities),
}
impl VideoCapabilities {
    pub fn read(v: &Value) -> Result<Self> {
        match Platform::read(v)? {
            Platform::Apple => native_video::Capabilities::read(v).map(Self::Apple),
            Platform::Android => AndroidCapabilities::read(v).map(Self::Android),
        }
    }
    pub fn accepts(&self, stream: &Value) -> bool {
        match self {
            Self::Apple(c) => c.accepts(stream),
            Self::Android(c) => c.accepts(stream),
        }
    }
    pub fn requires_probe(&self, stream: &Value) -> bool {
        match self {
            Self::Apple(c) => c.requires_probe(stream),
            Self::Android(_) => false,
        }
    }
    pub fn accepts_audio(&self, stream: &Value) -> bool {
        match self {
            Self::Apple(_) => matches!(stream["Codec"].as_str(), Some("aac" | "ac3" | "eac3")),
            Self::Android(c) => {
                stream["Codec"] == "aac"
                    && c.has("audio/mp4a-latm")
                    && stream["Channels"]
                        .as_u64()
                        .is_some_and(|n| (1..=2).contains(&n))
            }
        }
    }
    pub fn profile(&self) -> Value {
        match self {
            Self::Apple(c) => c.profile(),
            Self::Android(c) => c.profile(),
        }
    }
}

pub(super) struct AndroidCapabilities {
    mimes: Vec<String>,
}
impl AndroidCapabilities {
    fn read(v: &Value) -> Result<Self> {
        let Some(caps) = v.get("androidCapabilities").filter(|c| !c.is_null()) else {
            return Ok(Self { mimes: Vec::new() });
        };
        let mimes = caps["decoderMimeTypes"]
            .as_array()
            .filter(|v| v.len() <= 128)
            .ok_or_else(|| err("Invalid Android decoder observations"))?;
        if caps["schemaVersion"] != 1
            || [
                "nativeProbeSupported",
                "hdrQualified",
                "passthroughQualified",
            ]
            .iter()
            .any(|k| caps[*k] != false)
        {
            return Err(err("Unsupported Android capability qualification"));
        }
        let mimes = mimes
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| {
                        s.len() <= 128
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"/.-_".contains(&b))
                    })
                    .map(str::to_owned)
                    .ok_or_else(|| err("Invalid Android decoder MIME type"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { mimes })
    }
    fn has(&self, mime: &str) -> bool {
        self.mimes.iter().any(|v| v == mime)
    }
    fn accepts(&self, s: &Value) -> bool {
        self.has("video/avc")
            && s["Type"] == "Video"
            && s["Codec"] == "h264"
            && matches!(
                s["Profile"]
                    .as_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("baseline" | "constrained baseline" | "main" | "high")
            )
            && s["Width"].as_u64().is_some_and(|v| (1..=1920).contains(&v))
            && s["Height"]
                .as_u64()
                .is_some_and(|v| (1..=1080).contains(&v))
            && s["Level"].as_f64().is_some_and(|v| v > 0.0 && v <= 41.0)
            && s["BitDepth"].as_u64().is_some_and(|v| (1..=8).contains(&v))
            && s["VideoRangeType"]
                .as_str()
                .or_else(|| s["VideoRange"].as_str())
                == Some("SDR")
    }
    pub fn audio(&self, source: &Value) -> bool {
        if source["SupportsDirectPlay"] != true {
            return false;
        }
        let container = source["Container"].as_str().unwrap_or("");
        let Some(streams) = source["MediaStreams"].as_array() else {
            return false;
        };
        streams.iter().any(|s| s["Type"] == "Audio")
            && !streams.iter().any(|s| s["Type"] == "Video")
            && streams
                .iter()
                .filter(|s| s["Type"] == "Audio")
                .all(|s| self.audio_container(s["Codec"].as_str().unwrap_or(""), container))
    }
    fn audio_container(&self, codec: &str, container: &str) -> bool {
        self.audio_codec(codec)
            && match codec {
                "aac" => matches!(container, "aac" | "m4a" | "mp4"),
                "mp3" => container == "mp3",
                "flac" => container == "flac",
                "pcm_s16le" => container == "wav",
                _ => false,
            }
    }
    pub fn audio_codec(&self, codec: &str) -> bool {
        match codec {
            "aac" => self.has("audio/mp4a-latm"),
            "mp3" => self.has("audio/mpeg"),
            "flac" => self.has("audio/flac"),
            "pcm_s16le" => true,
            _ => false,
        }
    }
    fn profile(&self) -> Value {
        let mut value = super::profile();
        value["Name"] = json!("Bluefin Android Media3 conservative SDR");
        let mut direct = Vec::new();
        if self.has("video/avc") && self.has("audio/mp4a-latm") {
            direct.push(json!({"Container":"mp4,m4v","Type":"Video","VideoCodec":"h264","AudioCodec":"aac"}));
        }
        for (codec, containers) in [
            ("aac", "aac,m4a,mp4"),
            ("mp3", "mp3"),
            ("flac", "flac"),
            ("pcm_s16le", "wav"),
        ] {
            if self.audio_codec(codec) {
                direct.push(json!({"Container":containers,"Type":"Audio","AudioCodec":codec}));
            }
        }
        value["DirectPlayProfiles"] = json!(direct);
        value["CodecProfiles"].as_array_mut().unwrap().push(json!({"Type":"Audio","Codec":"aac","Conditions":[
            {"Condition":"LessThanEqual","Property":"AudioChannels","Value":"2","IsRequired":true}
        ]}));
        value
    }
}

pub(super) fn audio(v: &Value, source: &Value) -> Result<bool> {
    Ok(match Platform::read(v)? {
        Platform::Apple => super::native_audio(source),
        Platform::Android => AndroidCapabilities::read(v)?.audio(source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn android() -> Value {
        json!({"clientPlatform":"android","androidCapabilities":{"schemaVersion":1,"decoderMimeTypes":["video/avc","video/hevc","audio/mp4a-latm","audio/flac"],"nativeProbeSupported":false,"hdrQualified":false,"passthroughQualified":false}})
    }
    #[test]
    fn identity_and_unknown_platform_are_explicit() {
        assert_eq!(
            Platform::read(&android()).unwrap().identity().0,
            "Bluefin Android"
        );
        assert_eq!(
            Platform::read(&json!({})).unwrap().identity().0,
            "Bluefin Apple"
        );
        assert!(Platform::read(&json!({"clientPlatform":"official-client"})).is_err());
        assert!(Platform::read(&json!({"clientPlatform":1})).is_err());
        let mut request = android();
        request["server"] = json!("https://example.com");
        request["token"] = json!("fixture-token");
        request["deviceId"] = json!("fixture-device");
        let header = Api::new(&request).unwrap().auth(true).unwrap();
        assert!(header.contains("Client=\"Bluefin Android\""));
        assert!(!header.contains("Apple"));
    }
    #[test]
    fn android_does_not_inherit_apple_hdr_or_decoder_claims() {
        let mut v = android();
        v["videoCapabilities"] =
            json!({"schemaVersion":1,"nativeProbeSupported":true,"hevcHardwareDecode":true});
        let caps = VideoCapabilities::read(&v).unwrap();
        let s = json!({"Type":"Video","Codec":"h264","Profile":"High","Width":1920,"Height":1080,"Level":41,"BitDepth":8,"VideoRangeType":"SDR"});
        assert!(caps.accepts(&s));
        assert!(!caps.requires_probe(&s));
        for (key, value) in [
            ("Codec", json!("hevc")),
            ("VideoRangeType", json!("DOVI")),
            ("Width", json!(3840)),
            ("Level", json!(52)),
        ] {
            let mut bad = s.clone();
            bad[key] = value;
            assert!(!caps.accepts(&bad));
        }
        let profile = caps.profile().to_string();
        assert!(!profile.contains("Apple"));
        assert!(!profile.contains("alac"));
        assert!(!profile.contains("eac3"));
    }
    #[test]
    fn android_audio_keeps_codec_and_container_coupled() {
        let caps = AndroidCapabilities::read(&android()).unwrap();
        let mut source = json!({"SupportsDirectPlay":true,"Container":"flac","MediaStreams":[{"Type":"Audio","Codec":"flac"}]});
        assert!(caps.audio(&source));
        for container in ["wav", "mp3", "mp4"] {
            source["Container"] = json!(container);
            assert!(!caps.audio(&source));
        }
        let profile = caps.profile();
        for entry in profile["DirectPlayProfiles"].as_array().unwrap() {
            if entry["Type"] == "Audio" && entry["AudioCodec"] == "flac" {
                assert_eq!(entry["Container"], "flac");
            }
        }
    }
}
