//! Server-private supplied-stem admission. No caller paths or inference claims.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::{Component, Path},
};
use uuid::Uuid;
pub const MAX_STEM: u64 = 64 * 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid karaoke import")
}
/// Walk every component without following symlinks, including intermediate directories.
#[cfg(unix)]
pub fn open_regular(path: &Path) -> io::Result<File> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut dir = File::open("/")?;
    let parts: Vec<_> = path
        .components()
        .filter(|p| *p != Component::RootDir)
        .collect();
    for (i, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| invalid())?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if i + 1 < parts.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: dir stays live; name is NUL-terminated; returned fd is owned exactly once.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        dir = unsafe { File::from_raw_fd(fd) };
    }
    if !dir.metadata()?.is_file() {
        return Err(invalid());
    }
    Ok(dir)
}
#[cfg(not(unix))]
pub fn open_regular(_: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure import unavailable",
    ))
}
pub fn hash_file(path: &Path) -> io::Result<String> {
    let file = open_regular(path)?;
    if file.metadata()?.len() > 2 * 1024 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut hash = Sha256::new();
    let mut reader = file.take(2 * 1024 * 1024 * 1024 + 1);
    let mut buffer = [0; 65536];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > 2 * 1024 * 1024 * 1024 {
            return Err(invalid());
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Token {
    pub text: String,
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Line {
    pub id: String,
    pub voice: Voice,
    pub tokens: Vec<Token>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Voice {
    Lead,
    Partner,
    Together,
    Backing,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Timeline {
    pub offset_seconds: f64,
    pub lines: Vec<Line>,
}
impl Timeline {
    pub fn validate(&self, duration: f64) -> io::Result<()> {
        if !duration.is_finite()
            || duration <= 0.
            || !self.offset_seconds.is_finite()
            || self.lines.is_empty()
            || self.lines.len() > 10000
        {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        let mut last = -1.;
        for line in &self.lines {
            if line.id.is_empty()
                || line.id.len() > 128
                || !ids.insert(&line.id)
                || line.tokens.is_empty()
                || line.tokens.len() > 1000
            {
                return Err(invalid());
            }
            if line.tokens[0].start < last {
                return Err(invalid());
            }
            last = line.tokens[0].start;
            let mut end = 0.;
            for t in &line.tokens {
                if t.text.is_empty()
                    || t.text.len() > 4096
                    || !t.start.is_finite()
                    || !t.end.is_finite()
                    || t.start < end
                    || t.end <= t.start
                    || t.end + self.offset_seconds > duration + 0.05
                {
                    return Err(invalid());
                }
                end = t.end;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stem {
    pub sha256: String,
    pub bytes: u64,
}
/// Private processing custody. This is written by trusted server preparation,
/// never accepted from a client request. It does not replace source authorization.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Processing {
    #[default]
    SuppliedStems,
    ModelSeparated {
        model_archive_sha256: String,
        engine_version: String,
        input_pcm_sha256: String,
    },
}
impl Processing {
    fn is_supplied(&self) -> bool {
        matches!(self, Self::SuppliedStems)
    }
    fn presentation(
        &self,
        format: (u32, u16, u64),
        duration: f64,
    ) -> io::Result<(&'static str, Option<String>)> {
        match self {
            Self::SuppliedStems => Ok(("suppliedStems", None)),
            Self::ModelSeparated {
                model_archive_sha256,
                engine_version,
                input_pcm_sha256,
            } => {
                if model_archive_sha256
                    != "f3a90b39dd2874269e8b05a48a86745df897b848c61f3958efc80a39152bd692"
                    || engine_version.is_empty()
                    || engine_version.len() > 64
                    || !engine_version
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
                    || input_pcm_sha256.len() != 64
                    || !input_pcm_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || format.0 != 44100
                    || format.1 != 2
                    || duration > 300.
                {
                    return Err(invalid());
                }
                Ok((
                    "modelSeparatedStems",
                    Some("deezer/spleeter/2stems@v1.4.0".into()),
                ))
            }
        }
    }
}
/// Private import manifest. Never serialize this to clients.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Import {
    pub item_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_source_id: Option<String>,
    pub source_sha256: String,
    pub duration_seconds: f64,
    pub rights_reference: String,
    pub accompaniment: Stem,
    pub lead_vocal: Stem,
    pub timeline: Timeline,
    #[serde(default, skip_serializing_if = "Processing::is_supplied")]
    pub processing: Processing,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Package {
    pub item_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_source_id: Option<String>,
    pub revision: String,
    pub duration_seconds: f64,
    pub processing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_reference: Option<String>,
    pub accompaniment: Stem,
    pub lead_vocal: Stem,
    pub timeline: Timeline,
}
pub struct Admitted {
    pub package: Package,
    pub accompaniment: Vec<u8>,
    pub lead_vocal: Vec<u8>,
}
fn bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = open_regular(path)?;
    if file.metadata()?.len() > limit {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    Ok(bytes)
}
/// Deliberately strict canonical WAV: PCM16, fixed 44-byte header, no metadata chunks.
pub fn pcm(bytes: &[u8]) -> io::Result<(u32, u16, u64)> {
    if bytes.len() < 46 {
        return Err(invalid());
    }
    let u16at = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
    let u32at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let rate = u32at(24);
    let channels = u16at(22);
    let size = u32at(40) as usize;
    if &bytes[..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || u32at(16) != 16
        || u16at(20) != 1
        || !(1..=2).contains(&channels)
        || !(8000..=192000).contains(&rate)
        || u16at(34) != 16
        || u16at(32) != channels * 2
        || u32at(28) != rate * u32::from(channels) * 2
        || &bytes[36..40] != b"data"
        || size + 44 != bytes.len()
        || u32at(4) as usize + 8 != bytes.len()
        || !size.is_multiple_of(channels as usize * 2)
    {
        return Err(invalid());
    }
    Ok((rate, channels, (size / (channels as usize * 2)) as u64))
}
pub fn admit(root: &Path, item: Uuid, source: &Path, duration: f64) -> io::Result<Admitted> {
    if item.is_nil() || !duration.is_finite() || duration <= 0. || duration > 14400. {
        return Err(invalid());
    }
    let dir = root.join(item.to_string());
    let raw = bounded(&dir.join("manifest.json"), 1024 * 1024)?;
    let accompaniment = bounded(&dir.join("accompaniment.wav"), MAX_STEM)?;
    let lead_vocal = bounded(&dir.join("lead-vocal.wav"), MAX_STEM)?;
    admit_bytes(
        &raw,
        item,
        Some(&item.simple().to_string()),
        &hash_file(source)?,
        duration,
        accompaniment,
        lead_vocal,
    )
}

/// Validate bytes read from owned opened descriptors. The caller supplies the
/// independently verified original encoded digest, never the prepared PCM hash.
pub fn admit_bytes(
    raw: &[u8],
    item: Uuid,
    media_source_id: Option<&str>,
    source_sha256: &str,
    duration: f64,
    accompaniment: Vec<u8>,
    lead_vocal: Vec<u8>,
) -> io::Result<Admitted> {
    if raw.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let import: Import = serde_json::from_slice(raw).map_err(|_| invalid())?;
    if import.item_id != item
        || item.is_nil()
        || !duration.is_finite()
        || duration <= 0.
        || duration > 14400.
        || import
            .media_source_id
            .as_deref()
            .is_some_and(|id| Some(id) != media_source_id)
        || (!import.processing.is_supplied() && import.media_source_id.is_none())
        || import.rights_reference.trim().is_empty()
        || import.rights_reference.len() > 4096
        || import.source_sha256 != source_sha256
        || source_sha256.len() != 64
        || (import.duration_seconds - duration).abs() > 0.05
    {
        return Err(invalid());
    }
    for (data, stem) in [
        (&accompaniment, &import.accompaniment),
        (&lead_vocal, &import.lead_vocal),
    ] {
        if data.len() as u64 > MAX_STEM
            || data.len() as u64 != stem.bytes
            || format!("{:x}", Sha256::digest(data)) != stem.sha256
        {
            return Err(invalid());
        }
    }
    let a = pcm(&accompaniment)?;
    let b = pcm(&lead_vocal)?;
    let decoded_duration = a.2 as f64 / a.0 as f64;
    if a != b || decoded_duration > 14400. || (decoded_duration - duration).abs() > 0.05 {
        return Err(invalid());
    }
    import.timeline.validate(decoded_duration)?;
    let (processing, processing_reference) = import.processing.presentation(a, decoded_duration)?;
    Ok(Admitted {
        package: Package {
            item_id: item,
            media_source_id: import.media_source_id,
            revision: format!("{:x}", Sha256::digest(raw)),
            duration_seconds: decoded_duration,
            processing: processing.into(),
            processing_reference,
            accompaniment: import.accompaniment,
            lead_vocal: import.lead_vocal,
            timeline: import.timeline,
        },
        accompaniment,
        lead_vocal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wave(frequency: f64) -> Vec<u8> {
        let rate = 8000u32;
        let frames = 8000u32;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + frames * 2).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * 2).to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend((frames * 2).to_le_bytes());
        for i in 0..frames {
            let sample =
                ((i as f64 * frequency * std::f64::consts::TAU / rate as f64).sin() * 2000.) as i16;
            b.extend(sample.to_le_bytes());
        }
        b
    }
    fn fixture() -> (tempfile::TempDir, Uuid, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::from_u128(1);
        let dir = root.path().join(id.to_string());
        std::fs::create_dir(&dir).unwrap();
        let source = root.path().join("original.wav");
        let a = wave(220.);
        let v = wave(440.);
        let mut mix = a.clone();
        for i in (44..mix.len()).step_by(2) {
            let left = i16::from_le_bytes(a[i..i + 2].try_into().unwrap());
            let right = i16::from_le_bytes(v[i..i + 2].try_into().unwrap());
            mix[i..i + 2].copy_from_slice(&(left + right).to_le_bytes());
        }
        std::fs::write(&source, mix).unwrap();
        std::fs::write(dir.join("accompaniment.wav"), &a).unwrap();
        std::fs::write(dir.join("lead-vocal.wav"), &v).unwrap();
        let stem = |b: &[u8]| Stem {
            sha256: format!("{:x}", Sha256::digest(b)),
            bytes: b.len() as u64,
        };
        let import = Import {
            item_id: id,
            media_source_id: None,
            source_sha256: hash_file(&source).unwrap(),
            duration_seconds: 1.,
            rights_reference: "Original synthetic fixture, authored for this test".into(),
            processing: Processing::SuppliedStems,
            accompaniment: stem(&a),
            lead_vocal: stem(&v),
            timeline: Timeline {
                offset_seconds: 0.,
                lines: vec![Line {
                    id: "a".into(),
                    voice: Voice::Lead,
                    tokens: vec![Token {
                        text: "La".into(),
                        start: 0.,
                        end: 1.,
                    }],
                }],
            },
        };
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&import).unwrap(),
        )
        .unwrap();
        (root, id, source)
    }
    #[test]
    fn real_synthetic_import_and_public_manifest() {
        let (root, id, source) = fixture();
        let value = admit(root.path(), id, &source, 1.).unwrap();
        assert_eq!(pcm(&value.accompaniment).unwrap(), (8000, 1, 8000));
        assert_ne!(value.accompaniment, value.lead_vocal);
        let public = serde_json::to_string(&value.package).unwrap();
        for private in ["rightsReference", "sourceSha256", "original.wav"] {
            assert!(!public.contains(private));
        }
    }
    #[test]
    fn public_duration_uses_pcm_frames_with_rounded_catalog_metadata() {
        let (root, id, source) = fixture();
        let value = admit(root.path(), id, &source, 1.01).unwrap();
        assert_eq!(value.package.duration_seconds, 1.0);
    }
    #[test]
    fn timeline_is_pcm_bound_with_rounded_catalog_metadata() {
        let (root, id, source) = fixture();
        let path = root.path().join(id.to_string()).join("manifest.json");
        let mut import: Import = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        import.timeline.lines[0].tokens[0].end = 1.08;
        std::fs::write(&path, serde_json::to_vec(&import).unwrap()).unwrap();
        assert!(admit(root.path(), id, &source, 1.04).is_err());
    }
    #[test]
    fn processed_import_preserves_provenance_and_rejects_unknown_models() {
        let (root, id, source) = fixture();
        let dir = root.path().join(id.to_string());
        let path = dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // Original PCM data exercises admission, not source-separation quality.
        for (name, frequency, field) in [
            ("accompaniment.wav", 220., "accompaniment"),
            ("lead-vocal.wav", 440., "leadVocal"),
        ] {
            let mut bytes = Vec::new();
            bytes.extend(b"RIFF");
            bytes.extend((36u32 + 44100 * 4).to_le_bytes());
            bytes.extend(b"WAVEfmt ");
            bytes.extend(16u32.to_le_bytes());
            bytes.extend(1u16.to_le_bytes());
            bytes.extend(2u16.to_le_bytes());
            bytes.extend(44100u32.to_le_bytes());
            bytes.extend(176400u32.to_le_bytes());
            bytes.extend(4u16.to_le_bytes());
            bytes.extend(16u16.to_le_bytes());
            bytes.extend(b"data");
            bytes.extend(176400u32.to_le_bytes());
            for frame in 0..44100 {
                let sample = ((frame as f64 * frequency * std::f64::consts::TAU / 44100.).sin()
                    * 1000.) as i16;
                bytes.extend(sample.to_le_bytes());
                bytes.extend(sample.to_le_bytes());
            }
            std::fs::write(dir.join(name), &bytes).unwrap();
            manifest[field] = serde_json::json!({"sha256":format!("{:x}",Sha256::digest(&bytes)),"bytes":bytes.len()});
        }
        manifest["processing"] = serde_json::json!({
            "kind":"modelSeparated", "modelArchiveSha256":"f3a90b39dd2874269e8b05a48a86745df897b848c61f3958efc80a39152bd692",
            "engineVersion":"0.1.0", "inputPcmSha256":"e".repeat(64)
        });
        manifest["mediaSourceId"] = serde_json::json!(id.simple().to_string());
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let public =
            serde_json::to_value(admit(root.path(), id, &source, 1.).unwrap().package).unwrap();
        assert_eq!(public["processing"], "modelSeparatedStems");
        assert_eq!(
            public["processingReference"],
            "deezer/spleeter/2stems@v1.4.0"
        );
        assert!(public.get("sourceSha256").is_none());
        assert!(public.get("inputPcmSha256").is_none());
        for (field, value) in [
            ("modelArchiveSha256", "a".repeat(64)),
            ("engineVersion", String::new()),
            ("inputPcmSha256", "not-a-digest".into()),
            ("kind", "unqualifiedModel".into()),
        ] {
            let mut invalid = manifest.clone();
            invalid["processing"][field] = serde_json::json!(value);
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(admit(root.path(), id, &source, 1.).is_err(), "{field}");
        }
    }
    #[test]
    fn rejects_source_duration_integrity_and_unrelated_item() {
        let (root, id, source) = fixture();
        assert!(admit(root.path(), id, &source, 2.).is_err());
        assert!(admit(root.path(), Uuid::from_u128(2), &source, 1.).is_err());
        std::fs::write(&source, wave(660.)).unwrap();
        assert!(admit(root.path(), id, &source, 1.).is_err());
        let (root, id, source) = fixture();
        std::fs::write(
            root.path().join(id.to_string()).join("lead-vocal.wav"),
            wave(880.),
        )
        .unwrap();
        assert!(admit(root.path(), id, &source, 1.).is_err());
    }
    #[test]
    fn rejects_unsafe_manifest_and_timing() {
        for edit in [0, 1, 2, 3] {
            let (root, id, source) = fixture();
            let path = root.path().join(id.to_string()).join("manifest.json");
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            match edit {
                0 => v["rightsReference"] = serde_json::json!(""),
                1 => v["path"] = serde_json::json!("../../other"),
                2 => v["timeline"]["lines"][0]["tokens"][0]["end"] = serde_json::json!(2),
                _ => v["itemId"] = serde_json::json!(Uuid::from_u128(2)),
            }
            std::fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
            assert!(admit(root.path(), id, &source, 1.).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_leaf_directory_and_source_symlinks() {
        use std::os::unix::fs::symlink;
        let (root, id, source) = fixture();
        let dir = root.path().join(id.to_string());
        std::fs::rename(dir.join("lead-vocal.wav"), dir.join("saved.wav")).unwrap();
        symlink("saved.wav", dir.join("lead-vocal.wav")).unwrap();
        assert!(admit(root.path(), id, &source, 1.).is_err());
        let (root, id, source) = fixture();
        let dir = root.path().join(id.to_string());
        let saved = root.path().join("saved");
        std::fs::rename(&dir, &saved).unwrap();
        symlink(&saved, &dir).unwrap();
        assert!(admit(root.path(), id, &source, 1.).is_err());
        let (root, id, source) = fixture();
        let link = root.path().join("source-link");
        symlink(source, &link).unwrap();
        assert!(admit(root.path(), id, &link, 1.).is_err());
    }
    #[test]
    fn rejects_malformed_pcm_and_oversized_import() {
        let mut b = wave(220.);
        b[20] = 3;
        assert!(pcm(&b).is_err());
        let mut b = wave(220.);
        b.push(0);
        assert!(pcm(&b).is_err());
        let (root, id, source) = fixture();
        let path = root.path().join(id.to_string()).join("manifest.json");
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(1024 * 1024 + 1)
            .unwrap();
        assert!(admit(root.path(), id, &source, 1.).is_err());
    }
}
