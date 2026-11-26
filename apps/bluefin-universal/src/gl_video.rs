//! The render context and GL objects never cross threads. Egui callbacks carry
//! no raw handles: they access this thread's owner only while GL is current.
use crate::mpv::{GlTicket, Param};
use eframe::{
    egui, egui_glow,
    glow::{self, HasContext},
};
use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_void},
    sync::{Arc, atomic::Ordering},
};
thread_local! { static OWNER: RefCell<Option<Owner>> = const { RefCell::new(None) }; }
struct Owner {
    gl: Arc<glow::Context>,
    resolver: Resolver,
    video: Option<Video>,
}
struct Video {
    ticket: Arc<GlTicket>,
    render: *mut c_void,
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    size: [i32; 2],
}
struct Resolver {
    library: libloading::Library,
}
impl Resolver {
    fn new() -> Option<Self> {
        #[cfg(target_os = "linux")]
        let name = "libGL.so.1";
        #[cfg(target_os = "windows")]
        let name = "opengl32.dll";
        #[cfg(target_os = "macos")]
        let name = "/System/Library/Frameworks/OpenGL.framework/OpenGL";
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        let name = "unsupported";
        // SAFETY: platform GL dispatch library, retained for the owner lifetime.
        unsafe {
            libloading::Library::new(name)
                .ok()
                .map(|library| Self { library })
        }
    }
    unsafe extern "C" fn get(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        // SAFETY: mpv calls synchronously during creation, with our live resolver
        // and NUL-terminated symbol name. Platform dispatch uses current context.
        unsafe {
            let this = &*ctx.cast::<Self>();
            #[cfg(target_os = "linux")]
            if let Ok(get) = this
                .library
                .get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(
                    b"glXGetProcAddressARB\0",
                )
            {
                return get(name);
            }
            #[cfg(target_os = "windows")]
            if let Ok(get) = this
                .library
                .get::<unsafe extern "system" fn(*const c_char) -> *mut c_void>(
                    b"wglGetProcAddress\0",
                )
            {
                let p = get(name);
                if (p as usize) > 3 && p as isize != -1 {
                    return p;
                }
            }
            this.library
                .get::<*mut c_void>(CStr::from_ptr(name).to_bytes_with_nul())
                .map(|p| *p)
                .unwrap_or(std::ptr::null_mut())
        }
    }
}
#[repr(C)]
struct Init {
    get: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    ctx: *mut c_void,
}
#[repr(C)]
struct Fbo {
    id: i32,
    w: i32,
    h: i32,
    format: i32,
}
/// Called during eframe creation, while the primary context is current.
/// Returns false to select the software path when platform dispatch is absent.
pub fn initialize(gl: Option<Arc<glow::Context>>) {
    let owner = gl.and_then(|gl| {
        Resolver::new().map(|resolver| Owner {
            gl,
            resolver,
            video: None,
        })
    });
    crate::mpv::enable_gl(owner.is_some());
    OWNER.with(|o| *o.borrow_mut() = owner);
}
impl Owner {
    /// Prepare a replacement image before committing its declared dimensions.
    /// Completeness of the previous attachment does not prove allocation worked.
    unsafe fn resize(gl: &glow::Context, video: &mut Video, size: [i32; 2]) -> Result<(), String> {
        unsafe {
            let replacement = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(replacement));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            if gl.get_error() != glow::NO_ERROR {
                gl.delete_texture(replacement);
                return Err("Video surface allocation failed; playback stopped".into());
            }
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(replacement),
                0,
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE
                || gl.get_error() != glow::NO_ERROR
            {
                gl.delete_texture(replacement);
                return Err("Video surface attachment failed; playback stopped".into());
            }
            gl.delete_texture(video.texture);
            video.texture = replacement;
            video.size = size;
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            if gl.get_error() != glow::NO_ERROR {
                return Err("Video surface initialization failed; playback stopped".into());
            }
            Ok(())
        }
    }
    // SAFETY invariant for all methods: invoked only by egui_glow paint callback
    // (or on_exit), on the original owner thread with the same current context.
    unsafe fn retire(&mut self) {
        if let Some(v) = self.video.take() {
            unsafe {
                (v.ticket.api.render_free)(v.render);
                self.gl.delete_framebuffer(v.fbo);
                self.gl.delete_texture(v.texture);
            }
            v.ticket.phase.store(3, Ordering::Release);
        }
    }
    unsafe fn attach(&mut self, ticket: Arc<GlTicket>) {
        if ticket
            .phase
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let mut render = std::ptr::null_mut();
        let mut init = Init {
            get: Resolver::get,
            ctx: (&mut self.resolver as *mut Resolver).cast(),
        };
        let mut params = [
            Param {
                kind: 1,
                data: c"opengl".as_ptr().cast_mut().cast(),
            },
            Param {
                kind: 2,
                data: (&mut init as *mut Init).cast(),
            },
            Param {
                kind: 0,
                data: std::ptr::null_mut(),
            },
        ];
        // No client API calls on this thread. Decoder cannot destroy its core
        // until phase 3, and does not load media until phase 2.
        unsafe {
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.disable(glow::BLEND);
            self.gl.bind_vertex_array(None);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
            self.gl.use_program(None);
            self.gl.active_texture(glow::TEXTURE0);
            if (ticket.api.render_create)(
                &mut render,
                ticket.address as *mut c_void,
                params.as_mut_ptr(),
            ) < 0
            {
                ticket.phase.store(4, Ordering::Release);
                return;
            }
            let objects = (self.gl.create_framebuffer(), self.gl.create_texture());
            match objects {
                (Ok(fbo), Ok(texture)) => {
                    self.video = Some(Video {
                        ticket: ticket.clone(),
                        render,
                        fbo,
                        texture,
                        size: [0, 0],
                    });
                    ticket.phase.store(2, Ordering::Release);
                }
                (f, t) => {
                    if let Ok(f) = f {
                        self.gl.delete_framebuffer(f);
                    }
                    if let Ok(t) = t {
                        self.gl.delete_texture(t);
                    }
                    (ticket.api.render_free)(render);
                    ticket.phase.store(4, Ordering::Release);
                }
            }
        }
    }
    unsafe fn paint(
        &mut self,
        info: egui::PaintCallbackInfo,
        painter: &egui_glow::Painter,
        draw: bool,
    ) {
        if !Arc::ptr_eq(&self.gl, painter.gl()) {
            return;
        }
        unsafe {
            if self
                .video
                .as_ref()
                .is_some_and(|v| v.ticket.stop.load(Ordering::Acquire))
            {
                self.retire();
            }
            if self.video.is_none() {
                for t in crate::mpv::gl_tickets() {
                    if !t.stop.load(Ordering::Acquire) {
                        self.attach(t);
                        if self.video.is_some() {
                            break;
                        }
                    }
                }
            }
            // Cancellation can arrive while render_context_create owns phase 1.
            // Retire immediately on this GL owner before any draw or publication.
            if self
                .video
                .as_ref()
                .is_some_and(|v| v.ticket.stop.load(Ordering::Acquire))
            {
                self.retire();
            }
            if draw && let Some(v) = self.video.as_mut() {
                let viewport = info.viewport_in_pixels();
                let Some(size) =
                    crate::mpv::render_size(viewport.width_px as f64, viewport.height_px as f64)
                else {
                    self.gl
                        .bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
                    return;
                };
                let size = [size[0] as i32, size[1] as i32];
                self.gl.disable(glow::SCISSOR_TEST);
                self.gl.disable(glow::BLEND);
                self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(v.fbo));
                self.gl.bind_texture(glow::TEXTURE_2D, Some(v.texture));
                let resize = if v.size != size {
                    Self::resize(&self.gl, v, size)
                } else {
                    Ok(())
                };
                if let Err(error) = resize {
                    *v.ticket.error.lock().unwrap() = Some(error);
                    v.ticket.stop.store(true, Ordering::Release);
                    self.retire();
                } else if self.gl.check_framebuffer_status(glow::FRAMEBUFFER)
                    != glow::FRAMEBUFFER_COMPLETE
                {
                    *v.ticket.error.lock().unwrap() =
                        Some("Video framebuffer became unavailable; playback stopped".into());
                    v.ticket.stop.store(true, Ordering::Release);
                    self.retire();
                } else {
                    let mut fbo = Fbo {
                        id: v.fbo.0.get() as i32,
                        w: size[0],
                        h: size[1],
                        format: glow::RGBA8 as i32,
                    };
                    let mut flip = 1i32;
                    let mut block = 0i32;
                    let mut params = [
                        Param {
                            kind: 3,
                            data: (&mut fbo as *mut Fbo).cast(),
                        },
                        Param {
                            kind: 4,
                            data: (&mut flip as *mut i32).cast(),
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
                    (v.ticket.api.render_update)(v.render);
                    if (v.ticket.api.render)(v.render, params.as_mut_ptr()) < 0
                        || self.gl.get_error() != glow::NO_ERROR
                    {
                        *v.ticket.error.lock().unwrap() = Some(
                            "Video render context failed or was lost; playback stopped".into(),
                        );
                        v.ticket.stop.store(true, Ordering::Release);
                        self.retire();
                    } else {
                        self.gl
                            .bind_framebuffer(glow::READ_FRAMEBUFFER, Some(v.fbo));
                        self.gl
                            .bind_framebuffer(glow::DRAW_FRAMEBUFFER, painter.intermediate_fbo());
                        let clip = info.clip_rect_in_pixels();
                        self.gl.enable(glow::SCISSOR_TEST);
                        self.gl.scissor(
                            clip.left_px,
                            clip.from_bottom_px,
                            clip.width_px,
                            clip.height_px,
                        );
                        self.gl.blit_framebuffer(
                            0,
                            0,
                            size[0],
                            size[1],
                            viewport.left_px,
                            viewport.from_bottom_px,
                            viewport.left_px + viewport.width_px,
                            viewport.from_bottom_px + viewport.height_px,
                            glow::COLOR_BUFFER_BIT,
                            glow::LINEAR,
                        );
                        if self.gl.get_error() != glow::NO_ERROR {
                            *v.ticket.error.lock().unwrap() =
                                Some("Video presentation failed; playback stopped".into());
                            v.ticket.stop.store(true, Ordering::Release);
                            self.retire();
                        }
                    }
                }
            }
            // egui_glow 0.33.3 prepares its program/VAO/blend/viewport after
            // callbacks; framebuffer bindings must be restored explicitly here.
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
        }
    }
}
pub fn callback(ui: &mut egui::Ui, rect: egui::Rect, draw: bool) {
    ui.painter().add(egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            OWNER.with(|owner| {
                if let Some(o) = owner.borrow_mut().as_mut() {
                    unsafe {
                        o.paint(info, painter, draw);
                    }
                }
            });
        })),
    });
}
pub fn shutdown() {
    crate::mpv::enable_gl(false);
    OWNER.with(|owner| {
        if let Some(mut o) = owner.borrow_mut().take() {
            if let Some(v) = &o.video {
                v.ticket.stop.store(true, Ordering::Release);
            }
            unsafe {
                o.retire();
            }
        }
    });
    for t in crate::mpv::gl_tickets() {
        let _ = t
            .phase
            .compare_exchange(0, 4, Ordering::AcqRel, Ordering::Acquire);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    #[ignore = "requires existing surfaceless EGL and original 1080p fixture"]
    fn actual_egl_framebuffer_resize_and_retirement() {
        // SAFETY: test-local EGL display/pbuffer/context; exact EGL 1.5 ABI.
        // No window, device input or display session is opened. Every GL owner
        // is explicitly retired before releasing this thread's current context.
        unsafe {
            let egl = libloading::Library::new("libEGL.so.1").expect("existing EGL library");
            macro_rules! sym {
                ($name:literal,$ty:ty) => {
                    *egl.get::<$ty>(concat!($name, "\0").as_bytes()).unwrap()
                };
            }
            let get = sym!(
                "eglGetProcAddress",
                unsafe extern "C" fn(*const c_char) -> *mut c_void
            );
            let platform: unsafe extern "C" fn(u32, *mut c_void, *const i32) -> *mut c_void =
                std::mem::transmute(get(c"eglGetPlatformDisplayEXT".as_ptr()));
            let display = platform(0x31DD, std::ptr::null_mut(), std::ptr::null());
            let init = sym!(
                "eglInitialize",
                unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32) -> u32
            );
            let mut major = 0;
            let mut minor = 0;
            assert_ne!(
                init(display, &mut major, &mut minor),
                0,
                "surfaceless EGL unavailable"
            );
            let bind = sym!("eglBindAPI", unsafe extern "C" fn(u32) -> u32);
            assert_ne!(bind(0x30A2), 0);
            let choose = sym!(
                "eglChooseConfig",
                unsafe extern "C" fn(
                    *mut c_void,
                    *const i32,
                    *mut *mut c_void,
                    i32,
                    *mut i32,
                ) -> u32
            );
            let mut config = std::ptr::null_mut();
            let mut n = 0;
            let attrs = [
                0x3033, 1, 0x3040, 8, 0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3038,
            ];
            assert_ne!(choose(display, attrs.as_ptr(), &mut config, 1, &mut n), 0);
            assert!(n > 0);
            let surface = sym!(
                "eglCreatePbufferSurface",
                unsafe extern "C" fn(*mut c_void, *mut c_void, *const i32) -> *mut c_void
            )(
                display,
                config,
                [0x3057, 1920, 0x3056, 1080, 0x3038].as_ptr(),
            );
            assert!(!surface.is_null());
            let context = sym!(
                "eglCreateContext",
                unsafe extern "C" fn(
                    *mut c_void,
                    *mut c_void,
                    *mut c_void,
                    *const i32,
                ) -> *mut c_void
            )(
                display,
                config,
                std::ptr::null_mut(),
                [0x3098, 3, 0x30FB, 3, 0x3038].as_ptr(),
            );
            assert!(!context.is_null());
            let current = sym!(
                "eglMakeCurrent",
                unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> u32
            );
            assert_ne!(current(display, surface, surface, context), 0);
            let gl = Arc::new(glow::Context::from_loader_function_cstr(|name| {
                get(name.as_ptr()).cast_const()
            }));
            eprintln!(
                "EGL {major}.{minor}; GL {}; renderer {}",
                gl.get_parameter_string(glow::VERSION),
                gl.get_parameter_string(glow::RENDERER)
            );
            let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false).unwrap();
            initialize(Some(gl.clone()));
            let decoder_thread = std::thread::spawn(|| {
                crate::mpv::Decoder::start(
                    std::env::var("BLUEFIN_QUALITY_FIXTURE").unwrap(),
                    0.0,
                    true,
                    false,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut owner = OWNER.with(|o| o.borrow_mut().take().unwrap());
            while !decoder_thread.is_finished() {
                owner.paint(
                    egui::PaintCallbackInfo {
                        viewport: egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1920.0, 1080.0),
                        ),
                        clip_rect: egui::Rect::EVERYTHING,
                        pixels_per_point: 1.0,
                        screen_size_px: [1920, 1080],
                    },
                    &painter,
                    true,
                );
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            let decoder = decoder_thread.join().unwrap().unwrap();
            let mut nonblack = false;
            for size in [[1920, 1080], [800, 800], [1280, 720]] {
                let start = Instant::now();
                let mut pixels = vec![0u8; size[0] * size[1] * 4];
                while start.elapsed() < Duration::from_millis(350) {
                    let info = egui::PaintCallbackInfo {
                        viewport: egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(size[0] as f32, size[1] as f32),
                        ),
                        clip_rect: egui::Rect::from_min_max(
                            egui::Pos2::ZERO,
                            egui::pos2(1920.0, 1080.0),
                        ),
                        pixels_per_point: 1.0,
                        screen_size_px: [1920, 1080],
                    };
                    owner.paint(info, &painter, true);
                    assert_eq!(
                        owner.video.as_ref().unwrap().size,
                        [size[0] as i32, size[1] as i32]
                    );
                    gl.read_pixels(
                        0,
                        1080 - size[1] as i32,
                        size[0] as i32,
                        size[1] as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                    nonblack |= pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[0] > 40 || p[1] > 40 || p[2] > 40);
                    assert_eq!(gl.get_error(), glow::NO_ERROR);
                    std::thread::sleep(Duration::from_millis(16));
                }
                eprintln!(
                    "actual framebuffer {}x{}; nonblack={nonblack}",
                    size[0], size[1]
                );
                if size == [800, 800] {
                    let pixel = &pixels[(790 * 800 + 400) * 4..][..3];
                    assert!(
                        pixel.iter().all(|v| *v < 8),
                        "square viewport must letterbox widescreen video"
                    );
                }
            }
            assert!(nonblack);
            assert!(
                decoder.frame().is_none(),
                "GPU path must not enqueue CPU frames"
            );
            decoder.stop();
            owner.retire();
            drop(owner);
            while !decoder.snapshot().stopped {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            crate::mpv::enable_gl(false);
            painter.destroy();
            drop(painter);
            drop(gl);
            assert_ne!(
                current(
                    display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ),
                0
            );
            sym!(
                "eglDestroyContext",
                unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32
            )(display, context);
            sym!(
                "eglDestroySurface",
                unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32
            )(display, surface);
            sym!("eglTerminate", unsafe extern "C" fn(*mut c_void) -> u32)(display);
        }
    }
}
