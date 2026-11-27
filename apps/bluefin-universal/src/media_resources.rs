//! Narrow resource selectors shared by the browser mesh adapter.
//! These selectors never accept an origin, credential, or arbitrary media URL.
pub const MAX_KARAOKE_STEM_BYTES: usize = 64 * 1024 * 1024;

pub struct KaraokeStemResource {
    pub path: String,
    pub source: String,
    pub bytes: usize,
}
pub fn karaoke_stem(
    item: &str, source: &str, revision: &str, role: &str, bytes: u32,
) -> Result<KaraokeStemResource, String> {
    let item = uuid::Uuid::parse_str(item).map_err(|_| "Invalid Sing item.")?;
    let source = uuid::Uuid::parse_str(source).map_err(|_| "Invalid Sing source.")?;
    if item.is_nil() || source.is_nil() || !digest(revision)
        || !matches!(role, "accompaniment" | "lead-vocal")
        || !(44..=MAX_KARAOKE_STEM_BYTES).contains(&(bytes as usize)) {
        return Err("Invalid Sing resource.".into());
    }
    Ok(KaraokeStemResource {
        path: format!("Bluefin/Karaoke/{}/{revision}/Stems/{role}", item.simple()),
        source: source.simple().to_string(),
        bytes: bytes as usize,
    })
}
pub fn sports_logo(asset: &str) -> Result<String, String> {
    if !digest(asset) { return Err("Invalid team artwork.".into()); }
    Ok(format!("sports/api/v1/logos/{asset}"))
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub fn wav_matches(bytes: &[u8], expected: usize) -> bool {
    bytes.len() == expected && bytes.len() >= 44
        && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
        && u64::from(u32::from_le_bytes(bytes[4..8].try_into().unwrap())) + 8 == bytes.len() as u64
}
pub fn image_matches(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const ITEM: &str = "00000000-0000-0000-0000-000000000001";
    const SOURCE: &str = "00000000-0000-0000-0000-000000000002";
    fn revision() -> String { "a".repeat(64) }
    #[test]
    fn stem_selectors_bind_item_source_revision_role_and_size() {
        let value = karaoke_stem(ITEM, SOURCE, &revision(), "lead-vocal", 44).unwrap();
        assert_eq!(value.path, format!("Bluefin/Karaoke/00000000000000000000000000000001/{}/Stems/lead-vocal", revision()));
        assert_eq!(value.source, "00000000000000000000000000000002");
        for role in ["../accompaniment", "lead-vocal?api_key=x", "vocals", ""] {
            assert!(karaoke_stem(ITEM, SOURCE, &revision(), role, 44).is_err());
        }
        for id in ["", "https://other.example/file", "00000000-0000-0000-0000-000000000000"] {
            assert!(karaoke_stem(id, SOURCE, &revision(), "accompaniment", 44).is_err());
            assert!(karaoke_stem(ITEM, id, &revision(), "accompaniment", 44).is_err());
        }
        for bytes in [0, 43, MAX_KARAOKE_STEM_BYTES as u32 + 1, u32::MAX] {
            assert!(karaoke_stem(ITEM, SOURCE, &revision(), "accompaniment", bytes).is_err());
        }
    }
    #[test]
    fn artwork_only_accepts_the_exact_public_asset_identifier() {
        assert_eq!(sports_logo(&revision()).unwrap(), format!("sports/api/v1/logos/{}", revision()));
        for value in ["", "../logo.png", "https://a.espncdn.com/i/teamlogos/nhl/500/bos.png", &"g".repeat(64), &"a".repeat(65)] {
            assert!(sports_logo(value).is_err());
        }
    }
    #[test]
    fn truncated_wrong_type_and_wrong_length_audio_is_rejected() {
        let mut bytes = vec![0; 44];
        bytes[..4].copy_from_slice(b"RIFF");
        bytes[4..8].copy_from_slice(&36u32.to_le_bytes());
        bytes[8..12].copy_from_slice(b"WAVE");
        assert!(wav_matches(&bytes, 44));
        assert!(!wav_matches(&bytes, 45));
        assert!(!wav_matches(&bytes[..20], 20));
        bytes[4] = 35;
        assert!(!wav_matches(&bytes, 44));
        assert!(!wav_matches(b"<html>access denied</html>", 25));
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!wav_matches(&bytes, 44));
    }
    #[test]
    fn artwork_type_must_match_raster_bytes() {
        assert!(image_matches("image/png", b"\x89PNG\r\n\x1a\n"));
        assert!(!image_matches("image/png", b"<svg/>"));
        assert!(!image_matches("image/svg+xml", b"<svg/>"));
        assert!(!image_matches("image/jpeg", b"\x89PNG\r\n\x1a\n"));
    }
}
