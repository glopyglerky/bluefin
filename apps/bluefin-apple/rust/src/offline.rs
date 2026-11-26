//! Account-isolated, bounded native audio cache. All mutations use directory FDs.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::{FileExt, MetadataExt},
    path::{Component, Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex, OnceLock},
    thread::JoinHandle,
};
const MAX: u64 = 256 * 1024 * 1024;
const QUOTA: u64 = 2 * 1024 * 1024 * 1024;
const ITEMS: usize = 100;
// Android permits searching system ancestors without granting directory reads.
// O_DIRECTORY keeps O_PATH | O_NOFOLLOW from admitting a symlink descriptor.
#[cfg(any(target_os = "linux", target_os = "android"))]
const WALK_ACCESS: i32 = libc::O_PATH;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const WALK_ACCESS: i32 = libc::O_RDONLY;
fn hash(s: &[u8]) -> String {
    format!("{:x}", Sha256::digest(s))
}
fn account(v: &Value) -> Result<String> {
    Ok(hash(
        format!("{}\n{}", server(v)?, text(v, "userId")?).as_bytes(),
    ))
}
pub(super) struct Dir {
    file: File,
    path: PathBuf,
}
impl Dir {
    pub(super) fn root(raw: &str) -> Result<Self> {
        let p = Path::new(raw);
        if !p.is_absolute() {
            return Err(err("Cache root must be absolute"));
        }
        if p == Path::new("/") {
            return Err(err("Cache root cannot be filesystem root"));
        }
        let fd = unsafe {
            libc::open(
                c"/".as_ptr(),
                WALK_ACCESS | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(err("Cache root unavailable"));
        }
        let mut d = Self {
            file: unsafe { File::from_raw_fd(fd) },
            path: "/".into(),
        };
        for c in p.components() {
            match c {
                Component::RootDir => {}
                Component::Normal(n) => {
                    d = d.child_with_access(
                        n.to_str().ok_or_else(|| err("Invalid cache path"))?,
                        false,
                        WALK_ACCESS,
                    )?;
                }
                _ => return Err(err("Invalid cache path")),
            }
        }
        // Reopen the pinned final inode, not its absolute name. Cache operations
        // need read access and fsync/flock, which a path-only handle cannot supply.
        let mut readable = d.child(".", false)?;
        readable.path = d.path;
        Ok(readable)
    }
    pub(super) fn child(&self, n: &str, create: bool) -> Result<Self> {
        self.child_with_access(n, create, libc::O_RDONLY)
    }
    fn child_with_access(&self, n: &str, create: bool, access: i32) -> Result<Self> {
        let c = CString::new(n).map_err(|_| err("Invalid cache name"))?;
        if create {
            let rc = unsafe { libc::mkdirat(self.file.as_raw_fd(), c.as_ptr(), 0o700) };
            if rc != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(err("Cannot create cache directory"));
            }
        }
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c.as_ptr(),
                access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(err("Unsafe or unavailable cache directory"));
        }
        Ok(Self {
            file: unsafe { File::from_raw_fd(fd) },
            path: self.path.join(n),
        })
    }
    pub(super) fn open(&self, n: &str, write: bool) -> Result<File> {
        self.open_existing(n, write)?
            .ok_or_else(|| err("Unsafe or unavailable cache file"))
    }
    pub(super) fn open_existing(&self, n: &str, write: bool) -> Result<Option<File>> {
        let c = CString::new(n).map_err(|_| err("Invalid cache name"))?;
        let flags = if write {
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY | libc::O_NONBLOCK
        };
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            if !write && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(err("Unsafe or unavailable cache file"));
        }
        let f = unsafe { File::from_raw_fd(fd) };
        let m = f.metadata().map_err(|_| err("Cache stat failed"))?;
        if !m.is_file() || m.nlink() != 1 {
            return Err(err("Cache entry is not a regular file"));
        }
        Ok(Some(f))
    }
    pub(super) fn remove(&self, n: &str) -> Result<()> {
        let c = CString::new(n).map_err(|_| err("Invalid cache name"))?;
        let rc = unsafe { libc::unlinkat(self.file.as_raw_fd(), c.as_ptr(), 0) };
        if rc != 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound {
            return Err(err("Cache removal failed"));
        }
        Ok(())
    }
    pub(super) fn rename(&self, a: &str, b: &str) -> Result<()> {
        let a = CString::new(a).unwrap();
        let b = CString::new(b).unwrap();
        if unsafe {
            libc::renameat(
                self.file.as_raw_fd(),
                a.as_ptr(),
                self.file.as_raw_fd(),
                b.as_ptr(),
            )
        } != 0
        {
            return Err(err("Cache commit failed"));
        }
        self.file.sync_all().map_err(|_| err("Cache sync failed"))
    }
    pub(super) fn lock(&self) -> Result<()> {
        if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(err("Cache is in use by another operation"));
        }
        Ok(())
    }
    pub(super) fn path(&self) -> &Path { &self.path }
    pub(super) fn sync(&self) -> Result<()> {
        self.file.sync_all().map_err(|_| err("Local storage sync failed"))
    }
    fn read_json(&self, n: &str) -> Result<Value> {
        self.read_optional_json(n)?
            .ok_or_else(|| err("Unsafe or unavailable cache file"))
    }
    fn read_optional_json(&self, n: &str) -> Result<Option<Value>> {
        let Some(file) = self.open_existing(n, false)? else {
            return Ok(None);
        };
        let mut b = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut b)
            .map_err(|_| err("Cache read failed"))?;
        if b.len() > 1024 * 1024 {
            return Err(err("Cache metadata too large"));
        }
        serde_json::from_slice(&b)
            .map(Some)
            .map_err(|_| err("Invalid cache metadata"))
    }
    fn save(&self, n: &str, v: &Value) -> Result<()> {
        if v.to_string().len() > 1024 * 1024 {
            return Err(err("Cache metadata too large"));
        }
        let temp = format!("{n}.tmp");
        self.remove(&temp)?;
        let mut f = self.open(&temp, true)?;
        f.write_all(v.to_string().as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|_| err("Cache write failed"))?;
        self.rename(&temp, n)
    }
}
fn directory(v: &Value) -> Result<Dir> {
    Dir::root(text(v, "cacheRoot")?)?.child(&format!("bluefin-{}", account(v)?), true)
}
fn catalog(d: &Dir) -> Result<Vec<Value>> {
    // Fixed catalog avoids unbounded directory traversal.
    let Some(v) = d.read_optional_json("catalog.json")? else {
        return Ok(vec![]);
    };
    let a = v
        .as_array()
        .filter(|a| a.len() <= ITEMS)
        .ok_or_else(|| err("Invalid cache catalog"))?;
    Ok(a.clone())
}
#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl FileStamp {
    fn of(m: &std::fs::Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            bytes: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
}
type VerificationCache = std::collections::HashMap<(u64, u64, String), (FileStamp, String)>;
static VERIFIED: OnceLock<Mutex<VerificationCache>> = OnceLock::new();

const CONTAINERS: &[&str] = &["m4a", "mp3", "aac", "flac", "wav", "aiff"];
fn container(value: &str) -> Result<&str> {
    if value == "mp4" {
        return Ok("m4a");
    }
    CONTAINERS
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| err("Invalid cached container"))
}
fn audio_name(d: &Dir, record: &Value) -> Result<String> {
    let key = hash(text(record, "itemId")?.as_bytes());
    if let Some(value) = record.get("container") {
        return Ok(format!(
            "{key}.{}",
            container(
                value
                    .as_str()
                    .ok_or_else(|| err("Invalid cached container"))?
            )?
        ));
    }
    let legacy = format!("{key}.audio");
    if d.open(&legacy, false).is_ok() {
        return Ok(legacy);
    }
    // An interrupted legacy repair may have renamed the file before a catalog save.
    let candidates = CONTAINERS
        .iter()
        .map(|extension| format!("{key}.{extension}"))
        .filter(|name| d.open(name, false).is_ok())
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(err("Missing or ambiguous cached audio"));
    }
    Ok(candidates[0].clone())
}
fn remove_audio(d: &Dir, id: &str) -> Result<()> {
    let key = hash(id.as_bytes());
    for extension in CONTAINERS.iter().copied().chain(["audio"]) {
        d.remove(&format!("{key}.{extension}"))?;
    }
    Ok(())
}

fn verified(d: &Dir, r: &Value) -> Result<Value> {
    let id = text(r, "itemId")?;
    let key = hash(id.as_bytes());
    let mut name = audio_name(d, r)?;
    let mut f = d.open(&name, false)?;
    let size = r["bytes"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 1024 * 1024 * 1024)
        .ok_or_else(|| err("Invalid cached size"))?;
    let metadata = f.metadata().map_err(|_| err("Cache stat failed"))?;
    if metadata.len() != size {
        return Err(err("Cached audio size mismatch"));
    }
    let expected = r["sha256"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| err("Invalid cached hash"))?;
    let stamp = FileStamp::of(&metadata);
    let parent = d.file.metadata().map_err(|_| err("Cache stat failed"))?;
    let cache_key = (parent.dev(), parent.ino(), key.clone());
    let cache = VERIFIED.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let unchanged = cache.lock().ok().is_some_and(|entries| {
        entries
            .get(&cache_key)
            .is_some_and(|(previous, hash)| *previous == stamp && hash == expected)
    });
    if !unchanged {
        let mut reader = (&mut f).take(size + 1);
        let mut h = Sha256::new();
        let mut b = [0; 32768];
        loop {
            let n = reader
                .read(&mut b)
                .map_err(|_| err("Cache verification failed"))?;
            if n == 0 {
                break;
            }
            h.update(&b[..n]);
        }
        if expected != format!("{:x}", h.finalize())
            || FileStamp::of(&f.metadata().map_err(|_| err("Cache stat failed"))?) != stamp
        {
            return Err(err("Cached audio hash mismatch"));
        }
        if let Ok(mut entries) = cache.lock() {
            if entries.len() >= 256 {
                entries.clear();
            }
            entries.insert(cache_key, (stamp, expected.into()));
        }
    }
    if name.ends_with(".audio") {
        // Preserve legacy bytes and their digest while repairing the native type hint.
        let mut head = [0; 16];
        let read = f
            .read_at(&mut head, 0)
            .map_err(|_| err("Cache type inspection failed"))?;
        let head = &head[..read];
        let extension = if head.starts_with(b"fLaC") {
            "flac"
        } else if head.get(4..8) == Some(b"ftyp") {
            "m4a"
        } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
            "wav"
        } else if head.starts_with(b"FORM") && matches!(head.get(8..12), Some(b"AIFF" | b"AIFC")) {
            "aiff"
        } else if head.starts_with(b"ID3") {
            "mp3"
        } else if head.len() >= 2 && head[0] == 255 && head[1] & 246 == 240 {
            "aac"
        } else if head.len() >= 2 && head[0] == 255 && head[1] & 224 == 224 {
            "mp3"
        } else {
            return Err(err("Unknown legacy cached audio format"));
        };
        d.lock()?;
        let repaired = format!("{key}.{extension}");
        if d.open(&repaired, false).is_ok() {
            return Err(err("Conflicting cached audio files"));
        }
        d.rename(&name, &repaired)?;
        name = repaired;
    }
    let mut out = r.clone();
    out.as_object_mut()
        .ok_or_else(|| err("Invalid cache record"))?
        .remove("sha256");
    out["localUrl"] = json!(Url::from_file_path(d.path.join(name))
        .map_err(|_| err("Invalid cache URL"))?
        .as_str());
    Ok(out)
}
struct Job {
    server: String,
    account: String,
    root: String,
    item: String,
    cancel: Arc<AtomicBool>,
    state: Arc<Mutex<Value>>,
    handle: Option<JoinHandle<()>>,
}
static JOB: OnceLock<Mutex<Option<Job>>> = OnceLock::new();
fn manager() -> &'static Mutex<Option<Job>> {
    JOB.get_or_init(|| Mutex::new(None))
}
fn stop(j: &mut Job) -> Result<()> {
    j.cancel.store(true, Ordering::Release);
    if let Some(h) = j.handle.take() {
        h.join().map_err(|_| err("Download worker failed"))?;
    }
    Ok(())
}
pub(super) fn logout(v: &Value) -> Result<()> {
    let a = if v["userId"].is_string() {
        Some(account(v)?)
    } else {
        None
    };
    let base = server(v)?.to_string();
    let mut m = manager().lock().map_err(|_| err("Cache busy"))?;
    if let Some(j) = m
        .as_mut()
        .filter(|j| j.server == base && a.as_ref().is_none_or(|a| j.account == *a))
    {
        stop(j)?;
        *m = None;
    }
    Ok(())
}

/// Join the account's writer before removing only this cache namespace's entries.
pub(super) fn purge(v: &Value) -> Result<()> {
    let account = account(v)?;
    let mut current = manager().lock().map_err(|_| err("Cache busy"))?;
    if let Some(job) = current.as_mut().filter(|j| j.account == account) {
        stop(job)?;
        *current = None;
    }
    let directory = directory(v)?;
    directory.lock()?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&directory.path).map_err(|_| err("Cache unavailable"))? {
        let name = entry
            .map_err(|_| err("Cache unavailable"))?
            .file_name()
            .into_string()
            .map_err(|_| err("Unexpected cache entry"))?;
        let owned = matches!(
            name.as_str(),
            "catalog.json" | "catalog.json.tmp" | "pending.json" | "pending.json.tmp"
        ) || name.split_once('.').is_some_and(|(key, extension)| {
            key.len() == 64
                && key.bytes().all(|b| b.is_ascii_hexdigit())
                && (CONTAINERS.contains(&extension) || matches!(extension, "audio" | "part"))
        });
        if !owned || names.len() >= 1024 {
            return Err(err("Unexpected cache entry; local cleanup incomplete"));
        }
        names.push(name);
    }
    for name in names {
        directory.remove(&name)?;
    }
    directory
        .file
        .sync_all()
        .map_err(|_| err("Cache cleanup sync failed"))
}
pub(super) fn call(v: &Value, op: &str) -> Result<Value> {
    let a = account(v)?;
    let d = directory(v)?;
    let root = d.path.to_string_lossy().to_string();
    let mut m = manager().lock().map_err(|_| err("Cache busy"))?;
    if op == "downloadCancel" || op == "downloadRemove" {
        let id = text(v, "itemId")?;
        if let Some(j) = m
            .as_mut()
            .filter(|j| j.account == a && j.root == root && j.item == id)
        {
            stop(j)?;
            if op == "downloadCancel" {
                return Ok(json!({"cancelled":true}));
            }
            *m = None;
        }
        if op == "downloadRemove" {
            d.lock()?;
            let mut c = catalog(&d)?;
            let key = hash(id.as_bytes());
            c.retain(|r| r["itemId"] != id);
            d.save("catalog.json", &json!(c))?;
            remove_audio(&d, id)?;
            d.remove(&format!("{key}.part"))?;
            return Ok(json!({"removed":true}));
        }
        return Ok(json!({"cancelled":true}));
    }
    let mut c = catalog(&d)?;
    if op == "downloads" {
        let mut records = Vec::new();
        for r in &c {
            records.push(verified(&d,r).unwrap_or_else(|_|json!({"itemId":r["itemId"],"state":"failed","bytes":0,"totalBytes":0,"error":"Cached audio failed verification"})));
        }
        if let Some(j) = m.as_ref().filter(|j| j.account == a && j.root == root) {
            let s = j.state.lock().map_err(|_| err("Cache busy"))?.clone();
            if !records.iter().any(|r| r["itemId"] == s["itemId"]) {
                records.push(s);
            }
        }
        return Ok(
            json!({"Items":records,"maxBytes":MAX,"quotaBytes":QUOTA,"maxWorkers":1,"maxItems":ITEMS}),
        );
    }
    if m.as_ref()
        .is_some_and(|j| j.handle.as_ref().is_some_and(|h| !h.is_finished()))
    {
        return Err(err("Download worker busy"));
    }
    if let Some(mut j) = m.take() {
        if let Some(h) = j.handle.take() {
            h.join().map_err(|_| err("Download worker failed"))?;
        }
    }
    d.lock()?;
    d.remove("download.part")?;
    if let Ok(pending) = d.read_json("pending.json") {
        if let Some(old) = pending.as_str() {
            if !c.iter().any(|r| r["itemId"] == old) {
                remove_audio(&d, old)?;
            }
        }
        d.remove("pending.json")?;
    }
    let id = text(v, "itemId")?.to_owned();
    if c.iter().any(|r| r["itemId"] == id) {
        return Err(err("Download already exists; remove before replacing"));
    }
    if c.len() >= ITEMS {
        return Err(err("Download item limit reached"));
    }
    let max = count(v, "maxBytes", MAX, 1024 * 1024 * 1024)?;
    let quota = count(v, "quotaBytes", QUOTA, 8 * 1024 * 1024 * 1024)?;
    let used = c.iter().try_fold(0u64, |sum, r| {
        sum.checked_add(
            r["bytes"]
                .as_u64()
                .ok_or_else(|| err("Invalid cache size"))?,
        )
        .ok_or_else(|| err("Cache size overflow"))
    })?;
    if max == 0 || used.checked_add(max).is_none_or(|n| n > quota) {
        return Err(err("Insufficient download quota"));
    }
    let api = Api::new(v)?;
    api.auth(true)?; // Fail before creating worker for invalid credentials.
    d.save("pending.json", &json!(id))?;
    let state = Arc::new(Mutex::new(
        json!({"itemId":id,"state":"downloading","bytes":0,"totalBytes":0}),
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let s = state.clone();
    let flag = cancel.clone();
    let input = v.clone();
    let item = id.clone();
    let handle = std::thread::Builder::new()
        .name("bluefin-download".into())
        .spawn(move || {
            let result = download(&input, &d, &item, max, &flag, &s);
            let _ = d.remove("download.part");
            match result {
                Ok(record) => {
                    c.push(record.clone());
                    let result = d
                        .save("catalog.json", &json!(c))
                        .and_then(|_| verified(&d, &record));
                    match result {
                        Ok(out) => {
                            if let Ok(mut x) = s.lock() {
                                *x = out;
                            }
                        }
                        Err(_) => {
                            let _ = remove_audio(&d, &item);
                            if let Ok(mut x) = s.lock() {
                                x["state"] = json!("failed");
                                x["error"] = json!("Cache commit failed");
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Ok(mut x) = s.lock() {
                        x["state"] = json!(if flag.load(Ordering::Acquire) {
                            "cancelled"
                        } else {
                            "failed"
                        });
                        x["error"] = json!(e);
                    }
                }
            }
            let _ = d.remove("pending.json");
        })
        .map_err(|_| err("Cannot start download worker"))?;
    let out = state.lock().map_err(|_| err("Cache busy"))?.clone();
    *m = Some(Job {
        server: server(v)?.to_string(),
        account: a,
        root,
        item: id,
        cancel,
        state,
        handle: Some(handle),
    });
    Ok(out)
}
fn download(
    v: &Value,
    d: &Dir,
    id: &str,
    max: u64,
    cancel: &AtomicBool,
    state: &Mutex<Value>,
) -> Result<Value> {
    let api = Api::new(v)?;
    if cancel.load(Ordering::Acquire) {
        return Err(err("Download cancelled"));
    }
    let me = api.get(&["Users", "Me"], vec![])?;
    if cancel.load(Ordering::Acquire) {
        return Err(err("Download cancelled"));
    }
    let norm = |s: &str| s.replace('-', "").to_ascii_lowercase();
    if norm(text(&me, "Id")?) != norm(text(v, "userId")?) {
        return Err(err("Download account does not match authenticated user"));
    }
    if me["Policy"]["EnableContentDownloading"] != true {
        return Err(err("Account does not permit downloads"));
    }
    let item = api.get(
        &["Users", text(v, "userId")?, "Items", id],
        vec![("fields", premium::FIELDS.into())],
    )?;
    if cancel.load(Ordering::Acquire) {
        return Err(err("Download cancelled"));
    }
    if item["Type"] != "Audio" || item["MediaType"] != "Audio" {
        return Err(err("Only audio items may be downloaded"));
    }
    if premium::project(&item).to_string().len() > 8192 {
        return Err(err("Offline item metadata too large"));
    }
    // PlaybackInfo with IsPlayback=false never opens a live or transcode session.
    let info=api.post(&["Items",id,"PlaybackInfo"],json!({"UserId":text(v,"userId")?,"IsPlayback":false,"AutoOpenLiveStream":false,"EnableDirectPlay":true,"EnableDirectStream":false,"EnableTranscoding":false,"DeviceProfile":profile()}))?;
    if cancel.load(Ordering::Acquire) {
        return Err(err("Download cancelled"));
    }
    if info.get("ErrorCode").is_some_and(|x| !x.is_null()) {
        return Err(err("Server denied download source"));
    }
    let source = info["MediaSources"]
        .as_array()
        .and_then(|a| {
            a.iter().find(|s| {
                native_audio(s)
                    && s["Id"].as_str().is_some_and(|x| norm(x) == norm(id))
                    && s["IsInfiniteStream"] != true
                    && s["RequiresOpening"] != true
                    && s["Protocol"].as_str().is_none_or(|p| p == "File")
            })
        })
        .ok_or_else(|| err("No native direct audio download source"))?;
    let url = api.url(&["Items", id, "Download"])?;
    let (url, _mesh_tunnel) = api.media_fetch(url)?;
    if cancel.load(Ordering::Acquire) {
        return Err(err("Download cancelled"));
    }
    let mut r = api
        .client
        .get(url)
        .header("Authorization", api.auth(true)?)
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|_| err("Download request failed"))?;
    if r.status() != reqwest::StatusCode::OK {
        return Err(format!("Download returned HTTP {}", r.status().as_u16()));
    }
    if r.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|t| t.to_ascii_lowercase().contains("mpegurl"))
    {
        return Err(err("HLS cannot be downloaded as offline audio"));
    }
    let expected = r
        .content_length()
        .filter(|n| *n > 0 && *n <= max)
        .ok_or_else(|| err("Download needs a bounded Content-Length"))?;
    if source["Size"].as_u64().is_some_and(|n| n != expected) {
        return Err(err("Download source size mismatch"));
    }
    let key = hash(id.as_bytes());
    let part = "download.part".to_owned();
    d.remove(&part)?;
    let mut f = d.open(&part, true)?;
    let mut h = Sha256::new();
    let mut bytes = 0;
    let mut signature = Vec::with_capacity(16);
    let mut buf = [0; 32768];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(err("Download cancelled"));
        }
        let n = r.read(&mut buf).map_err(|_| err("Download read failed"))?;
        if n == 0 {
            break;
        }
        signature.extend_from_slice(&buf[..n.min(16 - signature.len())]);
        bytes += n as u64;
        if bytes > expected || bytes > max {
            return Err(err("Download exceeds size limit"));
        }
        f.write_all(&buf[..n])
            .map_err(|_| err("Download storage failed"))?;
        h.update(&buf[..n]);
        let mut s = state.lock().map_err(|_| err("Cache busy"))?;
        s["bytes"] = json!(bytes);
        s["totalBytes"] = json!(expected);
    }
    let native_signature = match source["Container"].as_str() {
        Some("flac") => signature.starts_with(b"fLaC"),
        Some("mp3") => {
            signature.starts_with(b"ID3")
                || signature.len() >= 2 && signature[0] == 255 && signature[1] & 224 == 224
        }
        Some("aac") => signature.len() >= 2 && signature[0] == 255 && signature[1] & 246 == 240,
        Some("mp4" | "m4a") => signature.get(4..8) == Some(b"ftyp"),
        Some("wav") => signature.starts_with(b"RIFF") && signature.get(8..12) == Some(b"WAVE"),
        Some("aiff") => {
            signature.starts_with(b"FORM")
                && matches!(signature.get(8..12), Some(b"AIFF" | b"AIFC"))
        }
        _ => false,
    };
    if !native_signature {
        return Err(err(
            "Downloaded bytes are not the admitted native audio format",
        ));
    }
    if bytes != expected {
        return Err(err("Incomplete download"));
    }
    if cancel.load(Ordering::Acquire) {
        return Err(err("Download cancelled"));
    }
    f.sync_all()
        .map_err(|_| err("Download storage sync failed"))?;
    let container = container(
        source["Container"]
            .as_str()
            .ok_or_else(|| err("Missing download container"))?,
    )?;
    d.rename(&part, &format!("{key}.{container}"))?;
    Ok(
        json!({"itemId":id,"state":"completed","bytes":bytes,"totalBytes":expected,"container":container,"sha256":format!("{:x}",h.finalize()),"item":premium::project(&item)}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    static SERIAL: Mutex<()> = Mutex::new(());
    #[test]
    fn purge_is_scoped_and_preserves_unknown_user_files() {
        let _serial = SERIAL.lock().unwrap();
        let path = root();
        let request =
            json!({"server":"https://example.com","userId":"purge-owner","cacheRoot":path});
        let owned = directory(&request).unwrap();
        owned.save("catalog.json", &json!([])).unwrap();
        owned
            .open(&format!("{}.part", hash(b"song")), true)
            .unwrap()
            .write_all(b"partial")
            .unwrap();
        let mut other = request.clone();
        other["userId"] = json!("other-owner");
        let untouched = directory(&other).unwrap();
        untouched.save("catalog.json", &json!([])).unwrap();
        purge(&request).unwrap();
        assert!(owned.open("catalog.json", false).is_err());
        assert!(untouched.open("catalog.json", false).is_ok());
        owned
            .open("user-note.txt", true)
            .unwrap()
            .write_all(b"preserve")
            .unwrap();
        assert!(purge(&request).is_err());
        assert!(owned.open("user-note.txt", false).is_ok());
        std::fs::remove_dir_all(path).unwrap();
    }
    fn root() -> PathBuf {
        let p = PathBuf::from(
            std::env::var("BLUEFIN_TEST_CACHE")
                .expect("BLUEFIN_TEST_CACHE must name bounded build storage"),
        )
        .join(format!("offline-{}-{}", std::process::id(), getrandom_id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn getrandom_id() -> u64 {
        let mut b = [0; 8];
        getrandom::fill(&mut b).unwrap();
        u64::from_ne_bytes(b)
    }
    fn input(p: &Path) -> Value {
        json!({"operation":"downloadStart","server":"https://example.com","userId":"u","token":"secret","cacheRoot":p.to_str().unwrap(),"itemId":"song","maxBytes":4096,"quotaBytes":8192})
    }
    fn settle() {
        let mut m = manager().lock().unwrap();
        if let Some(j) = m.as_mut() {
            if let Some(h) = j.handle.take() {
                h.join().unwrap();
            }
        }
    }
    #[test]
    fn verification_cache_rechecks_same_size_changes() {
        let p = root();
        let v = input(&p);
        let d = directory(&v).unwrap();
        let name = format!("{}.audio", hash(b"song"));
        let mut f = d.open(&name, true).unwrap();
        f.write_all(b"fLaCdata").unwrap();
        f.sync_all().unwrap();
        let record =
            json!({"itemId":"song","state":"completed","bytes":8,"sha256":hash(b"fLaCdata")});
        let repaired = verified(&d, &record).unwrap();
        let repaired_url = Url::parse(repaired["localUrl"].as_str().unwrap())
            .unwrap()
            .to_file_path()
            .unwrap();
        assert_eq!(repaired_url.extension().unwrap(), "flac");
        assert!(!d.path.join(&name).exists());
        assert!(verified(&d, &record).is_ok());
        std::fs::write(&repaired_url, b"fLaCevil").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&repaired_url)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        assert!(verified(&d, &record).is_err());
        drop(f);
        drop(d);
        std::fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn invalid_container_cannot_select_or_mutate_a_cache_path() {
        let p = root();
        let v = input(&p);
        let d = directory(&v).unwrap();
        let name = format!("{}.audio", hash(b"song"));
        let mut f = d.open(&name, true).unwrap();
        f.write_all(b"fLaCdata").unwrap();
        f.sync_all().unwrap();
        for invalid in [
            "../../outside",
            "flac/secret",
            "FLAC",
            "flac?token=secret",
            "",
        ] {
            let record =
                json!({"itemId":"song","bytes":8,"sha256":hash(b"fLaCdata"),"container":invalid});
            assert!(verified(&d, &record).is_err());
            assert_eq!(std::fs::read(d.path.join(&name)).unwrap(), b"fLaCdata");
        }
        let corrupt = json!({"itemId":"song","bytes":8,"sha256":hash(b"bad data")});
        assert!(verified(&d, &corrupt).is_err());
        assert!(
            d.path.join(name).exists(),
            "Unverified legacy bytes were renamed"
        );
        drop(f);
        drop(d);
        std::fs::remove_dir_all(p).unwrap();
    }

    fn mock(bad: bool, slow: bool) -> (String, JoinHandle<()>, std::sync::mpsc::Receiver<()>) {
        mock_payload(bad, slow, "fLaC")
    }
    fn mock_payload(
        bad: bool,
        slow: bool,
        payload: &str,
    ) -> (String, JoinHandle<()>, std::sync::mpsc::Receiver<()>) {
        let payload = payload.to_owned();
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let h = std::thread::spawn(move || {
            let bodies=[json!({"Id":"u","Policy":{"EnableContentDownloading":true}}).to_string(),json!({"Id":"song","Type":"Audio","MediaType":"Audio"}).to_string(),json!({"MediaSources":[{"Id":"song","Container":"flac","Size":payload.len(),"SupportsDirectPlay":true,"Protocol":"File","MediaStreams":[{"Type":"Audio","Codec":"flac"}]}]}).to_string(),payload];
            for (i, b) in bodies.iter().enumerate() {
                let (mut s, _) = l.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut buf = [0; 16384];
                let n = s.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.contains("Token=\"secret\""));
                assert!(!req.lines().next().unwrap().contains("secret"));
                if i == 3 {
                    assert!(req.starts_with("GET /Items/song/Download "));
                    if slow {
                        tx.send(()).unwrap();
                        std::thread::sleep(Duration::from_millis(150));
                    }
                }
                let status = if bad && i == 3 {
                    "403 Forbidden"
                } else {
                    "200 OK"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                    b.len()
                );
                let _ = s.write_all(response.as_bytes());
            }
        });
        (format!("http://{addr}"), h, rx)
    }
    #[test]
    fn persistent_download_account_integrity_remove_and_failure() {
        let _serial = SERIAL.lock().unwrap();
        let p = root();
        let mut v = input(&p);
        let (server, h, _) = mock(false, false);
        v["server"] = json!(server);
        v["allowLocalHttp"] = json!(true);
        call(&v, "downloadStart").unwrap();
        settle();
        h.join().unwrap();
        let listed = call(&v, "downloads").unwrap();
        assert_eq!(listed["Items"][0]["state"], "completed");
        let url = Url::parse(listed["Items"][0]["localUrl"].as_str().unwrap()).unwrap();
        assert_eq!(std::fs::read(url.to_file_path().unwrap()).unwrap(), b"fLaC");
        let catalog = directory(&v).unwrap().read_json("catalog.json").unwrap();
        assert!(!catalog.to_string().contains("secret"));
        *manager().lock().unwrap() = None;
        assert_eq!(
            call(&v, "downloads").unwrap()["Items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let mut other = v.clone();
        other["userId"] = json!("other");
        assert!(call(&other, "downloads").unwrap()["Items"]
            .as_array()
            .unwrap()
            .is_empty());
        std::fs::write(url.to_file_path().unwrap(), b"fake").unwrap();
        assert_eq!(
            call(&v, "downloads").unwrap()["Items"][0]["state"],
            "failed"
        );
        call(&v, "downloadRemove").unwrap();
        assert!(!url.to_file_path().unwrap().exists());
        let (server, h, _) = mock(true, false);
        v["server"] = json!(server);
        call(&v, "downloadStart").unwrap();
        settle();
        h.join().unwrap();
        assert_eq!(
            call(&v, "downloads").unwrap()["Items"][0]["state"],
            "failed"
        );
        assert!(!directory(&v).unwrap().path.join("download.part").exists());
        *manager().lock().unwrap() = None;
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn cancellation_and_logout_join_worker() {
        let _serial = SERIAL.lock().unwrap();
        for logout in [false, true] {
            let p = root();
            let mut v = input(&p);
            let (server, h, rx) = mock(false, true);
            v["server"] = json!(server);
            v["allowLocalHttp"] = json!(true);
            call(&v, "downloadStart").unwrap();
            rx.recv_timeout(Duration::from_secs(4)).unwrap();
            if logout {
                super::logout(&v).unwrap();
            } else {
                call(&v, "downloadCancel").unwrap();
                assert_eq!(
                    call(&v, "downloads").unwrap()["Items"][0]["state"],
                    "cancelled"
                );
            }
            h.join().unwrap();
            assert!(catalog(&directory(&v).unwrap()).unwrap().is_empty());
            *manager().lock().unwrap() = None;
            std::fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn rejects_hls_bytes_and_cross_account_token() {
        let _serial = SERIAL.lock().unwrap();
        let p = root();
        let mut v = input(&p);
        let (server, h, _) = mock_payload(false, false, "#EXTM3U");
        v["server"] = json!(server);
        v["allowLocalHttp"] = json!(true);
        call(&v, "downloadStart").unwrap();
        settle();
        h.join().unwrap();
        assert_eq!(
            call(&v, "downloads").unwrap()["Items"][0]["state"],
            "failed"
        );
        assert!(catalog(&directory(&v).unwrap()).unwrap().is_empty());
        *manager().lock().unwrap() = None;
        let (server, h) = crate::tests::mock(
            "200 OK",
            r#"{"Id":"another-user","Policy":{"EnableContentDownloading":true}}"#,
        );
        v["server"] = json!(server);
        call(&v, "downloadStart").unwrap();
        settle();
        h.join().unwrap();
        assert!(call(&v, "downloads").unwrap()["Items"][0]["error"]
            .as_str()
            .unwrap()
            .contains("account"));
        *manager().lock().unwrap() = None;
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn cache_walk_accepts_search_only_ancestors_without_following_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "requires an unprivileged test user"
        );
        let p = root();
        let ancestor = p.join("search-only");
        let cache = ancestor.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        symlink(&cache, ancestor.join("linked-cache")).unwrap();
        symlink(&ancestor, p.join("linked-ancestor")).unwrap();
        struct Cleanup(PathBuf, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.1, std::fs::Permissions::from_mode(0o700));
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(p.clone(), ancestor.clone());
        std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o111)).unwrap();
        assert_eq!(
            File::open(&ancestor).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let d = Dir::root(cache.to_str().unwrap()).unwrap();
        let owned = d.child("owned", true).unwrap();
        owned.lock().unwrap();
        owned.save("catalog.json", &json!([])).unwrap();
        assert_eq!(owned.read_json("catalog.json").unwrap(), json!([]));
        assert!(Dir::root(ancestor.join("linked-cache").to_str().unwrap()).is_err());
        assert!(Dir::root(p.join("linked-ancestor/cache").to_str().unwrap()).is_err());
        assert!(Dir::root("/").is_err());
        assert!(Dir::root("relative/cache").is_err());
        assert!(Dir::root(ancestor.join("../search-only/cache").to_str().unwrap()).is_err());
    }
    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[ignore = "requires process-scoped seccomp fault injection"]
    fn missing_catalog_does_not_require_access_probe() {
        if std::env::var_os("BLUEFIN_ACCESS_PROBE_CHILD").is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "offline::tests::missing_catalog_does_not_require_access_probe",
                    "--ignored",
                    "--nocapture",
                ])
                .env("BLUEFIN_ACCESS_PROBE_CHILD", "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        // Only this disposable test thread is constrained. Simulate an unavailable
        // access-check syscall; this does not identify Android's actual errno.
        let instruction = |code, jt, jf, k| libc::sock_filter { code, jt, jf, k };
        let mut filter = [
            instruction(0x20, 0, 0, 0), // load syscall number
            instruction(0x15, 1, 0, libc::SYS_faccessat as u32),
            instruction(0x15, 0, 1, libc::SYS_faccessat2 as u32),
            instruction(0x06, 0, 0, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
            instruction(0x06, 0, 0, libc::SECCOMP_RET_ALLOW),
        ];
        let program = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_mut_ptr(),
        };
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
            0
        );
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) },
            0
        );
        assert_eq!(
            unsafe {
                libc::syscall(
                    libc::SYS_faccessat,
                    libc::AT_FDCWD,
                    c"/".as_ptr(),
                    libc::F_OK,
                )
            },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ENOSYS)
        );
        let p = root();
        let result = call(&input(&p), "downloads");
        std::fs::remove_dir_all(p).unwrap();
        assert_eq!(result.unwrap()["Items"], json!([]));
    }
    #[test]
    fn missing_catalog_is_empty_but_unsafe_or_invalid_entries_are_errors() {
        let p = root();
        let v = input(&p);
        let d = directory(&v).unwrap();
        assert!(catalog(&d).unwrap().is_empty());
        let outside = p.join("foreign.json");
        std::fs::write(&outside, b"[]").unwrap();
        std::os::unix::fs::symlink(p.join("absent"), d.path.join("catalog.json")).unwrap();
        assert!(catalog(&d).is_err());
        d.remove("catalog.json").unwrap();
        std::fs::hard_link(&outside, d.path.join("catalog.json")).unwrap();
        assert!(catalog(&d).is_err());
        d.remove("catalog.json").unwrap();
        std::fs::create_dir(d.path.join("catalog.json")).unwrap();
        assert!(catalog(&d).is_err());
        std::fs::remove_dir(d.path.join("catalog.json")).unwrap();
        for invalid in ["", "invalid JSON", "null", "{}"] {
            std::fs::write(d.path.join("catalog.json"), invalid).unwrap();
            assert!(catalog(&d).is_err());
            assert_eq!(
                std::fs::read_to_string(d.path.join("catalog.json")).unwrap(),
                invalid
            );
        }
        std::fs::write(d.path.join("catalog.json"), b"[]").unwrap();
        assert!(catalog(&d).unwrap().is_empty());
        assert_eq!(std::fs::read(&outside).unwrap(), b"[]");
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn path_quota_and_storage_fail_closed() {
        let _serial = SERIAL.lock().unwrap();
        let p = root();
        let mut v = input(&p);
        v["quotaBytes"] = json!(4095);
        assert!(call(&v, "downloadStart").unwrap_err().contains("quota"));
        let link = p.join("link");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        v["cacheRoot"] = json!(link);
        assert!(call(&v, "downloads").is_err());
        v["cacheRoot"] = json!(p);
        let d = directory(&v).unwrap();
        std::os::unix::fs::symlink(p.join("outside"), d.path.join("catalog.json")).unwrap();
        assert!(call(&v, "downloads").is_err());
        d.remove("catalog.json").unwrap();
        std::fs::create_dir(d.path.join("catalog.json.tmp")).unwrap();
        assert!(d.save("catalog.json", &json!([])).is_err());
        assert!(!p.join("outside").exists());
        std::fs::remove_dir_all(p).unwrap();
    }
}
