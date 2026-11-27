//! Opt-in server-private karaoke preparation. No client paths, model uploads,
//! inferred timing or provider extraction enter this service.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) mod authority;
mod decode;
pub(crate) mod storage;

use crate::{ApiError, AppState};
pub(crate) use authority::Session;
use authority::Source;
use bluefin_karaoke::{Admitted, Import, Processing, Stem};
use bluefin_karaoke_engine::{Cancellation, Engine, Input, StagingRoot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use storage::{CacheEntry, Store};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

const HISTORY_LIMIT: usize = 256;
const HISTORY_TTL: Duration = Duration::from_secs(600);
const CACHE_TTL: Duration = Duration::from_secs(3600);
const QUEUE_LIMIT: usize = 3; // one executing, at most two waiting
const MODEL_HASH: &str = "f3a90b39dd2874269e8b05a48a86745df897b848c61f3958efc80a39152bd692";

/// Trusted process-start configuration, read only from an operator-owned file.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// Official archive on the server. Never packaged in clients.
    pub model_archive: PathBuf,
    /// Dedicated private generated-cache directory; separate from supplied imports.
    pub cache_root: PathBuf,
    /// Private operator metadata: ITEM_UUID/SOURCE_SIMPLE_UUID.json.
    pub timing_root: PathBuf,
    /// Hard application disk reservation budget, including staging and copies.
    pub cache_bytes: u32,
    /// Startup environment and actual Candle pools must match, in 1..=4.
    pub cpu_threads: usize,
    /// End-to-end deadline including queueing; synchronous kernels/syscalls
    /// remain cooperative and their actual cleanup join is always retained.
    pub deadline_seconds: u64,
}

/// Closed, intentionally small client-visible preparation states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Admitted and waiting for the single heavy worker.
    Queued,
    /// A worker still owns execution and resource guards.
    Preparing,
    /// Joined, validated and atomically cached.
    Ready,
    /// Unsupported input, timing or resource availability.
    Unavailable,
    /// Preparation failed with a closed generic reason.
    Failed,
    /// Stop requested; actual worker/cleanup join is still outstanding.
    Cancelling,
    /// Work and cleanup have joined, or a DELETE tombstone exists.
    Cancelled,
}
impl State {
    fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Preparing | Self::Cancelling)
    }
}
/// Closed reasons contain no paths or internal error messages.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// No matching authorized supplied word/syllable timing.
    TimingRequired,
    /// More than the model's inclusive 300-second maximum.
    DurationUnsupported,
    /// Unsupported or changed original source.
    UnsupportedSource,
    /// Reserved protocol reason; disabled servers feature-detect with 404.
    ServerDisabled,
    /// Bounded admission or cache budget exhausted.
    Busy,
    /// A processing or cleanup error, without internal detail.
    PreparationFailed,
}
/// Stable preparation response. Request id never supplies authority.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Requested real library item.
    pub item_id: Uuid,
    /// Owner-scoped idempotency identifier.
    pub job_id: Uuid,
    /// The real selected source, when resolved (null until selection succeeds).
    pub media_source_id: Option<String>,
    /// Current lifecycle state, with ready only after actual worker join.
    pub state: State,
    /// Recommended bounded client retry delay.
    pub retry_after_milliseconds: u32,
    /// Optional closed generic reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Key {
    owner: [u8; 32],
    item: Uuid,
    request: Uuid,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    records: Vec<StoppedRequest>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoppedRequest {
    key: Key,
    selection: Option<String>,
    created_unix_ms: u64,
}
struct Record {
    status: Status,
    selection: Option<String>,
    control: Arc<Control>,
    created: Instant,
}
#[derive(Default)]
struct Control {
    cancel: Cancellation,
    changed: Notify,
}
impl Control {
    fn stop(&self) {
        self.cancel.cancel();
        self.changed.notify_one();
    }
}
struct Table {
    stopped: bool,
    jobs: HashMap<Key, Record>,
    caches: HashMap<String, Arc<CacheEntry>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
struct Shared {
    config: Config,
    store: Arc<Store>,
    engine: Mutex<Engine>,
    table: Mutex<Table>,
    queue: Arc<Semaphore>,
    heavy: Arc<Semaphore>,
    budget: Arc<Semaphore>,
    shutdown: tokio::sync::Mutex<()>,
}
/// One startup-owned model, bounded queue, cache and cancellation/join domain.
#[derive(Clone)]
pub struct Service(Arc<Shared>);

fn failed() -> ApiError {
    ApiError::ServiceUnavailable("karaoke preparation unavailable".into())
}
fn missing() -> ApiError {
    ApiError::NotFound("karaoke unavailable".into())
}
type WorkResult<T> = Result<T, Reason>;
fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| {
            let hex = b"0123456789abcdef";
            [
                char::from(hex[usize::from(byte >> 4)]),
                char::from(hex[usize::from(byte & 15)]),
            ]
        })
        .collect()
}
pub(crate) fn original_digest(mut file: std::fs::File) -> Result<String, ApiError> {
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::fs::MetadataExt;
    const LIMIT: u64 = 2 * 1024 * 1024 * 1024;
    // Preserve the supplied-import encoded-source limit without allocating a
    // multi-gigabyte snapshot. Preparation separately caps immutable copies at
    // 128 MiB; this is only a bounded streaming validation of an existing import.
    let before = file.metadata().map_err(|_| missing())?;
    let stamp = |m: &std::fs::Metadata| {
        (
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    };
    if !before.is_file() || before.len() == 0 || before.len() > LIMIT {
        return Err(missing());
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| missing())?;
    let mut buffer = vec![0; 65536].into_boxed_slice();
    let mut total = 0u64;
    let mut hash = Sha256::new();
    loop {
        let size = file.read(&mut buffer).map_err(|_| missing())?;
        if size == 0 {
            break;
        }
        total += size as u64;
        if total > LIMIT {
            return Err(missing());
        }
        hash.update(&buffer[..size]);
    }
    if before.len() != total || stamp(&before) != stamp(&file.metadata().map_err(|_| missing())?) {
        return Err(missing());
    }
    Ok(hex(&hash.finalize()))
}

impl Service {
    /// Load the model once before accepting work; never mutate global env.
    ///
    /// # Errors
    /// Fails closed on invalid paths, model, thread settings or owned-cache recovery.
    pub async fn start(config: Config) -> Result<Self, std::io::Error> {
        if !(1..=4).contains(&config.cpu_threads)
            || !(1..=600).contains(&config.deadline_seconds)
            || config.cache_bytes < 2 * 1024 * 1024
            || config.cache_bytes > 2 * 1024 * 1024 * 1024
            || !config.model_archive.is_absolute()
            || !config.timing_root.is_absolute()
            || !config.cache_root.is_absolute()
            || config.model_archive.starts_with(&config.cache_root)
            || config.timing_root.starts_with(&config.cache_root)
            || config.cache_root.starts_with(&config.timing_root)
        {
            return Err(std::io::Error::other("invalid karaoke configuration"));
        }
        tokio::task::spawn_blocking(move || {
            let store = Arc::new(Store::open(&config.cache_root)?);
            storage::private_directory(&config.timing_root, false)?;
            let archive = storage::trusted_bytes(&config.model_archive, 128 * 1024 * 1024)?;
            let engine = Engine::load_official_archive(
                &archive,
                bluefin_karaoke_engine::Config {
                    cpu_threads: config.cpu_threads,
                    max_duration_seconds: 300,
                    task_timeout: Duration::from_secs(config.deadline_seconds),
                },
                &Cancellation::default(),
            )
            .map_err(std::io::Error::other)?;
            let jobs = Self::recover_requests(&store)?;
            Ok(Self(Arc::new(Shared {
                budget: Arc::new(Semaphore::new(config.cache_bytes as usize)),
                config,
                store,
                engine: Mutex::new(engine),
                table: Mutex::new(Table {
                    stopped: false,
                    jobs,
                    caches: HashMap::new(),
                    tasks: Vec::new(),
                }),
                queue: Arc::new(Semaphore::new(QUEUE_LIMIT)),
                heavy: Arc::new(Semaphore::new(1)),
                shutdown: tokio::sync::Mutex::new(()),
            })))
        })
        .await
        .map_err(std::io::Error::other)?
    }

    /// Default is disabled and performs no model/cache access. This setting is
    /// process-start, not a remotely writable named server configuration.
    ///
    /// # Errors
    /// A configured but invalid feature fails server startup closed.
    pub async fn from_environment() -> Result<Option<Self>, std::io::Error> {
        let Some(path) = std::env::var_os("BLUEFIN_KARAOKE_CONFIG") else {
            return Ok(None);
        };
        let bytes = storage::trusted_bytes(&PathBuf::from(path), 16384)?;
        let config = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        Self::start(config).await.map(Some)
    }

    fn prune(table: &mut Table) {
        table.jobs.retain(|_, record| {
            record.status.state.active() || record.created.elapsed() < HISTORY_TTL
        });
        table.tasks.retain(|task| !task.is_finished());
    }
    fn unix_ms() -> std::io::Result<u64> {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)?
                .as_millis(),
        )
        .map_err(std::io::Error::other)
    }
    fn recover_requests(store: &Store) -> std::io::Result<HashMap<Key, Record>> {
        let Some(raw) = store.journal()? else {
            return Ok(HashMap::new());
        };
        let saved: Journal = serde_json::from_slice(&raw).map_err(std::io::Error::other)?;
        if saved.version != 1 || saved.records.len() > HISTORY_LIMIT {
            return Err(std::io::Error::other("invalid preparation journal"));
        }
        let now = Self::unix_ms()?;
        let mut jobs = HashMap::new();
        for saved in saved.records {
            let age = Duration::from_millis(
                now.checked_sub(saved.created_unix_ms)
                    .ok_or_else(|| std::io::Error::other("preparation journal clock changed"))?,
            );
            if age >= HISTORY_TTL {
                continue;
            }
            if saved.key.item.is_nil()
                || saved.key.request.is_nil()
                || saved
                    .selection
                    .as_ref()
                    .is_some_and(|id| id.len() > 64 || Uuid::parse_str(id).is_err())
            {
                return Err(std::io::Error::other("invalid preparation journal scope"));
            }
            let status = Status {
                item_id: saved.key.item,
                job_id: saved.key.request,
                media_source_id: saved.selection.clone(),
                state: State::Cancelled,
                retry_after_milliseconds: 1000,
                reason: None,
            };
            jobs.insert(
                saved.key,
                Record {
                    status,
                    selection: saved.selection,
                    control: Arc::new(Control::default()),
                    created: Instant::now()
                        .checked_sub(age)
                        .ok_or_else(|| std::io::Error::other("invalid preparation journal age"))?,
                },
            );
        }
        Ok(jobs)
    }
    fn persist(&self, table: &Table) -> Result<(), ApiError> {
        let now = Self::unix_ms().map_err(|_| failed())?;
        let records = table
            .jobs
            .iter()
            .map(|(key, record)| StoppedRequest {
                key: *key,
                selection: record.selection.clone(),
                created_unix_ms: now.saturating_sub(record.created.elapsed().as_millis() as u64),
            })
            .collect();
        let bytes = serde_json::to_vec(&Journal {
            version: 1,
            records,
        })
        .map_err(|_| failed())?;
        self.0.store.save_journal(&bytes).map_err(|_| {
            self.0.store.poison();
            failed()
        })
    }

    pub(crate) fn submit(
        &self,
        state: AppState,
        session: Session,
        item: Uuid,
        request: Uuid,
        selection: Option<String>,
    ) -> Result<Status, ApiError> {
        let key = Key {
            owner: session.owner,
            item,
            request,
        };
        let mut table = self.0.table.lock().map_err(|_| failed())?;
        Self::prune(&mut table);
        if let Some(record) = table.jobs.get(&key) {
            // A DELETE tombstone always wins, including a delayed POST with a
            // source selection the DELETE could not yet have known.
            if record.status.state != State::Cancelled && record.selection != selection {
                return Err(ApiError::Conflict(
                    "preparation identifier already used".into(),
                ));
            }
            return Ok(record.status.clone());
        }
        if table.stopped || !self.0.store.healthy() || table.jobs.len() >= HISTORY_LIMIT {
            return Err(failed());
        }
        let permit = Arc::clone(&self.0.queue).try_acquire_owned();
        let control = Arc::new(Control::default());
        let status = Status {
            item_id: item,
            job_id: request,
            media_source_id: selection.clone(),
            state: if permit.is_ok() {
                State::Queued
            } else {
                State::Unavailable
            },
            retry_after_milliseconds: 1000,
            reason: if permit.is_ok() {
                None
            } else {
                Some(Reason::Busy)
            },
        };
        table.jobs.insert(
            key,
            Record {
                status: status.clone(),
                selection: selection.clone(),
                control: Arc::clone(&control),
                created: Instant::now(),
            },
        );
        if let Err(error) = self.persist(&table) {
            if let Some(record) = table.jobs.get_mut(&key) {
                record.status.state = State::Cancelled;
                record.control.stop();
            }
            return Err(error);
        }
        let Ok(permit) = permit else {
            return Ok(status);
        };
        let service = self.clone();
        table.tasks.push(tokio::spawn(async move {
            service
                .execute(state, session, key, selection, control, permit)
                .await;
        }));
        Ok(status)
    }

    pub(crate) fn status(
        &self,
        owner: [u8; 32],
        item: Uuid,
        request: Uuid,
        cancel: bool,
    ) -> Result<Status, ApiError> {
        let key = Key {
            owner,
            item,
            request,
        };
        let mut table = self.0.table.lock().map_err(|_| failed())?;
        Self::prune(&mut table);
        if let Some(record) = table.jobs.get_mut(&key) {
            if cancel && record.status.state.active() {
                record.status.state = State::Cancelling;
                record.control.stop();
            }
            let status = record.status.clone();
            if cancel {
                self.persist(&table)?;
            }
            return Ok(status);
        }
        if !cancel {
            return Err(missing());
        }
        if table.jobs.len() >= HISTORY_LIMIT {
            return Err(failed());
        }
        let status = Status {
            item_id: item,
            job_id: request,
            media_source_id: None,
            state: State::Cancelled,
            retry_after_milliseconds: 1000,
            reason: None,
        };
        table.jobs.insert(
            key,
            Record {
                status: status.clone(),
                selection: None,
                control: Arc::new(Control::default()),
                created: Instant::now(),
            },
        );
        self.persist(&table)?;
        Ok(status)
    }

    /// Stop enqueueing, cancel all queued/active work, and await real blocking
    /// execution, descriptor cleanup and resource release before closing Core/DB.
    pub async fn shutdown(&self) {
        let _shutdown = self.0.shutdown.lock().await;
        let tasks = {
            let mut table = self
                .0
                .table
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            table.stopped = true;
            for record in table.jobs.values_mut().filter(|r| r.status.state.active()) {
                record.status.state = State::Cancelling;
                record.control.stop();
            }
            let _ = self.persist(&table);
            std::mem::take(&mut table.tasks)
        };
        for task in tasks {
            let _ = task.await;
        }
        let caches = {
            let mut table = self
                .0
                .table
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut table.caches)
        };
        let _ = tokio::task::spawn_blocking(move || drop(caches)).await;
    }

    /// Qualification-only observation: (available queue slots, heavy slots).
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn testing_resource_counts(&self) -> (usize, usize) {
        (
            self.0.queue.available_permits(),
            self.0.heavy.available_permits(),
        )
    }

    // Lifecycle ordering keeps cancellation, real join and terminal publication
    // visible together; no detached continuation can mark a stopped job ready.
    #[allow(clippy::too_many_lines)]
    async fn execute(
        &self,
        state: AppState,
        session: Session,
        key: Key,
        selection: Option<String>,
        control: Arc<Control>,
        queue: OwnedSemaphorePermit,
    ) {
        let start = Instant::now();
        // Each bounded job owns this async future and at most one blocking join.
        // Checks below are revocation checks, never client status-poll workers.
        let operation = async {
            control
                .cancel
                .check()
                .map_err(|_| Reason::PreparationFailed)?;
            let source = Source::capture(&state, &session, key.item, selection.as_deref())
                .await
                .map_err(|_| Reason::UnsupportedSource)?;
            {
                let mut table = self.0.table.lock().map_err(|_| Reason::PreparationFailed)?;
                if let Some(record) = table.jobs.get_mut(&key) {
                    record.status.media_source_id = Some(source.selected.clone());
                }
            }
            let heavy = loop {
                tokio::select! {
                    permit = Arc::clone(&self.0.heavy).acquire_owned() => break permit.map_err(|_| Reason::Busy)?,
                    () = control.changed.notified() => return Err(Reason::PreparationFailed),
                    () = tokio::time::sleep(Duration::from_millis(250)) => {
                        if source.current(&state,&session).await.is_err()
                            || start.elapsed() > Duration::from_secs(self.0.config.deadline_seconds) {
                            control.stop(); return Err(Reason::UnsupportedSource);
                        }
                    }
                }
            };
            control
                .cancel
                .check()
                .map_err(|_| Reason::PreparationFailed)?;
            {
                let mut table = self.0.table.lock().map_err(|_| Reason::PreparationFailed)?;
                let record = table.jobs.get_mut(&key).ok_or(Reason::PreparationFailed)?;
                if record.status.state == State::Cancelling {
                    return Err(Reason::PreparationFailed);
                }
                record.status.state = State::Preparing;
            }
            let shared = Arc::clone(&self.0);
            let cancel = control.cancel.clone();
            let worker_state = state.clone();
            let worker_session = session.clone();
            let handle = tokio::runtime::Handle::current();
            let worker_source = source.clone();
            let mut work = tokio::task::spawn_blocking(move || {
                let _queue = queue;
                let _heavy = heavy;
                shared.prepare(
                    &handle,
                    &worker_state,
                    &worker_session,
                    &worker_source,
                    &cancel,
                )
            });
            loop {
                tokio::select! {
                    result = &mut work => break if let Ok(value)=result {value}
                        else {self.0.store.poison();Err(Reason::PreparationFailed)},
                    () = control.changed.notified() => { control.cancel.cancel(); },
                    () = tokio::time::sleep(Duration::from_millis(250)) => {
                        if start.elapsed() > Duration::from_secs(self.0.config.deadline_seconds)
                            || source.current(&state, &session).await.is_err() {
                            control.stop();
                            if let Ok(mut table) = self.0.table.lock()
                                && let Some(record) = table.jobs.get_mut(&key) {
                                record.status.state = State::Cancelling;
                            }
                        }
                    }
                }
            }
        };
        let result = operation.await;
        // Only this join owner writes terminal state. A cancelled task cannot
        // become ready after stop, even if inference finished in the same tick.
        if let Ok(mut table) = self.0.table.lock() {
            if let Ok(entry) = &result {
                table.caches.insert(entry.key.clone(), Arc::clone(entry));
            }
            if let Some(record) = table.jobs.get_mut(&key) {
                record.created = Instant::now();
                if control.cancel.check().is_err() || record.status.state == State::Cancelling {
                    record.status.state = State::Cancelled;
                    record.status.reason = None;
                } else {
                    match result {
                        Ok(_) => {
                            record.status.state = State::Ready;
                            record.status.reason = None;
                        }
                        Err(reason) => {
                            record.status.state = if matches!(reason, Reason::PreparationFailed) {
                                State::Failed
                            } else {
                                State::Unavailable
                            };
                            record.status.reason = Some(reason);
                        }
                    }
                }
            }
            let _ = self.persist(&table);
        }
    }

    pub(crate) async fn prepared(
        &self,
        state: &AppState,
        session: &Session,
        source: &Source,
    ) -> Result<Admitted, ApiError> {
        let evidence = source.current(state, session).await?;
        let store = Arc::clone(&self.0.store);
        let config = self.0.config.clone();
        let source = source.clone();
        let service = self.clone();
        let runtime = tokio::runtime::Handle::current();
        let worker_state = state.clone();
        let worker_session = session.clone();
        let permit = Arc::clone(&self.0.queue)
            .try_acquire_owned()
            .map_err(|_| failed())?;
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let encoded = decode::snapshot(evidence.input, &Cancellation::default())
                .map_err(|_| missing())?;
            let source_hash = digest(&encoded);
            let timing = storage::Timing::load(&config.timing_root, &source, &source_hash)
                .map_err(|_| missing())?;
            let key = source.cache_key(worker_session.user, &source_hash, &timing.hash);
            let entry = service
                .0
                .table
                .lock()
                .map_err(|_| failed())?
                .caches
                .get(&key)
                .cloned()
                .ok_or_else(missing)?;
            if entry.created.elapsed() >= CACHE_TTL {
                return Err(missing());
            }
            let admitted = store
                .read(&entry, &source, &source_hash)
                .map_err(|_| missing())?;
            let current = runtime.block_on(source.current(&worker_state, &worker_session))?;
            if original_digest(current.input)? != source_hash {
                return Err(missing());
            }
            Ok(admitted)
        })
        .await
        .map_err(|_| failed())??;
        // Serving never falls back to stale source data or an expired policy.
        session.authorize(state).await?;
        Ok(result)
    }
}

impl Shared {
    // This is the single blocking ownership scope through model and cleanup.
    #[allow(clippy::too_many_lines)]
    fn prepare(
        &self,
        runtime: &tokio::runtime::Handle,
        state: &AppState,
        session: &Session,
        source: &Source,
        cancel: &Cancellation,
    ) -> WorkResult<Arc<CacheEntry>> {
        if !self.store.healthy() {
            return Err(Reason::PreparationFailed);
        }
        cancel.check().map_err(|_| Reason::PreparationFailed)?;
        let expired = {
            let mut table = self.table.lock().map_err(|_| Reason::PreparationFailed)?;
            #[allow(clippy::needless_collect)] // keys must be copied before mutating the map
            let keys: Vec<_> = table
                .caches
                .iter()
                .filter(|(_, entry)| entry.created.elapsed() >= CACHE_TTL)
                .map(|(key, _)| key.clone())
                .collect(); // collect before mutating the same map
            keys.into_iter()
                .filter_map(|key| table.caches.remove(&key))
                .collect::<Vec<_>>()
        };
        drop(expired); // filesystem cleanup belongs to this admitted blocking worker
        let evidence = runtime
            .block_on(source.current(state, session))
            .map_err(|_| Reason::UnsupportedSource)?;
        let duration = evidence.duration_ticks as f64 / 10_000_000.;
        if duration > 300. {
            return Err(Reason::DurationUnsupported);
        }
        let encoded = decode::snapshot(evidence.input, cancel)?;
        let original_hash = digest(&encoded);
        let timing = storage::Timing::load(&self.config.timing_root, source, &original_hash)?;
        let key = source.cache_key(session.user, &original_hash, &timing.hash);
        if let Some(entry) = self
            .table
            .lock()
            .map_err(|_| Reason::PreparationFailed)?
            .caches
            .get(&key)
            && entry.created.elapsed() < CACHE_TTL
        {
            return Ok(Arc::clone(entry));
        }
        let pcm = decode::canonical(encoded, duration, cancel)?;
        let format = bluefin_karaoke::pcm(&pcm).map_err(|_| Reason::UnsupportedSource)?;
        let actual_duration = format.2 as f64 / 44100.;
        bluefin_karaoke::validate_timing(
            Some(timing.kind),
            timing.timeline.as_ref(),
            actual_duration,
        )
        .map_err(|_| Reason::TimingRequired)?;
        let reservation =
            u32::try_from(format.2 * 16 + 2 * 1024 * 1024).map_err(|_| Reason::Busy)?;
        let budget = Arc::clone(&self.budget)
            .try_acquire_many_owned(reservation)
            .map_err(|_| Reason::Busy)?;
        cancel.check().map_err(|_| Reason::PreparationFailed)?;
        let staging_root = StagingRoot::from_opened_directory(
            self.store
                .staging
                .try_clone()
                .map_err(|_| Reason::PreparationFailed)?,
        )
        .map_err(|_| Reason::PreparationFailed)?;
        let processed = self
            .engine
            .lock()
            .map_err(|_| Reason::PreparationFailed)?
            .process(
                Input {
                    pcm_wav: &pcm,
                    expected_pcm_sha256: Sha256::digest(&pcm).into(),
                    attachment: (source.clone(), session.clone()),
                },
                &staging_root,
                cancel,
            )
            .map_err(|error| {
                if matches!(error, bluefin_karaoke_engine::Error::Cleanup { .. }) {
                    self.store.poison();
                }
                Reason::PreparationFailed
            })?;
        let result = (|| {
            let accompaniment = storage::read_descriptor(
                processed
                    .files
                    .open_stem(bluefin_karaoke_engine::Stem::Accompaniment)
                    .map_err(|_| Reason::PreparationFailed)?,
                bluefin_karaoke::MAX_STEM,
            )
            .map_err(|_| Reason::PreparationFailed)?;
            let vocal = storage::read_descriptor(
                processed
                    .files
                    .open_stem(bluefin_karaoke_engine::Stem::LeadVocal)
                    .map_err(|_| Reason::PreparationFailed)?,
                bluefin_karaoke::MAX_STEM,
            )
            .map_err(|_| Reason::PreparationFailed)?;
            let p = &processed.provenance;
            let raw = serde_json::to_vec(&Import {
                item_id: source.item,
                media_source_id: Some(source.selected.clone()),
                source_sha256: original_hash.clone(),
                duration_seconds: actual_duration,
                rights_reference: source.admission_reference(session.user, &original_hash),
                timing_kind: Some(timing.kind),
                timeline: timing.timeline.clone(),
                accompaniment: Stem {
                    sha256: p.accompaniment.sha256.clone(),
                    bytes: p.accompaniment.bytes,
                },
                lead_vocal: Stem {
                    sha256: p.lead_vocal.sha256.clone(),
                    bytes: p.lead_vocal.bytes,
                },
                processing: Processing::ModelSeparated {
                    model_archive_sha256: p.model_archive_sha256.clone(),
                    engine_version: p.engine_version.clone(),
                    input_pcm_sha256: p.input_pcm_sha256.clone(),
                },
            })
            .map_err(|_| Reason::PreparationFailed)?;
            let admitted = bluefin_karaoke::admit_bytes(
                &raw,
                source.item,
                Some(&source.selected),
                &original_hash,
                actual_duration,
                accompaniment,
                vocal,
            )
            .map_err(|_| Reason::PreparationFailed)?;
            let current = runtime
                .block_on(source.current(state, session))
                .map_err(|_| Reason::UnsupportedSource)?;
            if digest(&decode::snapshot(current.input, cancel)?) != original_hash
                || storage::Timing::load(&self.config.timing_root, source, &original_hash)?.hash
                    != timing.hash
            {
                return Err(Reason::UnsupportedSource);
            }
            cancel.check().map_err(|_| Reason::PreparationFailed)?;
            let receipt = serde_json::to_vec(&serde_json::json!({
                "itemId": source.item, "mediaSourceId": source.selected, "generation": source.generation,
                "sourceSha256": original_hash, "inputPcmSha256": p.input_pcm_sha256,
                "timingKind": timing.kind, "timingSha256": timing.hash,
                "rightsReference": source.admission_reference(session.user, &original_hash),
                "suppliedTimingRightsReference": timing.rights,
                "decoderRuntime": "symphonia-0.5.5-rubato-0.16.2",
                "ownerScopeSha256": hex(&session.owner),
                "engine": p,
            })).map_err(|_| Reason::PreparationFailed)?;
            self.store
                .publish(key, &raw, &receipt, &admitted, budget, cancel, || {
                    let current = runtime
                        .block_on(source.current(state, session))
                        .map_err(|_| std::io::Error::other("source authority changed"))?;
                    if digest(
                        &decode::snapshot(current.input, cancel)
                            .map_err(|_| std::io::Error::other("source unavailable"))?,
                    ) != original_hash
                        || storage::Timing::load(&self.config.timing_root, source, &original_hash)
                            .map_err(|_| std::io::Error::other("timing authority changed"))?
                            .hash
                            != timing.hash
                    {
                        return Err(std::io::Error::other("source or timing changed"));
                    }
                    Ok(())
                })
                .map_err(|_| Reason::PreparationFailed)
        })();
        // Explicitly observe engine cleanup, holding model/source/resource owners
        // until cleanup completes even when HTTP cancellation already returned.
        if processed.files.remove().is_err() {
            self.store.poison();
            return Err(Reason::PreparationFailed);
        }
        result
    }
}
