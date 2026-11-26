#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: if std::env::args().any(|a| a == "--wgpu") {
            eframe::Renderer::Wgpu
        } else {
            eframe::Renderer::Glow
        },
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([360.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Bluefin",
        options,
        Box::new(|cc| {
            bluefin_universal::gl_video::initialize(cc.gl.clone());
            bluefin_universal::ui::configure(&cc.egui_ctx);
            Ok(Box::new(bluefin_universal::ui::Bluefin::default()))
        }),
    )
}
#[cfg(target_arch = "wasm32")]
fn main() {}
