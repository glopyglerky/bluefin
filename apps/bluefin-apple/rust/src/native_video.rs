//! Native format admission. Capability observations do not establish viewer rights.
//! Expanded formats require the host to probe the authorized asset with its decoder.
use super::*;

#[derive(Default)]
pub(super) struct Capabilities {
    hevc: bool,
    av1: bool,
    probe: bool,
    width: u64,
    height: u64,
}
impl Capabilities {
    pub(super) fn read(v: &Value) -> Result<Self> {
        let Some(value) = v.get("videoCapabilities").filter(|v| !v.is_null()) else {
            return Ok(Self {
                width: 1920,
                height: 1080,
                ..Self::default()
            });
        };
        if !value.is_object() || value["schemaVersion"] != 1 {
            return Err(err("Unsupported video capabilities"));
        }
        let flag = |key: &str| -> Result<bool> {
            match value.get(key) {
                None | Some(Value::Null) => Ok(false),
                Some(Value::Bool(v)) => Ok(*v),
                _ => Err(err("Invalid video capabilities")),
            }
        };
        let dimension = |key: &str, fallback, maximum| -> Result<u64> {
            match value.get(key).filter(|v| !v.is_null()) {
                None => Ok(fallback),
                Some(v) => v
                    .as_u64()
                    .filter(|n| (256..=maximum).contains(n))
                    .ok_or_else(|| err("Invalid video dimension")),
            }
        };
        Ok(Self {
            hevc: flag("hevcHardwareDecode")?,
            av1: flag("av1HardwareDecode")?,
            probe: flag("nativeProbeSupported")?,
            width: dimension("maximumWidth", 1920, 7680)?,
            height: dimension("maximumHeight", 1080, 4320)?,
        })
    }
    pub(super) fn accepts(&self, s: &Value) -> bool {
        if s["Type"] != "Video" {
            return false;
        }
        let codec = s["Codec"].as_str().unwrap_or("").to_ascii_lowercase();
        let profile = s["Profile"].as_str().unwrap_or("").to_ascii_lowercase();
        let within = s["Width"]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= self.width)
            && s["Height"]
                .as_u64()
                .is_some_and(|n| n > 0 && n <= self.height);
        if !within {
            return false;
        }
        let range = s["VideoRangeType"]
            .as_str()
            .or_else(|| s["VideoRange"].as_str())
            .unwrap_or("");
        if codec == "h264" {
            return ["high", "main", "baseline", "constrained baseline"]
                .contains(&profile.as_str())
                && s["Level"]
                    .as_f64()
                    .is_some_and(|n| n > 0.0 && n <= if self.probe { 52.0 } else { 41.0 })
                && range == "SDR"
                && s["BitDepth"].as_u64().is_some_and(|n| n <= 8)
                && (!self.requires_probe(s) || self.probe);
        }
        if !self.probe || !((codec == "hevc" && self.hevc) || (codec == "av1" && self.av1)) {
            return false;
        }
        if !["main", "main 10", "main10"].contains(&profile.as_str())
            || !s["BitDepth"]
                .as_u64()
                .is_some_and(|n| (8..=10).contains(&n))
        {
            return false;
        }
        match range {
            "SDR" | "HDR10" | "HLG" | "HDR10Plus" => true,
            "DOVI" | "DOVIWithHDR10" | "DOVIWithHLG" | "DOVIWithSDR" | "DOVIWithHDR10Plus" => {
                let enhancement = s["ElPresentFlag"] == true || s["ElPresentFlag"] == 1;
                if enhancement {
                    return false;
                }
                match s["DvProfile"].as_u64() {
                    Some(5) => codec == "hevc",
                    Some(8) => {
                        codec == "hevc"
                            && matches!(s["DvBlSignalCompatibilityId"].as_u64(), Some(1 | 2 | 4))
                    }
                    Some(10) => codec == "av1",
                    _ => false,
                }
            }
            _ => false,
        }
    }
    pub(super) fn requires_probe(&self, s: &Value) -> bool {
        s["Codec"] != "h264"
            || s["Width"].as_u64().is_some_and(|n| n > 1920)
            || s["Height"].as_u64().is_some_and(|n| n > 1080)
            || s["Level"].as_f64().is_some_and(|n| n > 41.0)
    }
    pub(super) fn profile(&self) -> Value {
        let mut value = super::profile();
        if !self.probe {
            return value;
        }
        value["Name"] = json!("Bluefin native asset-probed playback");
        let mut codecs = vec!["h264"];
        if self.hevc {
            codecs.push("hevc");
        }
        if self.av1 {
            codecs.push("av1");
        }
        value["DirectPlayProfiles"][0]["VideoCodec"] = json!(codecs.join(","));
        value["CodecProfiles"][0]["Conditions"][1]["Value"] = json!("52");
        value["CodecProfiles"][0]["Conditions"][4]["Value"] = json!(self.width.to_string());
        value["CodecProfiles"][0]["Conditions"][5]["Value"] = json!(self.height.to_string());
        for codec in codecs.into_iter().skip(1) {
            value["CodecProfiles"].as_array_mut().unwrap().push(json!({"Type":"Video","Codec":codec,"Conditions":[
                {"Condition":"EqualsAny","Property":"VideoProfile","Value":"main|main 10","IsRequired":true},
                {"Condition":"LessThanEqual","Property":"Width","Value":self.width.to_string(),"IsRequired":true},
                {"Condition":"LessThanEqual","Property":"Height","Value":self.height.to_string(),"IsRequired":true},
                {"Condition":"LessThanEqual","Property":"VideoBitDepth","Value":"10","IsRequired":true},
                {"Condition":"EqualsAny","Property":"VideoRangeType","Value":"SDR|HDR10|HLG|HDR10Plus|DOVI|DOVIWithHDR10|DOVIWithHLG|DOVIWithSDR|DOVIWithHDR10Plus","IsRequired":true}
            ]}));
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capable() -> Capabilities {
        Capabilities::read(&json!({"videoCapabilities":{"schemaVersion":1,"hevcHardwareDecode":true,"av1HardwareDecode":true,"nativeProbeSupported":true,"maximumWidth":3840,"maximumHeight":2160}})).unwrap()
    }
    fn source(codec: &str, range: &str) -> Value {
        json!({"Type":"Video","Codec":codec,"Profile":"Main 10","Width":3840,"Height":2160,"BitDepth":10,"VideoRangeType":range})
    }
    #[test]
    fn expanded_formats_need_host_probe_and_hardware_observation() {
        let stream = source("hevc", "HDR10");
        assert!(!Capabilities::read(&json!({})).unwrap().accepts(&stream));
        let no_probe = Capabilities::read(
            &json!({"videoCapabilities":{"schemaVersion":1,"hevcHardwareDecode":true}}),
        )
        .unwrap();
        assert!(!no_probe.accepts(&stream));
        assert!(capable().accepts(&stream));
        assert!(capable().requires_probe(&stream));
        for range in ["SDR", "HDR10", "HLG", "HDR10Plus"] {
            assert!(capable().accepts(&source("av1", range)));
        }
    }
    #[test]
    fn unsupported_dolby_layers_and_unknown_range_are_not_admitted() {
        let caps = capable();
        let mut stream = source("hevc", "DOVIWithHDR10");
        stream["DvProfile"] = json!(7);
        assert!(!caps.accepts(&stream));
        stream["DvProfile"] = json!(8);
        stream["DvBlSignalCompatibilityId"] = json!(1);
        assert!(caps.accepts(&stream));
        stream["ElPresentFlag"] = json!(1);
        assert!(!caps.accepts(&stream));
        stream["ElPresentFlag"] = json!(0);
        stream["VideoRangeType"] = json!("DOVIInvalid");
        assert!(!caps.accepts(&stream));
        assert!(!caps.accepts(&source("hevc", "HDR")));
    }
    #[test]
    fn capability_bounds_and_unknown_versions_fail_before_network() {
        for capabilities in [
            json!({"schemaVersion":2}),
            json!({"schemaVersion":1,"maximumWidth":99999}),
            json!({"schemaVersion":1,"nativeProbeSupported":"yes"}),
        ] {
            assert!(Capabilities::read(&json!({"videoCapabilities":capabilities})).is_err());
        }
        assert!(!capable().accepts(&json!({"Type":"Video","Codec":"hevc","Profile":"Main 10","BitDepth":10,"VideoRangeType":"HDR10"})));
    }
    #[test]
    fn profile_has_no_unsupported_lossless_atmos_or_software_decoder_claim() {
        let profile = capable().profile();
        assert_eq!(
            profile["DirectPlayProfiles"][0]["VideoCodec"],
            "h264,hevc,av1"
        );
        assert_eq!(profile["TranscodingProfiles"][0]["VideoCodec"], "h264");
        assert!(!profile.to_string().contains("truehd"));
        assert!(!profile.to_string().contains("dts"));
    }
}
