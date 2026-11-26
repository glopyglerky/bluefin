//! Narrow libmpv client/render ABI adapter. GL tickets lease a live core to the
//! current-context UI owner; software fallback uses one bounded latest frame.
use crate::protocol::Result;
use libloading::Library;
use std::{
    ffi::{CString, c_char, c_int, c_void},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[repr(C)]
pub(crate) struct Param {
    pub kind: c_int,
    pub data: *mut c_void,
}
#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}
type Handle = *mut c_void;
pub(crate) struct Api {
    _library: Library,
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    option: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    get: unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    free: unsafe extern "C" fn(*mut c_void),
    wait: unsafe extern "C" fn(Handle, f64) -> *const Event,
    request_log: unsafe extern "C" fn(Handle, *const c_char) -> c_int,
    destroy: unsafe extern "C" fn(Handle),
    pub render_create: unsafe extern "C" fn(*mut Handle, Handle, *mut Param) -> c_int,
    pub render_update: unsafe extern "C" fn(Handle) -> u64,
    pub render: unsafe extern "C" fn(Handle, *mut Param) -> c_int,
    pub render_free: unsafe extern "C" fn(Handle),
}
impl Api {
    fn load() -> Result<Arc<Self>> {
        let candidates = if let Ok(path) = std::env::var("BLUEFIN_MPV_LIBRARY") {
            vec![path]
        } else {
            #[cfg(target_os = "windows")]
            let names = vec!["mpv-2.dll", "libmpv-2.dll"];
            #[cfg(target_os = "macos")]
            let names = vec!["libmpv.2.dylib", "libmpv.dylib"];
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let names = vec!["libmpv.so.2", "libmpv.so"];
            names.into_iter().map(str::to_owned).collect()
        };
        for path in candidates {
            // SAFETY: load only an operator-installed compatible libmpv library.
            // Each symbol has its published client.h/render.h ABI signature.
            unsafe {
                let Ok(library) = Library::new(path) else {
                    continue;
                };
                macro_rules! sym {
                    ($name:literal) => {
                        *library
                            .get(concat!($name, "\0").as_bytes())
                            .map_err(|_| "Installed libmpv lacks the required client/render API")?
                    };
                }
                let api = Self {
                    create: sym!("mpv_create"),
                    initialize: sym!("mpv_initialize"),
                    option: sym!("mpv_set_option_string"),
                    command: sym!("mpv_command"),
                    get: sym!("mpv_get_property"),
                    free: sym!("mpv_free"),
                    wait: sym!("mpv_wait_event"),
                    request_log: sym!("mpv_request_log_messages"),
                    destroy: sym!("mpv_terminate_destroy"),
                    render_create: sym!("mpv_render_context_create"),
                    render_update: sym!("mpv_render_context_update"),
                    render: sym!("mpv_render_context_render"),
                    render_free: sym!("mpv_render_context_free"),
                    _library: library,
                };
                return Ok(Arc::new(api));
            }
        }
        Err("Embedded playback needs libmpv 0.35 or newer. Install a compatible licensed libmpv and its codec dependencies; BLUEFIN_MPV_LIBRARY can select it.".into())
    }
    fn option(&self, h: Handle, name: &str, value: &str) -> Result<()> {
        let n = CString::new(name).map_err(|_| "Invalid media option")?;
        let v = CString::new(value).map_err(|_| "Invalid media value")?;
        // SAFETY: live handle; C strings live throughout the call and mpv copies them.
        if unsafe { (self.option)(h, n.as_ptr(), v.as_ptr()) } < 0 {
            Err(format!("libmpv rejected option {name}"))
        } else {
            Ok(())
        }
    }
    fn command(&self, h: Handle, args: &[&str]) -> Result<()> {
        let strings: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s))
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| "Invalid media command")?;
        let mut pointers: Vec<_> = strings.iter().map(|s| s.as_ptr()).collect();
        pointers.push(std::ptr::null());
        // SAFETY: null-terminated argument array and live handle, on control thread.
        if unsafe { (self.command)(h, pointers.as_ptr()) } < 0 {
            Err("Embedded media command failed".into())
        } else {
            Ok(())
        }
    }
    fn number(&self, h: Handle, name: &str) -> Option<f64> {
        let n = CString::new(name).ok()?;
        let mut value = 0.0f64;
        // SAFETY: MPV_FORMAT_DOUBLE=5 writes one f64 to the provided pointer.
        (unsafe { (self.get)(h, n.as_ptr(), 5, (&mut value as *mut f64).cast()) } >= 0)
            .then_some(value)
    }
    fn text(&self, h: Handle, name: &str) -> String {
        let name = CString::new(name).unwrap();
        let mut value: *mut c_char = std::ptr::null_mut();
        // SAFETY: MPV_FORMAT_STRING writes an allocated C string. Copy it and
        // release exactly once through the same loaded mpv library's allocator.
        unsafe {
            if (self.get)(h, name.as_ptr(), 1, (&mut value as *mut *mut c_char).cast()) < 0
                || value.is_null()
            {
                return String::new();
            }
            let text = std::ffi::CStr::from_ptr(value)
                .to_string_lossy()
                .into_owned();
            (self.free)(value.cast());
            text
        }
    }
    fn flag(&self, h: Handle, name: &str) -> bool {
        let n = CString::new(name).unwrap();
        let mut value = 0i32;
        // SAFETY: MPV_FORMAT_FLAG=3 writes one int to the provided pointer.
        unsafe { (self.get)(h, n.as_ptr(), 3, (&mut value as *mut i32).cast()) >= 0 && value != 0 }
    }
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub loaded: bool,
    pub eof: bool,
    pub failed: bool,
    pub stopped: bool,
    pub error: Option<String>,
    pub audio_decoded: bool,
    pub video_decoded: bool,
    pub source_size: [usize; 2],
    pub display_size: [usize; 2],
    pub audio_rate: usize,
    pub audio_channels: usize,
    pub codec: String,
    pub hwdec: String,
    pub source_transfer: String,
    pub render_path: String,
}

/// Preserve display aspect and bound both axes and total pixel allocation (4K SDR).
pub fn render_size(width: f64, height: f64) -> Option<[usize; 2]> {
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
        return None;
    }
    let scale = (4096.0 / width).min(2160.0 / height).min(1.0);
    Some([
        (width * scale).round().max(1.0) as usize,
        (height * scale).round().max(1.0) as usize,
    ])
}

// A ticket is a lease, not a transferable render context. The decoder owns the
// core and library until the GL owner acknowledges retirement (phase 3). Only
// render_context_create reads address; no client API is called by the GL owner.
pub(crate) struct GlTicket {
    pub address: usize,
    pub api: Arc<Api>,
    pub phase: std::sync::atomic::AtomicU8,
    pub stop: Arc<AtomicBool>,
    pub error: Mutex<Option<String>>,
}
static GL_ENABLED: AtomicBool = AtomicBool::new(false);
static GL_TICKETS: Mutex<Vec<std::sync::Weak<GlTicket>>> = Mutex::new(Vec::new());
pub fn enable_gl(enabled: bool) {
    GL_ENABLED.store(enabled, Ordering::Release);
}
pub fn gl_active() -> bool {
    GL_TICKETS
        .lock()
        .unwrap()
        .iter()
        .filter_map(|t| t.upgrade())
        .any(|t| t.phase.load(Ordering::Acquire) == 2)
}
pub(crate) fn gl_tickets() -> Vec<Arc<GlTicket>> {
    let mut tickets = GL_TICKETS.lock().unwrap();
    tickets.retain(|t| t.strong_count() > 0);
    tickets.iter().filter_map(|t| t.upgrade()).collect()
}
pub struct Frame {
    pub rgba: Vec<u8>,
    pub size: [usize; 2],
}
pub enum Command {
    Pause(bool),
    Seek(f64),
    Volume(f64),
    Stop,
}
pub struct Decoder {
    commands: mpsc::Sender<Command>,
    state: Arc<Mutex<Snapshot>>,
    frames: Arc<Mutex<Option<Frame>>>,
    stop: Arc<AtomicBool>,
}
#[derive(Debug)]
pub struct StartupError {
    pub message: String,
    /// True only when the control thread has acknowledged core/render retirement.
    pub retired: bool,
}
impl Decoder {
    pub fn start(url: String, position: f64, headless_audio: bool, hls: bool) -> Result<Self> {
        Self::start_with_cancellation(
            url,
            position,
            headless_audio,
            hls,
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|error| error.message)
    }
    pub fn start_with_cancellation(
        url: String,
        position: f64,
        headless_audio: bool,
        hls: bool,
        stop: Arc<AtomicBool>,
    ) -> std::result::Result<Self, StartupError> {
        if stop.load(Ordering::Acquire) {
            return Err(StartupError {
                message: "Playback preparation canceled".into(),
                retired: true,
            });
        }
        let api = Api::load().map_err(|message| StartupError {
            message,
            retired: true,
        })?;
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let frames = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let shared = state.clone();
        let pixels = frames.clone();
        let end = stop.clone();
        std::thread::spawn(move || {
            // SAFETY: handle is created, initialized and destroyed on this thread.
            let handle = unsafe { (api.create)() };
            if handle.is_null() {
                let _ = ready_tx.send(Err("libmpv could not initialize".into()));
                return;
            }
            let mut gpu = None;
            let setup = (|| -> Result<Handle> {
                for (key, value) in [
                    ("config", "no"),
                    ("terminal", "no"),
                    ("msg-level", "all=no"),
                    ("load-scripts", "no"),
                    ("ytdl", "no"),
                    ("input-default-bindings", "no"),
                    ("input-vo-keyboard", "no"),
                    ("vo", "libmpv"),
                    ("video-timing-offset", "0"),
                    (
                        "hwdec",
                        if GL_ENABLED.load(Ordering::Acquire) {
                            "auto-safe"
                        } else {
                            "no"
                        },
                    ),
                    ("network-timeout", "10"),
                    ("cache", "yes"),
                    ("demuxer-max-bytes", "32MiB"),
                    ("demuxer-max-back-bytes", "8MiB"),
                    ("access-references", if hls { "yes" } else { "no" }),
                    (
                        "demuxer-lavf-o",
                        "protocol_whitelist=[http,tcp,crypto,data]",
                    ),
                ] {
                    api.option(handle, key, value)?;
                }
                if hls {
                    api.option(handle, "demuxer-lavf-format", "hls")?;
                }
                if GL_ENABLED.load(Ordering::Acquire) {
                    api.option(handle, "target-prim", "bt.709")?;
                    api.option(handle, "target-trc", "srgb")?;
                    api.option(handle, "tone-mapping", "bt.2390")?;
                }
                if std::env::var_os("BLUEFIN_TEST_MPV_LOG").is_some() {
                    api.option(handle, "msg-level", "all=debug")?;
                    unsafe {
                        (api.request_log)(handle, c"debug".as_ptr());
                    }
                }
                if headless_audio {
                    api.option(handle, "ao", "null")?;
                }
                // SAFETY: handle is live and not initialized yet.
                if unsafe { (api.initialize)(handle) } < 0 {
                    return Err("libmpv initialization failed".into());
                }
                if GL_ENABLED.load(Ordering::Acquire) {
                    let ticket = Arc::new(GlTicket {
                        address: handle as usize,
                        api: api.clone(),
                        phase: std::sync::atomic::AtomicU8::new(0),
                        stop: end.clone(),
                        error: Mutex::new(None),
                    });
                    GL_TICKETS.lock().unwrap().push(Arc::downgrade(&ticket));
                    let deadline = std::time::Instant::now() + Duration::from_secs(3);
                    loop {
                        let phase = ticket.phase.load(Ordering::Acquire);
                        if phase == 2 {
                            gpu = Some(ticket);
                            return Ok(std::ptr::null_mut());
                        }
                        if phase == 4 || phase == 3 {
                            break;
                        }
                        if (end.load(Ordering::Acquire) || std::time::Instant::now() > deadline)
                            && ticket
                                .phase
                                .compare_exchange(0, 4, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    if end.load(Ordering::Acquire) {
                        return Err("Playback preparation canceled".into());
                    }
                    api.option(handle, "hwdec", "no")?;
                }
                let mut render = std::ptr::null_mut();
                let mut params = [
                    Param {
                        kind: 1,
                        data: c"sw".as_ptr().cast_mut().cast(),
                    },
                    Param {
                        kind: 0,
                        data: std::ptr::null_mut(),
                    },
                ];
                // SAFETY: render API copies parameter values. Software backend
                // needs no platform display or GL context.
                if unsafe { (api.render_create)(&mut render, handle, params.as_mut_ptr()) } < 0 {
                    return Err("libmpv software video renderer unavailable".into());
                }
                Ok(render)
            })();
            let render = match setup {
                Ok(r) => r,
                Err(e) => {
                    unsafe { (api.destroy)(handle) };
                    shared.lock().unwrap().stopped = true;
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let renderer_api = api.clone();
            let renderer_end = end.clone();
            let render_address = render as usize;
            let render_state = shared.clone();
            let renderer = (!render.is_null()).then(|| {
                std::thread::spawn(move || {
                    let render = render_address as Handle;
                    // u32 allocation guarantees the required four-byte pixel alignment.
                    let mut buffer = Vec::<u32>::new();
                    let mut size = [0i32, 0i32];
                    let mut stride = 0usize;
                    let mut block = 0i32;
                    while !renderer_end.load(Ordering::Acquire) {
                        // SAFETY: exclusive render API thread, context remains live
                        // until this thread has joined; destination has exact capacity.
                        unsafe {
                            let source = render_state.lock().unwrap().display_size;
                            if let Some(dimensions) =
                                render_size(source[0] as f64, source[1] as f64)
                            {
                                size = [dimensions[0] as i32, dimensions[1] as i32];
                                stride = dimensions[0] * 4;
                                buffer.resize(dimensions[0] * dimensions[1], 0);
                            }
                            if !buffer.is_empty() && (renderer_api.render_update)(render) & 1 != 0 {
                                let mut params = [
                                    Param {
                                        kind: 17,
                                        data: size.as_mut_ptr().cast(),
                                    },
                                    Param {
                                        kind: 18,
                                        data: c"rgb0".as_ptr().cast_mut().cast(),
                                    },
                                    Param {
                                        kind: 19,
                                        data: (&mut stride as *mut usize).cast(),
                                    },
                                    Param {
                                        kind: 20,
                                        data: buffer.as_mut_ptr().cast(),
                                    },
                                    Param {
                                        kind: 12,
                                        data: (&mut block as *mut i32).cast(),
                                    },
                                    Param {
                                        kind: 0,
                                        data: std::ptr::null_mut(),
                                    },
                                ];
                                if (renderer_api.render)(render, params.as_mut_ptr()) >= 0 {
                                    let bytes = std::slice::from_raw_parts(
                                        buffer.as_ptr().cast::<u8>(),
                                        buffer.len() * 4,
                                    );
                                    let mut rgba = bytes.to_vec();
                                    for pixel in rgba.as_chunks_mut::<4>().0 {
                                        pixel[3] = 255;
                                    }
                                    *pixels.lock().unwrap() = Some(Frame {
                                        rgba,
                                        size: [size[0] as usize, size[1] as usize],
                                    });
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_millis(16));
                    }
                    // SAFETY: no render call remains; free before destroying mpv core.
                    unsafe { (renderer_api.render_free)(render) };
                })
            });
            let load = if end.load(Ordering::Acquire) {
                Err("Playback preparation canceled".into())
            } else {
                api.command(handle, &["loadfile", &url])
            };
            let startup_error = load.err();
            if startup_error.is_some() {
                end.store(true, Ordering::Release);
            } else {
                let _ = ready_tx.send(Ok(()));
            }
            let mut initial_seek = Some(position);
            while !end.load(Ordering::Acquire) {
                for command in rx.try_iter() {
                    let result = match command {
                        Command::Pause(p) => {
                            api.option(handle, "pause", if p { "yes" } else { "no" })
                        }
                        Command::Seek(s) => api
                            .command(handle, &["seek", &s.max(0.0).to_string(), "absolute+exact"]),
                        Command::Volume(v) => {
                            api.option(handle, "volume", &v.clamp(0.0, 100.0).to_string())
                        }
                        Command::Stop => {
                            end.store(true, Ordering::Release);
                            Ok(())
                        }
                    };
                    if let Err(e) = result {
                        shared.lock().unwrap().error = Some(e);
                    }
                }
                loop {
                    // SAFETY: event pointer belongs to mpv until next wait call;
                    // copy fields now and never retain borrowed event memory.
                    let event = unsafe { &*(api.wait)(handle, 0.0) };
                    if event.id == 0 {
                        break;
                    }
                    if event.id == 2
                        && !event.data.is_null()
                        && std::env::var_os("BLUEFIN_TEST_MPV_LOG").is_some()
                    {
                        #[repr(C)]
                        struct Log {
                            prefix: *const c_char,
                            level: *const c_char,
                            text: *const c_char,
                        }
                        let log = unsafe { &*event.data.cast::<Log>() };
                        if !log.text.is_null() {
                            eprint!(
                                "{}",
                                unsafe { std::ffi::CStr::from_ptr(log.text) }.to_string_lossy()
                            );
                        }
                    }
                    if event.id == 8 {
                        shared.lock().unwrap().loaded = true;
                    }
                    if event.id == 7 && !event.data.is_null() {
                        let end_file = unsafe { &*event.data.cast::<EndFile>() };
                        let reason = end_file.reason;
                        let mut s = shared.lock().unwrap();
                        s.eof = reason == 0;
                        s.failed = reason == 4;
                        if s.failed {
                            s.error = Some(format!(
                                "Native decoder could not play this media, libmpv error {}",
                                end_file.error
                            ));
                        }
                    }
                }
                if shared.lock().unwrap().loaded
                    && let Some(seconds) = initial_seek.take()
                    && seconds > 0.0
                {
                    let _ = api.command(handle, &["seek", &seconds.to_string(), "absolute+exact"]);
                }
                // Synchronous client operations may stall. Never hold a mutex
                // read by the UI/render owner across any of these calls.
                let position = api.number(handle, "time-pos");
                let duration = api.number(handle, "duration").unwrap_or(0.0);
                let paused = api.flag(handle, "pause");
                let audio_rate = api.number(handle, "audio-params/samplerate");
                let video_width = api.number(handle, "video-params/w");
                let source_size = [
                    video_width.unwrap_or(0.0) as usize,
                    api.number(handle, "video-params/h").unwrap_or(0.0) as usize,
                ];
                let display_size = render_size(
                    api.number(handle, "video-out-params/dw").unwrap_or(0.0),
                    api.number(handle, "video-out-params/dh").unwrap_or(0.0),
                )
                .unwrap_or([0, 0]);
                let codec = api.text(handle, "video-codec");
                let hwdec = api.text(handle, "hwdec-current");
                let source_transfer = api.text(handle, "video-params/gamma");
                let audio_channels = api
                    .number(handle, "audio-params/channel-count")
                    .unwrap_or(0.0) as usize;
                {
                    let mut s = shared.lock().unwrap();
                    s.position = position.unwrap_or(s.position);
                    s.duration = duration;
                    s.paused = paused;
                    s.audio_decoded |= audio_rate.is_some();
                    s.video_decoded |= video_width.is_some();
                    s.source_size = source_size;
                    s.display_size = display_size;
                    s.audio_rate = audio_rate.unwrap_or(0.0) as usize;
                    s.codec = codec;
                    s.hwdec = hwdec;
                    s.source_transfer = source_transfer;
                    s.render_path = if gpu.is_some() {
                        "OpenGL RGBA8 SDR"
                    } else {
                        "Software RGBA8"
                    }
                    .into();
                    s.audio_channels = audio_channels;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // Stop render thread first, then quit. Pending network operations have
            // the read-idle timeout; UI never waits for synchronous destruction.
            end.store(true, Ordering::Release);
            if let Some(renderer) = renderer {
                let _ = renderer.join();
            }
            if let Some(ticket) = gpu {
                if let Some(error) = ticket.error.lock().unwrap().clone() {
                    let mut s = shared.lock().unwrap();
                    s.failed = true;
                    s.error = Some(error);
                }
                while ticket.phase.load(Ordering::Acquire) != 3 {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let _ = api.command(handle, &["stop"]);
            unsafe { (api.destroy)(handle) };
            shared.lock().unwrap().stopped = true;
            if let Some(error) = startup_error {
                let _ = ready_tx.send(Err(error));
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(12)) {
            Ok(Ok(())) => Ok(Self {
                commands: tx,
                state,
                frames,
                stop,
            }),
            Ok(Err(e)) => {
                stop.store(true, Ordering::Release);
                Err(StartupError {
                    message: e,
                    retired: true,
                })
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                let retired = state.lock().unwrap().stopped;
                Err(StartupError {
                    message: "Embedded decoder startup timed out; retirement must be checked"
                        .into(),
                    retired,
                })
            }
        }
    }
    pub fn command(&self, command: Command) {
        let _ = self.commands.send(command);
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    pub fn frame(&self) -> Option<Frame> {
        self.frames.lock().unwrap().take()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}
