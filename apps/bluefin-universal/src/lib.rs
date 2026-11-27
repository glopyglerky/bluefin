//! Shared desktop and browser application. No server connectors or administration.
#[cfg(not(target_arch = "wasm32"))]
pub mod bridge;
pub mod catalog;
#[cfg(all(not(target_arch = "wasm32"), feature = "ui"))]
pub mod gl_video;
#[cfg(target_arch = "wasm32")]
pub mod mesh;
#[cfg(not(target_arch = "wasm32"))]
pub mod mpv;
pub mod playback;
#[cfg(not(target_arch = "wasm32"))]
pub mod player;
pub mod protocol;
pub mod media_resources;
pub mod service;
pub mod state;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(target_arch = "wasm32")]
pub mod web_client;
#[cfg(all(target_arch = "wasm32", feature = "ui"))]
mod web_input;

#[cfg(all(target_arch = "wasm32", feature = "ui"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = startBluefinCanvas)]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    // Semantic browser presentation requires neither a GPU nor a canvas.
    if web_sys::window().unwrap().location().search()? != "?presentation=canvas" {
        if let Some(link) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("presentation-switch"))
        {
            link.set_attribute("href", "?presentation=canvas")?;
            link.set_text_content(Some("GPU view"));
        }
        return ui::web_dom::start();
    }
    use wasm_bindgen::JsCast;
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document.get_element_by_id("bluefin").unwrap().dyn_into()?;
    document
        .get_element_by_id("bluefin")
        .unwrap()
        .remove_attribute("hidden")?;
    let options = eframe::WebOptions::default();
    let runner = eframe::WebRunner::new();
    if runner
        .start(
            canvas,
            options,
            Box::new(|cc| {
                ui::configure(&cc.egui_ctx);
                Ok(Box::new(ui::Bluefin::default()))
            }),
        )
        .await
        .is_err()
    {
        document
            .get_element_by_id("bluefin")
            .unwrap()
            .set_attribute("hidden", "")?;
        if let Some(status) = document.get_element_by_id("media-status") {
            status.set_text_content(Some("GPU view is unavailable on this browser."));
        }
        return ui::web_dom::start();
    }
    web_input::install()?;
    // Runner must live for the lifetime of this page.
    std::mem::forget(runner);
    Ok(())
}
