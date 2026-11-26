//! Keep eframe's inactive IME proxy from enlarging the document after a resize.
//! Defer blur handling: Gboard intentionally blurs and refocuses within one event.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

thread_local! {
    static INPUT_PROXY: RefCell<Option<InputProxy>> = const { RefCell::new(None) };
}

struct InputProxy {
    input: web_sys::HtmlElement,
    window: web_sys::Window,
    alive: Rc<Cell<bool>>,
    queue: js_sys::Function,
    _reset: js_sys::Function,
}

impl Drop for InputProxy {
    fn drop(&mut self) {
        self.alive.set(false);
        let _ = self
            .input
            .remove_event_listener_with_callback("blur", &self.queue);
        let _ = self
            .window
            .remove_event_listener_with_callback("resize", &self.queue);
        // Callbacks are JS-owned, so an already queued microtask remains valid and
        // observes alive=false. Its captured state can then be garbage collected.
    }
}

pub(super) fn install() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Browser unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("Document unavailable"))?;
    // eframe 0.33.3 creates this single, direct body child after WebRunner::start.
    // Scope the handler to that proxy; ordinary form inputs remain unchanged.
    let input: web_sys::HtmlElement = document
        .query_selector("body > input[type=text][autocapitalize=off]")?
        .ok_or_else(|| JsValue::from_str("Text input proxy unavailable"))?
        .dyn_into()?;
    input.set_attribute("data-bluefin-ime-proxy", "")?;
    let alive = Rc::new(Cell::new(true));
    let pending = Rc::new(Cell::new(false));
    let reset: js_sys::Function = {
        let alive = alive.clone();
        let pending = pending.clone();
        let input = input.clone();
        Closure::<dyn FnMut()>::new(move || {
            pending.set(false);
            if !alive.get() || document.active_element().as_ref() == Some(input.as_ref()) {
                return;
            }
            // Reset the actual stale position, preserving the focused proxy's
            // caret and composition placement. Never hide page overflow.
            let _ = input.style().set_property("left", "0px");
            let _ = input.style().set_property("top", "0px");
        })
        .into_js_value()
        .unchecked_into()
    };
    let queue: js_sys::Function = {
        let alive = alive.clone();
        let pending = pending.clone();
        let reset = reset.clone();
        let window = window.clone();
        Closure::<dyn FnMut()>::new(move || {
            if alive.get() && !pending.replace(true) {
                window.queue_microtask(&reset);
            }
        })
        .into_js_value()
        .unchecked_into()
    };
    let guard = InputProxy {
        input,
        window,
        alive,
        queue,
        _reset: reset,
    };
    guard
        .input
        .add_event_listener_with_callback("blur", &guard.queue)?;
    guard
        .window
        .add_event_listener_with_callback("resize", &guard.queue)?;
    INPUT_PROXY.with(|slot| {
        slot.borrow_mut().replace(guard);
    });
    Ok(())
}
