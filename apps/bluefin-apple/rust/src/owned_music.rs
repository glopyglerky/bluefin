//! Durable, account-free user imports. Separate from the bounded server cache.
//! Only the native Files picker supplies a source path; no network response,
//! service download, or peer cache is promoted into this catalog.
#![forbid(unsafe_code)]

use super::{err, text, Result};
use super::offline::Dir;
use bluefin_contract::owned_library::{
    self as contract, AudioFormat, ImportSource, OwnedLibrary, OwnedQueue, OwnedTrack,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const INDEX: &str = "library.json";
const INDEX_TEMP: &str = "library.json.tmp";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingImport {
    part: String,
    track: Option<OwnedTrack>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiskLibrary {
    catalog: OwnedLibrary,
    pending_import: Option<PendingImport>,
    retiring: Vec<OwnedTrack>,
}
impl DiskLibrary {
    fn validate(&self) -> Result<()> {
        self.catalog.validate().map_err(|e| e.to_string())?;
        if self.retiring.len() > 1 { return Err(err("Invalid local removal record.")); }
        for track in &self.retiring {
            track.validate().map_err(|e| e.to_string())?;
            if self.catalog.track(&track.id).is_some() { return Err(err("Local removal conflicts with the library.")); }
        }
        if let Some(pending) = &self.pending_import {
            if !valid_part(&pending.part) { return Err(err("Invalid local import record.")); }
            if let Some(track) = &pending.track { track.validate().map_err(|e| e.to_string())?; }
        }
        Ok(())
    }
}
fn random_id() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| err("The local library identity could not be created."))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_part(name: &str) -> bool {
    name.strip_prefix("import-").and_then(|s| s.strip_suffix(".part")).is_some_and(contract::valid_id)
}
fn save(root: &Dir, disk: &DiskLibrary) -> Result<()> {
    disk.validate()?;
    let bytes = serde_json::to_vec(disk).map_err(|_| err("The local library could not be encoded."))?;
    if bytes.len() > contract::MAX_CATALOG_BYTES - 1024 { return Err(err("The local music index is full.")); }
    root.remove(INDEX_TEMP)?;
    let mut file = root.open(INDEX_TEMP, true)?;
    file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|_| err("The local library could not be saved."))?;
    root.rename(INDEX_TEMP, INDEX)
}
fn load(root: &Dir) -> Result<DiskLibrary> {
    let Some(file) = root.open_existing(INDEX, false)? else {
        let disk = DiskLibrary {
            catalog: OwnedLibrary::new(random_id()?).map_err(|e| e.to_string())?,
            pending_import: None, retiring: Vec::new(),
        };
        save(root, &disk)?;
        return Ok(disk);
    };
    let mut bytes = Vec::new();
    file.take(contract::MAX_CATALOG_BYTES as u64 + 1).read_to_end(&mut bytes)
        .map_err(|_| err("The local music index could not be read."))?;
    if bytes.len() > contract::MAX_CATALOG_BYTES { return Err(err("The local music index is too large.")); }
    let disk: DiskLibrary = serde_json::from_slice(&bytes).map_err(|_| err("The local music index is invalid."))?;
    disk.validate()?;
    Ok(disk)
}
fn stamp(file: &File) -> Result<(u64, u64, u64, i64, i64, i64, i64)> {
    let value = file.metadata().map_err(|_| err("The selected audio could not be inspected."))?;
    if !value.is_file() || value.nlink() != 1 { return Err(err("Choose a regular local audio file.")); }
    Ok((value.dev(), value.ino(), value.len(), value.mtime(), value.mtime_nsec(), value.ctime(), value.ctime_nsec()))
}
fn hash_file(mut file: &File, expected: u64) -> Result<String> {
    if expected == 0 || expected > contract::MAX_AUDIO_BYTES { return Err(err("The local audio size is invalid.")); }
    file.seek(SeekFrom::Start(0)).map_err(|_| err("The local audio could not be read."))?;
    let before = stamp(file)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| err("The local audio could not be read."))?;
        if count == 0 { break }
        bytes = bytes.checked_add(count as u64).ok_or_else(|| err("The local audio is too large."))?;
        if bytes > expected { return Err(err("The local audio changed.")); }
        hash.update(&buffer[..count]);
    }
    if bytes != expected || stamp(file)? != before { return Err(err("The local audio changed.")); }
    Ok(format!("{:x}", hash.finalize()))
}
fn verify(file: &File, track: &OwnedTrack) -> Result<()> {
    if hash_file(file, track.byte_length)? != track.id {
        return Err(err("An imported audio file changed. Remove its library entry and import it again."));
    }
    Ok(())
}
fn recover(root: &Dir, media: &Dir, disk: &mut DiskLibrary) -> Result<()> {
    if !disk.retiring.is_empty() {
        for track in &disk.retiring { media.remove(&track.file_name())?; }
        media.sync()?;
        disk.retiring.clear();
        save(root, disk)?;
    }
    let Some(pending) = disk.pending_import.clone() else { return Ok(()) };
    let Some(track) = pending.track else {
        media.remove(&pending.part)?;
        media.sync()?;
        disk.pending_import = None;
        return save(root, disk);
    };
    let final_name = track.file_name();
    if let Some(existing) = media.open_existing(&final_name, false)? {
        verify(&existing, &track)?;
        media.remove(&pending.part)?;
        media.sync()?;
    } else if let Some(part) = media.open_existing(&pending.part, false)? {
        verify(&part, &track)?;
        media.rename(&pending.part, &final_name)?;
    } else {
        // No import was acknowledged before catalog commit. Missing uncommitted
        // bytes leave the previously acknowledged library intact.
        disk.pending_import = None;
        return save(root, disk);
    }
    disk.catalog.import_track(track).map_err(|e| e.to_string())?;
    disk.pending_import = None;
    save(root, disk)
}
fn expected(v: &Value, disk: &DiskLibrary) -> Result<()> {
    let revision = v["expectedRevision"].as_u64().ok_or_else(|| err("Refresh the local library before changing it."))?;
    disk.catalog.check_revision(text(v, "libraryId")?, revision).map_err(|e| e.to_string())
}
fn value(catalog: &OwnedLibrary) -> Result<Value> {
    serde_json::to_value(catalog).map_err(|_| err("The local library could not be read."))
}
fn import(v: &Value, root: &Dir, media: &Dir, disk: &mut DiskLibrary) -> Result<Value> {
    expected(v, disk)?;
    if disk.catalog.items.len() >= contract::MAX_ITEMS { return Err(err("The local music library is full.")); }
    let source = Path::new(text(v, "sourcePath")?);
    if !source.is_absolute() || source.starts_with(root.path()) {
        return Err(err("Choose an audio file outside this library."));
    }
    // A Files security scope may admit the selected file without admitting a
    // directory listing of its provider. Pin that regular file directly; all
    // destination mutations still use the private library's directory FDs.
    let mut input = OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(source).map_err(|_| err("The selected audio is unavailable or is a link."))?;
    let before = stamp(&input)?;
    let length = before.2;
    if length == 0 || length > contract::MAX_AUDIO_BYTES { return Err(err("Choose an audio file smaller than 2 GiB.")); }
    let mut prefix = [0_u8; 64];
    let count = input.read(&mut prefix).map_err(|_| err("The selected audio could not be read."))?;
    let extension = source.extension().and_then(|v| v.to_str()).ok_or_else(|| err("The audio file needs a supported extension."))?;
    let format = AudioFormat::detect(&prefix[..count], extension).map_err(|e| e.to_string())?;
    input.seek(SeekFrom::Start(0)).map_err(|_| err("The selected audio could not be read."))?;
    let title = text(v, "title")?.trim().to_owned();
    let imported_at_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| err("The device clock is unavailable."))?.as_millis();
    let imported_at_unix_ms = u64::try_from(imported_at_unix_ms).map_err(|_| err("The device clock is invalid."))?;
    let mut track = OwnedTrack { id: "0".repeat(64), title, format, byte_length: length,
        imported_at_unix_ms, source: ImportSource::UserFileImport };
    track.validate().map_err(|e| e.to_string())?;
    let part = format!("import-{}.part", random_id()?);
    disk.pending_import = Some(PendingImport { part: part.clone(), track: None });
    save(root, disk)?;
    let mut output = media.open(&part, true)?;
    let mut hash = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|_| err("The selected audio could not be copied."))?;
        if count == 0 { break }
        copied = copied.checked_add(count as u64).ok_or_else(|| err("The selected audio is too large."))?;
        if copied > length { return Err(err("The selected file changed during import. Retry it.")); }
        output.write_all(&buffer[..count]).map_err(|_| err("There is not enough available storage to import this audio."))?;
        hash.update(&buffer[..count]);
    }
    if copied != length || stamp(&input)? != before { return Err(err("The selected file changed during import. Retry it.")); }
    output.sync_all().map_err(|_| err("The imported audio could not be saved."))?;
    media.sync()?;
    track.id = format!("{:x}", hash.finalize());
    // Recovery may publish this file and must then be able to commit its catalog
    // record. Reject conflicts before retaining a completed pending import. An
    // error here leaves only the uncommitted part, which ordinary recovery removes.
    let mut proposed = disk.catalog.clone();
    let imported = proposed.import_track(track.clone()).map_err(|e| e.to_string())?;
    let id = track.id.clone();
    disk.pending_import = Some(PendingImport { part, track: Some(track) });
    save(root, disk)?;
    recover(root, media, disk)?;
    Ok(json!({"catalog": disk.catalog, "itemId": id, "imported": imported}))
}

pub(super) fn call(v: &Value, operation: &str) -> Result<Value> {
    // Account credentials are irrelevant here. Never pass this catalog through
    // Api or the server cache's account-scoped storage namespace.
    let root = Dir::root(text(v, "libraryRoot")?)?;
    root.lock()?;
    let media = root.child("media", true)?;
    let mut disk = load(&root)?;
    recover(&root, &media, &mut disk)?;
    match operation {
        "ownedLibraryList" => value(&disk.catalog),
        "ownedLibraryImport" => import(v, &root, &media, &mut disk),
        "ownedLibraryRemove" => {
            expected(v, &disk)?;
            let id = text(v, "itemId")?;
            if !contract::valid_id(id) { return Err(err("The local music item is invalid.")); }
            if let Some(track) = disk.catalog.track(id).cloned() {
                disk.catalog.remove_track(id).map_err(|e| e.to_string())?;
                disk.retiring = vec![track];
                save(&root, &disk)?;
                recover(&root, &media, &mut disk)?;
            }
            value(&disk.catalog)
        },
        "ownedLibraryQueue" | "ownedLibrarySelect" | "ownedLibraryAdvance" => {
            expected(v, &disk)?;
            match operation {
                "ownedLibraryQueue" => {
                    let queue: OwnedQueue = serde_json::from_value(v["queue"].clone())
                        .map_err(|_| err("The local music queue is invalid."))?;
                    disk.catalog.set_queue(queue).map_err(|e| e.to_string())?;
                },
                "ownedLibrarySelect" => { disk.catalog.select(text(v, "itemId")?).map_err(|e| e.to_string())?; },
                _ => {
                    let forward = v["forward"].as_bool().ok_or_else(|| err("Invalid local queue direction."))?;
                    let automatic = v["automatic"].as_bool().ok_or_else(|| err("Invalid local queue action."))?;
                    disk.catalog.advance(forward, automatic).map_err(|e| e.to_string())?;
                },
            }
            save(&root, &disk)?;
            value(&disk.catalog)
        },
        "ownedLibraryPlay" => {
            let track = disk.catalog.track(text(v, "itemId")?).ok_or_else(|| err("This local audio is unavailable."))?;
            let file = media.open(&track.file_name(), false)?;
            verify(&file, track)?;
            let url = url::Url::from_file_path(media.path().join(track.file_name())).map_err(|_| err("The local audio path is invalid."))?;
            Ok(json!({"url": url.as_str(), "item": track}))
        },
        "ownedLibrarySearch" => {
            let query = v.get("query").and_then(Value::as_str).unwrap_or("");
            let items = disk.catalog.search(query).map_err(|e| e.to_string())?;
            Ok(json!({"items":items, "libraryId":disk.catalog.library_id, "revision":disk.catalog.revision}))
        },
        _ => Err(err("Unsupported local music operation.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("bluefin-owned-{}", random_id().unwrap()));
            std::fs::create_dir(&root).unwrap();
            std::fs::create_dir(root.join("library")).unwrap();
            std::fs::write(root.join("song.wav"), b"RIFF0000WAVEdataexampleaudio").unwrap();
            Self(root.canonicalize().unwrap())
        }
        fn input(&self) -> Value { json!({"libraryRoot":self.0.join("library").to_str().unwrap()}) }
        fn import(&self) -> Value {
            let mut input = self.input();
            let catalog = call(&input, "ownedLibraryList").unwrap();
            input["libraryId"] = catalog["libraryId"].clone();
            input["expectedRevision"] = catalog["revision"].clone();
            input["sourcePath"] = json!(self.0.join("song.wav").to_str().unwrap());
            input["title"] = json!("My song");
            input
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn local_import_survives_reopen_and_deduplicates_without_an_account() {
        let fixture = Fixture::new();
        let mut request = fixture.import();
        request["operation"] = json!("ownedLibraryImport");
        let first = crate::call(&request).unwrap();
        assert_eq!(first["imported"], true);
        let second = call(&fixture.import(), "ownedLibraryImport").unwrap();
        assert_eq!(second["imported"], false);
        assert_eq!(first["catalog"], second["catalog"]);
        let mut request = fixture.input();
        request["itemId"] = first["itemId"].clone();
        let playable = call(&request, "ownedLibraryPlay").unwrap();
        assert!(playable["url"].as_str().unwrap().starts_with("file:"));
        let catalog = call(&fixture.input(), "ownedLibraryList").unwrap();
        assert_eq!(catalog, first["catalog"]);
        assert_eq!(catalog["items"][0]["source"], "user_file_import");
    }

    #[test]
    fn conflicting_import_does_not_poison_the_saved_library() {
        let fixture = Fixture::new();
        // ID3 metadata is permitted by both bounded container checks. Native
        // playback validation happens later and cannot repair an import ledger.
        let original = fixture.0.join("song.mp3");
        let conflicting = fixture.0.join("song.aac");
        let bytes = b"ID3same-original-audio-bytes";
        std::fs::write(&original, bytes).unwrap();
        std::fs::write(&conflicting, bytes).unwrap();
        let mut first_request = fixture.import();
        first_request["sourcePath"] = json!(original.to_str().unwrap());
        let first = call(&first_request, "ownedLibraryImport").unwrap();
        let mut second_request = fixture.import();
        second_request["sourcePath"] = json!(conflicting.to_str().unwrap());
        assert!(call(&second_request, "ownedLibraryImport").is_err());

        let reopened = call(&fixture.input(), "ownedLibraryList").unwrap();
        assert_eq!(reopened, first["catalog"]);
        let mut playback = fixture.input();
        playback["itemId"] = first["itemId"].clone();
        assert!(call(&playback, "ownedLibraryPlay").unwrap()["url"]
            .as_str().unwrap().ends_with(".mp3"));
        let media = fixture.0.join("library/media");
        assert_eq!(std::fs::read_dir(media).unwrap().count(), 1);
        assert_eq!(std::fs::read(original).unwrap(), bytes);
        assert_eq!(std::fs::read(conflicting).unwrap(), bytes);
        assert_eq!(call(&fixture.import(), "ownedLibraryImport").unwrap()["imported"], true);
    }

    #[test]
    fn stale_writes_and_symlink_sources_have_no_import_effect() {
        let fixture = Fixture::new();
        let mut request = fixture.import();
        request["expectedRevision"] = json!(1);
        assert!(call(&request, "ownedLibraryImport").is_err());
        assert!(call(&fixture.input(), "ownedLibraryList").unwrap()["items"].as_array().unwrap().is_empty());
        symlink(fixture.0.join("song.wav"), fixture.0.join("linked.wav")).unwrap();
        request = fixture.import();
        request["sourcePath"] = json!(fixture.0.join("linked.wav").to_str().unwrap());
        assert!(call(&request, "ownedLibraryImport").is_err());
        assert!(call(&fixture.input(), "ownedLibraryList").unwrap()["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn committed_import_and_pending_removal_recover_after_restart() {
        let fixture = Fixture::new();
        let first = call(&fixture.import(), "ownedLibraryImport").unwrap();
        let id = first["itemId"].as_str().unwrap();
        {
            let root = Dir::root(fixture.0.join("library").to_str().unwrap()).unwrap();
            root.lock().unwrap();
            let media = root.child("media", false).unwrap();
            let mut disk = load(&root).unwrap();
            let track = disk.catalog.track(id).unwrap().clone();
            // Crash window after file rename and before catalog commit.
            disk.catalog.remove_track(id).unwrap();
            disk.pending_import = Some(PendingImport { part: format!("import-{}.part", "a".repeat(64)), track: Some(track.clone()) });
            save(&root, &disk).unwrap();
            recover(&root, &media, &mut disk).unwrap();
            assert_eq!(disk.catalog.track(id), Some(&track));
            disk.catalog.remove_track(id).unwrap();
            disk.retiring = vec![track];
            save(&root, &disk).unwrap();
        }
        assert!(call(&fixture.input(), "ownedLibraryList").unwrap()["items"].as_array().unwrap().is_empty());
        let media = fixture.0.join("library/media");
        assert_eq!(std::fs::read_dir(media).unwrap().count(), 0);
    }

    #[test]
    fn changed_bytes_fail_playback_without_deleting_other_files() {
        let fixture = Fixture::new();
        let first = call(&fixture.import(), "ownedLibraryImport").unwrap();
        let id = first["itemId"].as_str().unwrap();
        let media = fixture.0.join("library/media");
        std::fs::write(media.join(format!("{id}.wav")), b"altered").unwrap();
        std::fs::write(media.join("unrelated.txt"), b"keep").unwrap();
        let mut request = fixture.input(); request["itemId"] = json!(id);
        assert!(call(&request, "ownedLibraryPlay").is_err());
        request["libraryId"] = first["catalog"]["libraryId"].clone();
        request["expectedRevision"] = first["catalog"]["revision"].clone();
        assert!(call(&request, "ownedLibraryRemove").unwrap()["items"].as_array().unwrap().is_empty());
        assert_eq!(std::fs::read(media.join("unrelated.txt")).unwrap(), b"keep");
    }
}
