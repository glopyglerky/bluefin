use super::{Reason, WorkResult, authority::Source};
use bluefin_karaoke::{Admitted, Timeline};
use bluefin_karaoke_engine::Cancellation;
use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
use serde::Deserialize;
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    sync::Arc,
    time::Instant,
};
use tokio::sync::OwnedSemaphorePermit;

fn invalid() -> std::io::Error {
    std::io::Error::other("invalid private karaoke storage")
}
fn directory_at(parent: &File, name: &str) -> std::io::Result<File> {
    Ok(File::from(fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?))
}
fn file_at(parent: &File, name: &str) -> std::io::Result<File> {
    let file = File::from(fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    let m = file.metadata()?;
    if !m.is_file()
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.nlink() != 1
        || m.mode() & 0o077 != 0
    {
        return Err(invalid());
    }
    Ok(file)
}
pub(super) fn private_directory(path: &Path, create_leaf: bool) -> std::io::Result<File> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut dir = File::open("/")?;
    let parts: Vec<_> = path
        .components()
        .filter(|p| *p != Component::RootDir)
        .collect();
    if parts.is_empty() {
        return Err(invalid());
    }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        let name = name.to_str().ok_or_else(invalid)?;
        if index + 1 == parts.len() && create_leaf {
            match fs::mkdirat(&dir, name, Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
        }
        dir = directory_at(&dir, name)?;
    }
    let m = dir.metadata()?;
    if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o777 != 0o700 {
        return Err(invalid());
    }
    Ok(dir)
}
pub(super) fn trusted_bytes(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let file = bluefin_karaoke::open_regular(path)?;
    let m = file.metadata()?;
    if m.uid() != rustix::process::geteuid().as_raw() || m.nlink() != 1 || m.mode() & 0o022 != 0 {
        return Err(invalid());
    }
    read_descriptor(file, limit)
}
pub(crate) fn read_descriptor(mut file: File, limit: u64) -> std::io::Result<Vec<u8>> {
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || bytes.len() as u64 != file.metadata()?.len() {
        return Err(invalid());
    }
    Ok(bytes)
}
fn names(dir: &File) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::Dir::read_from(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_str().map_err(|_| invalid())?;
        if name != "." && name != ".." {
            names.push(name.to_owned());
        }
        if names.len() > 1024 {
            return Err(invalid());
        }
    }
    Ok(names)
}
fn generated_name(name: &str, prefix: &str, length: usize) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == length && suffix.bytes().all(|b| b.is_ascii_hexdigit())
    })
}
fn cleanup(parent: &File, name: &str, files: &[&str]) -> std::io::Result<()> {
    let dir = directory_at(parent, name)?;
    let meta = dir.metadata()?;
    if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o777 != 0o700 {
        return Err(invalid());
    }
    let entries = names(&dir)?;
    // Preserve unexpected members and symlinks for operator reconciliation.
    for entry in &entries {
        if !files.contains(&entry.as_str()) {
            return Err(invalid());
        }
        file_at(&dir, entry)?;
    }
    for entry in entries {
        fs::unlinkat(&dir, &entry, AtFlags::empty())?;
    }
    fs::unlinkat(parent, name, AtFlags::REMOVEDIR)?;
    parent.sync_all()
}
const CACHE_FILES: [&str; 4] = [
    "manifest.json",
    "receipt.json",
    "accompaniment.wav",
    "lead-vocal.wav",
];
const ENGINE_FILES: [&str; 3] = ["provenance.json", "accompaniment.wav", "lead-vocal.wav"];
const MARKER: &[u8] = b"bluefin-owned-model-cache-v1\n";
pub(super) struct Store {
    root: File,
    cache: File,
    pub staging: File,
    _lock: File,
    poisoned: std::sync::atomic::AtomicBool,
}
impl Store {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let root = private_directory(path, true)?;
        // Only a previously marked dedicated cache or a new empty directory is
        // eligible for recovery; supplied imports can never be swept here.
        if !names(&root)?.contains(&"owner".to_owned()) {
            if !names(&root)?.is_empty() {
                return Err(invalid());
            }
            let mut marker = File::from(fs::openat(
                &root,
                "owner",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )?);
            marker.write_all(MARKER)?;
            marker.sync_all()?;
            root.sync_all()?;
        }
        let lock = file_at(&root, "owner")?;
        fs::flock(&lock, fs::FlockOperation::NonBlockingLockExclusive)?;
        if read_descriptor(lock.try_clone()?, 128)? != MARKER {
            return Err(invalid());
        }
        if names(&root)?.iter().any(|name| {
            ![
                "owner",
                "cache",
                "staging",
                "requests.json",
                "requests.next",
            ]
            .contains(&name.as_str())
        }) {
            return Err(invalid());
        }
        if names(&root)?.contains(&"requests.next".to_owned()) {
            file_at(&root, "requests.next")?;
            fs::unlinkat(&root, "requests.next", AtFlags::empty())?;
        }
        for name in ["cache", "staging"] {
            match fs::mkdirat(&root, name, Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let cache = directory_at(&root, "cache")?;
        let staging = directory_at(&root, "staging")?;
        for (dir, engine) in [(&cache, false), (&staging, true)] {
            let m = dir.metadata()?;
            if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o777 != 0o700 {
                return Err(invalid());
            }
            for name in names(dir)? {
                let recognized = if engine {
                    generated_name(&name, ".pending-", 32)
                        || generated_name(&name, "processed-", 32)
                } else {
                    generated_name(&name, "pending-", 32) || generated_name(&name, "prepared-", 64)
                };
                if !recognized {
                    return Err(invalid());
                }
                cleanup(
                    dir,
                    &name,
                    if engine { &ENGINE_FILES } else { &CACHE_FILES },
                )?;
            }
        }
        Ok(Self {
            root,
            cache,
            staging,
            _lock: lock,
            poisoned: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn healthy(&self) -> bool {
        !self.poisoned.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn poison(&self) {
        self.poisoned
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn journal(&self) -> std::io::Result<Option<Vec<u8>>> {
        if !names(&self.root)?.contains(&"requests.json".to_owned()) {
            return Ok(None);
        }
        read_descriptor(file_at(&self.root, "requests.json")?, 256 * 1024).map(Some)
    }
    pub fn save_journal(&self, bytes: &[u8]) -> std::io::Result<()> {
        if !self.healthy() || bytes.len() > 256 * 1024 {
            return Err(invalid());
        }
        if names(&self.root)?.contains(&"requests.json".to_owned()) {
            file_at(&self.root, "requests.json")?;
        }
        let mut file = File::from(fs::openat(
            &self.root,
            "requests.next",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::renameat(&self.root, "requests.next", &self.root, "requests.json")?;
        self.root.sync_all()
    }
    // Publication carries its disk permit and final authority check through the
    // same atomic write; neither is optional or detached from cleanup.
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        self: &Arc<Self>,
        key: String,
        raw: &[u8],
        receipt: &[u8],
        value: &Admitted,
        budget: OwnedSemaphorePermit,
        cancel: &Cancellation,
        before_publish: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<Arc<CacheEntry>> {
        if !self.healthy() {
            return Err(invalid());
        }
        let name = format!("pending-{}", uuid::Uuid::new_v4().simple());
        fs::mkdirat(&self.cache, &name, Mode::from_raw_mode(0o700))?;
        let mut entry = CacheEntry {
            key,
            name,
            store: Arc::clone(self),
            created: Instant::now(),
            budget: Some(budget),
        };
        let dir = directory_at(&self.cache, &entry.name)?;
        for (name, bytes) in [
            ("manifest.json", raw),
            ("receipt.json", receipt),
            ("accompaniment.wav", value.accompaniment.as_slice()),
            ("lead-vocal.wav", value.lead_vocal.as_slice()),
        ] {
            let mut file = File::from(fs::openat(
                &dir,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )?);
            for chunk in bytes.chunks(65536) {
                cancel.check().map_err(std::io::Error::other)?;
                file.write_all(chunk)?;
            }
            file.sync_all()?;
        }
        dir.sync_all()?;
        before_publish()?;
        cancel.check().map_err(std::io::Error::other)?;
        let target = format!("prepared-{}", entry.key);
        fs::renameat_with(
            &self.cache,
            &entry.name,
            &self.cache,
            &target,
            RenameFlags::NOREPLACE,
        )?;
        entry.name = target;
        self.cache.sync_all()?;
        self.root.sync_all()?;
        Ok(Arc::new(entry))
    }
    pub fn read(
        &self,
        entry: &CacheEntry,
        source: &Source,
        original: &str,
    ) -> std::io::Result<Admitted> {
        let dir = directory_at(&self.cache, &entry.name)?;
        bluefin_karaoke::admit_bytes(
            &read_descriptor(file_at(&dir, "manifest.json")?, 1024 * 1024)?,
            source.item,
            Some(&source.selected),
            original,
            source.duration,
            read_descriptor(
                file_at(&dir, "accompaniment.wav")?,
                bluefin_karaoke::MAX_STEM,
            )?,
            read_descriptor(file_at(&dir, "lead-vocal.wav")?, bluefin_karaoke::MAX_STEM)?,
        )
    }
}
pub(super) struct CacheEntry {
    pub key: String,
    name: String,
    store: Arc<Store>,
    pub created: Instant,
    budget: Option<OwnedSemaphorePermit>,
}
impl Drop for CacheEntry {
    fn drop(&mut self) {
        if cleanup(&self.store.cache, &self.name, &CACHE_FILES).is_err() {
            tracing::error!("karaoke owned cache cleanup requires reconciliation");
            // Do not release disk budget after unsuccessful cleanup.
            if let Some(budget) = self.budget.take() {
                budget.forget();
            }
            self.store.poison();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TimingFile {
    item_id: uuid::Uuid,
    media_source_id: String,
    source_sha256: String,
    rights_reference: String,
    timing_kind: String,
    timeline: Timeline,
}
pub(super) struct Timing {
    pub hash: String,
    pub rights: String,
    pub timeline: Timeline,
}
impl Timing {
    pub fn load(root: &Path, source: &Source, original: &str) -> WorkResult<Self> {
        let path = root
            .join(source.item.to_string())
            .join(format!("{}.json", source.selected));
        let raw = trusted_bytes(&path, 1024 * 1024).map_err(|_| Reason::TimingRequired)?;
        let timing: TimingFile =
            serde_json::from_slice(&raw).map_err(|_| Reason::TimingRequired)?;
        if timing.item_id != source.item
            || timing.media_source_id != source.selected
            || timing.source_sha256 != original
            || !matches!(timing.timing_kind.as_str(), "word" | "syllable")
            || timing.rights_reference.trim().is_empty()
            || timing.rights_reference.len() > 4096
            || timing.timeline.validate(source.duration).is_err()
        {
            return Err(Reason::TimingRequired);
        }
        Ok(Self {
            hash: super::digest(&raw),
            rights: timing.rights_reference,
            timeline: timing.timeline,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn exclusive_private_cache_recovery_preserves_unknown_and_supplied_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("owned");
        let store = Store::open(&path).unwrap();
        assert!(Store::open(&path).is_err());
        let name = format!("pending-{}", uuid::Uuid::new_v4().simple());
        std::fs::create_dir(path.join("cache").join(&name)).unwrap();
        std::fs::set_permissions(
            path.join("cache").join(&name),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(!path.join("cache").join(name).exists());
        std::fs::write(path.join("cache/user-supplied"), b"preserve").unwrap();
        drop(store);
        assert!(Store::open(&path).is_err());
        assert_eq!(
            std::fs::read(path.join("cache/user-supplied")).unwrap(),
            b"preserve"
        );
        let other = root.path().join("other");
        std::fs::create_dir(&other).unwrap();
        symlink(&other, root.path().join("link")).unwrap();
        assert!(Store::open(&root.path().join("link")).is_err());
    }
    #[test]
    fn symlink_member_and_unmarked_nonempty_root_are_not_swept() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("owned");
        let store = Store::open(&path).unwrap();
        let target = root.path().join("user-data");
        std::fs::write(&target, b"keep").unwrap();
        let name = format!("pending-{}", uuid::Uuid::new_v4().simple());
        let dir = path.join("cache").join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&target, dir.join("manifest.json")).unwrap();
        drop(store);
        assert!(Store::open(&path).is_err());
        assert!(dir.join("manifest.json").is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        let unmarked = root.path().join("supplied");
        std::fs::create_dir(&unmarked).unwrap();
        std::fs::set_permissions(&unmarked, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(unmarked.join("manifest.json"), b"keep").unwrap();
        assert!(Store::open(&unmarked).is_err());
    }
    #[test]
    fn publication_never_overwrites_and_cleanup_debt_retains_budget() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&root.path().join("owned")).unwrap());
        let budget = Arc::new(tokio::sync::Semaphore::new(4));
        let value=Admitted {package:serde_json::from_value(serde_json::json!({
            "itemId":uuid::Uuid::new_v4(),"revision":"a".repeat(64),"durationSeconds":1.,"processing":"suppliedStems",
            "accompaniment":{"sha256":"a".repeat(64),"bytes":4},"leadVocal":{"sha256":"a".repeat(64),"bytes":4},
            "timeline":{"offsetSeconds":0.,"lines":[]}})).unwrap(),accompaniment:b"stem".to_vec(),lead_vocal:b"stem".to_vec()};
        let token = Cancellation::default();
        let key = "a".repeat(64);
        let entry = store
            .publish(
                key.clone(),
                b"original",
                b"receipt",
                &value,
                Arc::clone(&budget).try_acquire_owned().unwrap(),
                &token,
                || Ok(()),
            )
            .unwrap();
        assert!(
            store
                .publish(
                    key,
                    b"overwrite",
                    b"receipt",
                    &value,
                    Arc::clone(&budget).try_acquire_owned().unwrap(),
                    &token,
                    || Ok(())
                )
                .is_err()
        );
        let dir = directory_at(&store.cache, &entry.name).unwrap();
        assert_eq!(
            read_descriptor(file_at(&dir, "manifest.json").unwrap(), 100).unwrap(),
            b"original"
        );
        let mut extra = File::from(
            fs::openat(
                &dir,
                "unexpected",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                Mode::from_raw_mode(0o600),
            )
            .unwrap(),
        );
        extra.write_all(b"preserve").unwrap();
        drop(extra);
        drop(entry);
        assert!(!store.healthy());
        assert_eq!(
            budget.available_permits(),
            3,
            "failed cleanup retains its reservation"
        );
        assert_eq!(names(&store.cache).unwrap().len(), 1);
    }
}
