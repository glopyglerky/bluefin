//! Narrow Rust protocol adapter for the progressive native-style web view.
//! DOM events never receive device seeds or perform a cryptographic fallback.
use std::{cell::RefCell, time::Duration};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use crate::{mesh, protocol::{Item, Session}, playback::Selection, service::{Service, Command}};

thread_local! { static PLAYER: RefCell<Option<(String, Service)>> = const { RefCell::new(None) }; }
fn error(value: String) -> JsValue { JsValue::from_str(&value) }
fn serialize(value: Value) -> String { value.to_string() }
fn progress(value: mesh::Progress) -> String {
    serialize(match value {
        mesh::Progress::Empty => json!({"kind":"empty"}),
        mesh::Progress::Pairing(step) => json!({"kind":"pairing","externalUrl":step.external_url,
            "awaitingApproval":step.awaiting_approval,"needsBeginRetry":step.needs_begin_retry}),
        mesh::Progress::Connected(session) => json!({"kind":"connected","handle":session.token,
            "origin":session.base,"user":session.user}),
    })
}
#[wasm_bindgen(js_name = webMeshBegin)]
pub async fn begin(code: String, name: String, remember: bool) -> Result<String, JsValue> {
    mesh::begin(code,name,remember).await.map(progress).map_err(error)
}
#[wasm_bindgen(js_name = webMeshResume)]
pub async fn resume(remembered: bool) -> Result<String, JsValue> {
    if remembered {mesh::resume_remembered().await} else {mesh::resume().await}.map(progress).map_err(error)
}
#[wasm_bindgen(js_name = webMeshProceed)]
pub async fn proceed() -> Result<String, JsValue> {
    mesh::proceed().await.map(progress).map_err(error)
}
#[wasm_bindgen(js_name = webMeshImage)]
pub fn decode_image(bytes: &[u8]) -> Result<String, JsValue> {
    if bytes.len() > 8 * 1024 * 1024 {return Err(JsValue::from_str("Choose a PNG or JPEG smaller than 8 MiB."));}
    mesh::image_code(bytes).map_err(error)
}
#[wasm_bindgen(js_name = webMeshRequest)]
pub async fn request(handle: String, method: String, path: String, query: String, body: Option<String>)
    -> Result<String, JsValue> {
    if query.len() > 8192 || body.as_ref().is_some_and(|body| body.len()>8192) {
        return Err(JsValue::from_str("Request limit."));
    }
    let session = mesh::restored_session(&handle).map_err(error)?;
    let query: Vec<(String,String)> = serde_json::from_str(&query).map_err(|_| JsValue::from_str("Invalid query."))?;
    let refs: Vec<_> = query.iter().map(|(key,value)|(key.as_str(),value.clone())).collect();
    let body: Option<Value> = body.map(|value| serde_json::from_str(&value))
        .transpose().map_err(|_| JsValue::from_str("Invalid request."))?;
    mesh::request(&session.base,&handle,&method,&path,&refs,body).await.map(serialize).map_err(error)
}
#[wasm_bindgen(js_name = webMeshArtwork)]
pub async fn artwork(handle: String, item: String, backdrop: bool) -> Result<Vec<u8>, JsValue> {
    mesh::artwork(&handle,&item,backdrop).await.map_err(error)
}
#[wasm_bindgen(js_name = webMeshInvitation)]
pub fn invitation(serialized: String) -> Result<String, JsValue> {
    if serialized.len()>65536 {return Err(JsValue::from_str("Invitation response limit."));}
    let value = serde_json::from_str(&serialized).map_err(|_| JsValue::from_str("Invalid invitation response."))?;
    let invite = mesh::IssuedInvite::from_response(value).map_err(error)?;
    Ok(serialize(json!({"code":invite.code,"webLink":invite.link,"image":invite.image,"handle":invite.handle})))
}
fn player(handle: &str) -> Result<(), JsValue> {
    let session: Session = mesh::restored_session(handle).map_err(error)?;
    PLAYER.with(|player| {
        let mut player = player.borrow_mut();
        if player.as_ref().is_some_and(|(current,_)| current != handle) {
            return Err(JsValue::from_str("Sign out before changing playback accounts."));
        }
        if player.is_none() { *player=Some((handle.to_owned(),Service::start(session,false,None))); }
        Ok(())
    })
}
#[wasm_bindgen(js_name = webMeshPlay)]
pub fn play(handle: String, serialized: String) -> Result<(), JsValue> {
    if serialized.len()>128*1024 {return Err(JsValue::from_str("Item limit."));}
    let item: Item = serde_json::from_str(&serialized).map_err(|_| JsValue::from_str("Invalid library item."))?;
    if item.id.is_nil() || !item.playable() {return Err(JsValue::from_str("Choose a playable item."));}
    player(&handle)?;
    PLAYER.with(|player| {if let Some((_,service))=player.borrow().as_ref() {
        service.send(Command::Play(Box::new(item),Selection::default(),None));
    }});
    Ok(())
}
#[wasm_bindgen(js_name = webMeshPlayback)]
pub fn playback(handle: String) -> Result<String, JsValue> {
    PLAYER.with(|player| {
        let player = player.borrow();
        let Some((current,service))=player.as_ref() else {return Ok("null".into());};
        if current != &handle {return Err(JsValue::from_str("Playback account mismatch."));}
        let state=service.snapshot();
        Ok(serialize(json!({"plan":state.plan,"position":state.position,"duration":state.duration,
            "paused":state.paused,"busy":state.busy,"finished":state.finished,"message":state.message,
            "cleanupAcknowledged":state.cleanup_acknowledged,"revoked":state.revoked})))
    })
}
#[wasm_bindgen(js_name = webMeshControl)]
pub fn control(handle: String, command: String, value: f64) -> Result<(), JsValue> {
    let command=match command.as_str() {
        "stop"=>Command::Stop,
        "pause"=>Command::Pause(value!=0.0),
        "seek" if value.is_finite() && value>=0.0=>Command::Seek(value),
        "volume" if value.is_finite() && (0.0..=100.0).contains(&value)=>Command::Volume(value),
        _=>return Err(JsValue::from_str("Invalid playback command.")),
    };
    PLAYER.with(|player| {
        let player=player.borrow();
        if let Some((current,service))=player.as_ref() {
            if current!=&handle {return Err(JsValue::from_str("Playback account mismatch."));}
            service.send(command);
        }
        Ok(())
    })
}
#[wasm_bindgen(js_name = webMeshLogout)]
pub async fn logout(handle: String) -> Result<(), JsValue> {
    let playback=PLAYER.with(|player| {
        let mut player=player.borrow_mut();
        if player.as_ref().is_some_and(|(current,_)| current==&handle) {player.take()} else {None}
    });
    if let Some((_,service))=playback {
        service.send(Command::Logout);
        // Service owns draining any in-flight PlaybackInfo before remote release.
        let result=n0_future::time::timeout(Duration::from_secs(45),async {
            loop {
                let state=service.snapshot();
                if state.finished {break if state.revoked {Ok(())} else {Err(JsValue::from_str(&state.message))};}
                n0_future::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.map_err(|_| JsValue::from_str("Local playback stopped. Remote sign-out is not confirmed."))?;
        result
    } else {mesh::logout(&handle).await.map_err(error)}
}
#[wasm_bindgen(js_name = webMeshPageExit)]
pub fn page_exit() {
    PLAYER.with(|player| {
        if let Some((_,service))=player.borrow().as_ref() {service.send(Command::Stop);}
    });
    mesh::retire_page();
}
