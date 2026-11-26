//! One playback lifecycle for desktop and WASM. Mutation responses are drained
//! before scoped cleanup; browsing cancellation never kills playback reporting.
use crate::{
    playback::{self, Plan, Platform, QueueRun, Selection},
    protocol::{self, Item, Result, Session},
};
use bluefin_contract::QueueState;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Preparation {
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
}
#[derive(Clone, Default)]
pub struct Status {
    pub plan: Option<Plan>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub busy: bool,
    pub finished: bool,
    pub message: String,
    pub cleanup_acknowledged: bool,
    pub revoked: bool,
}
pub enum Command {
    Play(Box<Item>, Selection, Option<(QueueState, usize)>),
    Pause(bool),
    Seek(f64),
    Volume(f64),
    Stop,
    Logout,
}
pub struct Service {
    tx: mpsc::Sender<Command>,
    pub status: Arc<Mutex<Status>>,
    #[cfg(not(target_arch = "wasm32"))]
    kill: Arc<Mutex<Option<Arc<crate::player::Player>>>>,
    #[cfg(not(target_arch = "wasm32"))]
    preparation: Arc<Mutex<Preparation>>,
}
impl Service {
    pub fn start(
        session: Session,
        fixture: bool,
        peer: Option<(String, std::net::SocketAddr)>,
    ) -> Self {
        Self::make(session, fixture, peer, false)
    }
    /// A canceled sign-in may still mint a token. Retire only that session,
    /// without touching the newer account's player or worker namespace.
    pub fn revoke_only(session: Session) -> Self {
        Self::make(session, false, None, true)
    }
    fn make(
        session: Session,
        fixture: bool,
        peer: Option<(String, std::net::SocketAddr)>,
        revoke_only: bool,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        #[cfg(not(target_arch = "wasm32"))]
        let kill = Arc::new(Mutex::new(None));
        #[cfg(not(target_arch = "wasm32"))]
        let preparation = Arc::new(Mutex::new(Preparation::default()));
        let task = run(
            session,
            fixture,
            peer,
            revoke_only,
            rx,
            status.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            NativeOutputs {
                kill: kill.clone(),
                preparation: preparation.clone(),
            },
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
            RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
                .spawn(task);
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(task);
        Self {
            tx,
            status,
            #[cfg(not(target_arch = "wasm32"))]
            kill,
            #[cfg(not(target_arch = "wasm32"))]
            preparation,
        }
    }
    pub fn send(&self, command: Command) {
        if matches!(
            &command,
            Command::Play(..) | Command::Stop | Command::Logout
        ) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                // This gate only protects local publication/cancellation. It
                // never spans a synchronous decoder or network operation.
                let mut preparing = self.preparation.lock().unwrap();
                preparing.generation = preparing.generation.wrapping_add(1);
                if let Some(cancel) = &preparing.cancel {
                    cancel.store(true, Ordering::Release);
                }
                let player = self.kill.lock().unwrap().clone();
                if let Some(player) = player {
                    player.stop();
                }
            }
            #[cfg(target_arch = "wasm32")]
            web::interrupt_media();
        }
        let _ = self.tx.send(command);
    }
    pub fn snapshot(&self) -> Status {
        self.status.lock().unwrap().clone()
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if !self.snapshot().finished {
            self.send(Command::Logout);
        }
    }
}
#[derive(Default)]
struct Sample {
    position: f64,
    duration: f64,
    paused: bool,
    loaded: bool,
    eof: bool,
    failed: bool,
    error: Option<String>,
}
struct Backend {
    #[cfg(not(target_arch = "wasm32"))]
    player: Arc<crate::player::Player>,
}
struct StartupFailure {
    message: String,
    retired: bool,
}
#[cfg(target_arch = "wasm32")]
impl StartupFailure {
    fn unconfirmed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retired: false,
        }
    }
}
impl Backend {
    async fn start(
        session: &Session,
        plan: &Plan,
        fixture: bool,
        peer: Option<(String, std::net::SocketAddr)>,
        #[cfg(not(target_arch = "wasm32"))] cancellation: Arc<AtomicBool>,
    ) -> std::result::Result<Self, StartupFailure> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = fixture;
            Ok(Self {
                player: Arc::new(
                    crate::player::Player::start_with_cancellation(
                        session.clone(),
                        plan.clone(),
                        peer,
                        cancellation,
                    )
                    .map_err(|error| StartupFailure {
                        message: error.message,
                        retired: error.retired,
                    })?,
                ),
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = peer;
            let mesh = crate::mesh::is_mesh(&session.token);
            let authorization = if mesh { String::new() } else {
                protocol::authorization(&session.token).map_err(StartupFailure::unconfirmed)?
            };
            web::start_media(
                session.base.as_str(),
                &authorization,
                &serde_json::to_string(plan)
                    .map_err(|_| StartupFailure::unconfirmed("Invalid playback plan"))?,
                fixture,
                mesh.then(|| session.token.clone()),
            )
            .await
            .map_err(|_| {
                StartupFailure::unconfirmed(
                    "Browser media preparation failed; cleanup must be checked",
                )
            })?;
            Ok(Self {})
        }
    }
    fn sample(&self) -> Sample {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let s = self.player.snapshot();
            Sample {
                position: s.position,
                duration: s.duration,
                paused: s.paused,
                loaded: s.loaded,
                eof: s.eof,
                failed: s.failed,
                error: s.error,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let value: serde_json::Value =
                serde_json::from_str(&web::sample_media()).unwrap_or_default();
            Sample {
                position: value["position"].as_f64().unwrap_or(0.0),
                duration: value["duration"].as_f64().unwrap_or(0.0),
                paused: value["paused"].as_bool().unwrap_or(true),
                loaded: value["loaded"].as_bool().unwrap_or(false),
                eof: value["eof"].as_bool().unwrap_or(false),
                failed: value["failed"].as_bool().unwrap_or(false),
                error: value["error"].as_str().map(str::to_owned),
            }
        }
    }
    fn control(&self, kind: &str, value: f64) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use crate::mpv::Command;
            self.player.command(match kind {
                "pause" => Command::Pause(value != 0.0),
                "seek" => Command::Seek(value),
                "volume" => Command::Volume(value),
                _ => return Err("Invalid player command".into()),
            });
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        web::control_media(kind, value).map_err(|_| "Browser media command failed".into())
    }
    async fn stop(self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.player.stop();
            for _ in 0..120 {
                if self.player.stopped() {
                    return self.player.cleanup_receipt();
                }
                delay().await;
            }
            Err("Native decoder/bridge retirement not acknowledged".into())
        }
        #[cfg(target_arch = "wasm32")]
        {
            web::stop_media()
                .await
                .map(|_| ())
                .map_err(|_| "Media-worker cleanup was not acknowledged".into())
        }
    }
}
struct Active {
    plan: Plan,
    backend: Backend,
    started: bool,
    attempted_start: bool,
    position: f64,
    queue: Option<QueueRun>,
    elapsed: u32,
}
async fn close(session: &Session, active: Active) -> Result<()> {
    let stopped = active.backend.stop().await;
    // An attempted start can reach the server despite a lost response. Always
    // send a scoped stop report, then release resources, retaining both failures.
    let report = playback::report(session, &active.plan, "stopped", active.position, true).await;
    let release = playback::release(session, &active.plan).await;
    stopped.and(report).and(release)
}
#[cfg(not(target_arch = "wasm32"))]
struct NativeOutputs {
    kill: Arc<Mutex<Option<Arc<crate::player::Player>>>>,
    preparation: Arc<Mutex<Preparation>>,
}
async fn run(
    session: Session,
    fixture: bool,
    peer: Option<(String, std::net::SocketAddr)>,
    revoke_only: bool,
    rx: mpsc::Receiver<Command>,
    state: Arc<Mutex<Status>>,
    #[cfg(not(target_arch = "wasm32"))] outputs: NativeOutputs,
) {
    #[cfg(not(target_arch = "wasm32"))]
    let NativeOutputs { kill, preparation } = outputs;
    let mut active: Option<Active> = None;
    let mut queued = None;
    let mut logout = revoke_only;
    loop {
        let mut stop = false;
        for command in rx.try_iter() {
            match command {
                Command::Play(item, selection, queue) => {
                    queued = Some((
                        *item,
                        selection,
                        queue.and_then(|(q, i)| {
                            QueueRun::new(q, i, uuid::Uuid::new_v4().as_u128() as u64)
                        }),
                    ));
                    stop = true;
                }
                Command::Stop => {
                    queued = None;
                    stop = true;
                }
                Command::Logout => {
                    queued = None;
                    stop = true;
                    logout = true;
                }
                Command::Pause(paused) => {
                    if let Some(a) = &mut active {
                        if let Err(e) = a.backend.control("pause", if paused { 1.0 } else { 0.0 }) {
                            state.lock().unwrap().message = e;
                        }
                        a.elapsed = 50;
                    }
                }
                Command::Seek(position) => {
                    if let Some(a) = &mut active {
                        if a.plan.live {
                            state.lock().unwrap().message =
                                "Live stream has no seekable archive".into();
                        } else if let Err(e) = a
                            .backend
                            .control("seek", (position - a.plan.offset).max(0.0))
                        {
                            state.lock().unwrap().message = e;
                        }
                        a.elapsed = 50;
                    }
                }
                Command::Volume(value) => {
                    if let Some(a) = &active {
                        let _ = a.backend.control("volume", value);
                    }
                }
            }
        }
        if stop && let Some(a) = active.take() {
            state.lock().unwrap().busy = true;
            let result = close(&session, a).await;
            let mut s = state.lock().unwrap();
            s.plan = None;
            s.cleanup_acknowledged = result.is_ok();
            s.busy = false;
            s.message = match result {
                Ok(()) => "Playback cleanup acknowledged".into(),
                Err(e) => e,
            };
            #[cfg(not(target_arch = "wasm32"))]
            {
                *kill.lock().unwrap() = None;
            }
        }
        if logout {
            #[cfg(target_arch = "wasm32")]
            if !revoke_only && active.is_none() && web::stop_media().await.is_err() {
                state.lock().unwrap().message = "Media-worker cleanup was not acknowledged".into();
                state.lock().unwrap().cleanup_acknowledged = false;
            }
            let revoked = protocol::request(
                &session.base,
                &session.token,
                "POST",
                "Sessions/Logout",
                &[],
                None,
            )
            .await
            .is_ok();
            let mut s = state.lock().unwrap();
            s.revoked = revoked;
            s.busy = false;
            s.finished = true;
            if !revoked {
                s.message
                    .push_str("; server session revocation was not acknowledged");
            } else {
                s.message
                    .push_str("; server session revocation acknowledged");
            }
            return;
        }
        if active.is_none()
            && let Some((item, selection, queue)) = queued.take()
        {
            #[cfg(not(target_arch = "wasm32"))]
            let generation = preparation.lock().unwrap().generation;
            {
                let mut s = state.lock().unwrap();
                s.busy = true;
                s.cleanup_acknowledged = false;
                s.message = "Negotiating PlaybackInfo".into();
            }
            #[cfg(not(target_arch = "wasm32"))]
            let platform = Platform::Native;
            #[cfg(target_arch = "wasm32")]
            let platform = Platform::Browser;
            match playback::negotiate(&session, item, selection, platform).await {
                Ok(plan) => {
                    #[cfg(not(target_arch = "wasm32"))]
                    let cancellation = {
                        let mut pending = preparation.lock().unwrap();
                        let cancel = Arc::new(AtomicBool::new(pending.generation != generation));
                        pending.cancel = Some(cancel.clone());
                        cancel
                    };
                    // Stop/account replacement arriving during preparation cannot
                    // publish a backend or sequence another item.
                    let commands: Vec<_> = rx.try_iter().collect();
                    let mut canceled = false;
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        canceled |= cancellation.load(Ordering::Acquire);
                    }
                    for command in commands {
                        match command {
                            Command::Logout => {
                                logout = true;
                                canceled = true;
                                queued = None;
                            }
                            Command::Stop => {
                                canceled = true;
                                queued = None;
                            }
                            Command::Play(i, s, q) => {
                                canceled = true;
                                queued = Some((
                                    *i,
                                    s,
                                    q.and_then(|(q, i)| {
                                        QueueRun::new(q, i, uuid::Uuid::new_v4().as_u128() as u64)
                                    }),
                                ));
                            }
                            _ => {}
                        }
                    }
                    if canceled {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            preparation.lock().unwrap().cancel = None;
                        }
                        let released = playback::release(&session, &plan).await;
                        let mut s = state.lock().unwrap();
                        s.busy = false;
                        s.cleanup_acknowledged = released.is_ok();
                        s.message = if released.is_ok() {
                            "Canceled preparation cleanup acknowledged".into()
                        } else {
                            "Canceled preparation cleanup was not acknowledged".into()
                        };
                    } else {
                        match Backend::start(
                            &session,
                            &plan,
                            fixture,
                            peer.clone(),
                            #[cfg(not(target_arch = "wasm32"))]
                            cancellation.clone(),
                        )
                        .await
                        {
                            Ok(backend) => {
                                #[cfg(not(target_arch = "wasm32"))]
                                let admitted = {
                                    let mut pending = preparation.lock().unwrap();
                                    pending.cancel = None;
                                    if pending.generation == generation
                                        && !cancellation.load(Ordering::Acquire)
                                    {
                                        *kill.lock().unwrap() = Some(backend.player.clone());
                                        let mut status = state.lock().unwrap();
                                        status.plan = Some(plan.clone());
                                        status.message = "Prepared playback".into();
                                        true
                                    } else {
                                        false
                                    }
                                };
                                #[cfg(target_arch = "wasm32")]
                                let admitted = {
                                    state.lock().unwrap().plan = Some(plan.clone());
                                    state.lock().unwrap().message = "Prepared playback".into();
                                    true
                                };
                                if admitted {
                                    active = Some(Active {
                                        plan,
                                        backend,
                                        started: false,
                                        attempted_start: false,
                                        position: selection.position,
                                        queue,
                                        elapsed: 0,
                                    });
                                } else {
                                    let stopped = backend.stop().await;
                                    let released = playback::release(&session, &plan).await;
                                    let mut status = state.lock().unwrap();
                                    status.cleanup_acknowledged = stopped.and(released).is_ok();
                                    status.message = if status.cleanup_acknowledged {
                                        "Canceled preparation cleanup acknowledged".into()
                                    } else {
                                        "Canceled preparation cleanup was not acknowledged".into()
                                    };
                                }
                            }
                            Err(e) => {
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    preparation.lock().unwrap().cancel = None;
                                }
                                let released = playback::release(&session, &plan).await;
                                let mut s = state.lock().unwrap();
                                s.cleanup_acknowledged = e.retired && released.is_ok();
                                s.message = if released.is_ok() {
                                    e.message
                                } else {
                                    format!("{}; resource cleanup was not acknowledged", e.message)
                                };
                            }
                        }
                    }
                }
                Err(e) => state.lock().unwrap().message = e,
            }
            state.lock().unwrap().busy = false;
        }
        if let Some(a) = &mut active {
            let sample = a.backend.sample();
            a.position = sample.position + a.plan.offset;
            a.elapsed += 1;
            {
                let mut s = state.lock().unwrap();
                s.position = a.position;
                s.duration = if sample.duration > 0.0 {
                    sample.duration + a.plan.offset
                } else {
                    0.0
                };
                s.paused = sample.paused;
            }
            // Metadata alone does not count as playing. Time must actually advance.
            if sample.loaded
                && sample.position > 0.0
                && !a.started
                && (!a.attempted_start || a.elapsed >= 50)
            {
                a.attempted_start = true;
                match playback::report(&session, &a.plan, "started", a.position, sample.paused)
                    .await
                {
                    Ok(()) => {
                        a.started = true;
                        a.elapsed = 0;
                        state.lock().unwrap().message = "Playback start acknowledged".into();
                    }
                    Err(e) => {
                        state.lock().unwrap().message = e;
                        a.elapsed = 0;
                    }
                }
            } else if a.started && a.elapsed >= 50 {
                if let Err(e) =
                    playback::report(&session, &a.plan, "progress", a.position, sample.paused).await
                {
                    state.lock().unwrap().message = e;
                }
                a.elapsed = 0;
            }
            if sample.eof || sample.failed {
                let mut a = active.take().unwrap();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    *kill.lock().unwrap() = None;
                }
                let next = if sample.eof {
                    a.queue.take().and_then(|mut q| {
                        q.advance(uuid::Uuid::new_v4().as_u128() as u64)
                            .map(|index| (q, index))
                    })
                } else {
                    None
                };
                let result = close(&session, a).await;
                let acknowledged = result.is_ok();
                {
                    let mut s = state.lock().unwrap();
                    s.plan = None;
                    s.cleanup_acknowledged = acknowledged;
                    s.message = sample.error.unwrap_or_else(|| {
                        if acknowledged {
                            "Playback ended; cleanup acknowledged".into()
                        } else {
                            "Playback ended; cleanup was not acknowledged".into()
                        }
                    });
                }
                if acknowledged && let Some((q, n)) = next {
                    let item = Item {
                        id: q.state.item_ids[n],
                        name: "Queued track".into(),
                        kind: "Audio".into(),
                        overview: String::new(),
                        start_date: None,
                        end_date: None,
                        channel_name: None,
                        channel_id: None,
                    };
                    queued = Some((item, Selection::default(), Some(q)));
                }
            }
        }
        delay().await;
    }
}
async fn delay() {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    #[cfg(target_arch = "wasm32")]
    web::delay(100).await;
}
#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(module = "/web-player.js")]
    extern "C" {
        #[wasm_bindgen(catch,js_name=startMedia)]
        pub async fn start_media(
            base: &str,
            auth: &str,
            plan: &str,
            fixture: bool,
            mesh_handle: Option<String>,
        ) -> std::result::Result<JsValue, JsValue>;
        #[wasm_bindgen(catch,js_name=stopMedia)]
        pub async fn stop_media() -> std::result::Result<JsValue, JsValue>;
        #[wasm_bindgen(js_name=sampleMedia)]
        pub fn sample_media() -> String;
        #[wasm_bindgen(catch,js_name=controlMedia)]
        pub fn control_media(kind: &str, value: f64) -> std::result::Result<(), JsValue>;
        #[wasm_bindgen(js_name=interruptMedia)]
        pub fn interrupt_media();
        pub async fn delay(ms: u32);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Service {
    /// The UI reads the decoder's bounded latest-frame mailbox directly.
    /// Reporting stays on its independent, slower lifecycle tick.
    pub fn diagnostics(&self) -> Option<crate::mpv::Snapshot> {
        let player = self.kill.lock().unwrap().clone();
        player.map(|p| p.snapshot())
    }
    pub fn frame(&self) -> Option<crate::mpv::Frame> {
        let player = self.kill.lock().unwrap().clone();
        player.and_then(|p| p.frame())
    }
}
