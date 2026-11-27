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
    (if remembered {mesh::resume_remembered().await} else {mesh::resume().await}).map(progress).map_err(error)
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
/// Room controls use the same device-bound account and protocol as playback.
#[wasm_bindgen(js_name = webMeshRoom)]
pub async fn room(handle: String, request: String) -> Result<String, JsValue> {
    if request.len() > 4096 {
        return Err(JsValue::from_str("Room request limit."));
    }
    let session = mesh::restored_session(&handle).map_err(error)?;
    let operation = serde_json::from_str(&request)
        .map_err(|_| JsValue::from_str("Invalid room request."))?;
    crate::sync_play::request(&session, operation).await.map(serialize).map_err(error)
}
#[wasm_bindgen(js_name = webMeshArtwork)]
pub async fn artwork(handle: String, item: String, backdrop: bool) -> Result<Vec<u8>, JsValue> {
    mesh::artwork(&handle,&item,backdrop).await.map_err(error)
}
#[wasm_bindgen(js_name = webMeshSportsPreferences)]
pub async fn sports_preferences(handle: String, version: String, body: String) -> Result<String, JsValue> {
    mesh::sports_preferences(&handle,&version,&body).await.map(serialize).map_err(error)
}


#[wasm_bindgen(js_name = webMeshKaraokeStem)]
pub async fn karaoke_stem(handle: String, item: String, source: String,
    revision: String, role: String, expected_bytes: u32) -> Result<Vec<u8>, JsValue> {
    mesh::karaoke_stem(&handle,&item,&source,&revision,&role,expected_bytes).await.map_err(error)
}
#[wasm_bindgen(js_name = webMeshSportsLogo)]
pub async fn sports_logo(handle: String, asset: String) -> Result<Vec<u8>, JsValue> {
    mesh::sports_logo(&handle,&asset).await.map_err(error)
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
pub fn play(handle: String, serialized: String, start_paused: Option<bool>) -> Result<(), JsValue> {
    if serialized.len()>128*1024 {return Err(JsValue::from_str("Item limit."));}
    let value: Value = serde_json::from_str(&serialized).map_err(|_| JsValue::from_str("Invalid library item."))?;
    let item: Item = serde_json::from_value(value.clone()).map_err(|_| JsValue::from_str("Invalid library item."))?;
    let position = value["UserData"]["PlaybackPositionTicks"].as_u64()
        .filter(|position| *position <= 9_007_199_254_740_991 &&
            value["RunTimeTicks"].as_u64().is_some_and(|duration| *position < duration))
        .map_or(0.0, |ticks| ticks as f64 / 10_000_000.0);
    if item.id.is_nil() || !item.playable() {return Err(JsValue::from_str("Choose a playable item."));}
    player(&handle)?;
    PLAYER.with(|player| {if let Some((_,service))=player.borrow().as_ref() {
        service.send(Command::Play(Box::new(item),Selection {position,start_paused:start_paused.unwrap_or(false),..Selection::default()},None));
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
        let media_source_id = state.plan.as_ref().map(|plan| plan.media_source.clone());
        Ok(serialize(json!({"plan":state.plan,"mediaSourceId":media_source_id,"position":state.position,"duration":state.duration,
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
#[wasm_bindgen(js_name = webMeshStop)]
pub async fn stop(handle: String) -> Result<(), JsValue> {
    let receipt = PLAYER.with(|player| {
        let player = player.borrow();
        let Some((current, service)) = player.as_ref() else { return Ok(None); };
        if current != &handle { return Err(JsValue::from_str("Playback account mismatch.")); }
        let status = service.snapshot();
        if status.finished {
            return if status.cleanup_acknowledged { Ok(None) }
                else { Err(JsValue::from_str("Playback retired without confirmed remote cleanup. Reload before starting another source.")) };
        }
        let (tx, rx) = std::sync::mpsc::channel();
        service.send(Command::StopReceipt(tx));
        Ok(Some(rx))
    })?;
    let Some(receipt) = receipt else { return Ok(()); };
    n0_future::time::timeout(Duration::from_secs(45), async {
        loop {
            match receipt.try_recv() {
                Ok(result) => return result.map_err(error),
                Err(std::sync::mpsc::TryRecvError::Disconnected) =>
                    return Err(JsValue::from_str("The playback retirement receipt was lost.")),
                Err(std::sync::mpsc::TryRecvError::Empty) =>
                    n0_future::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }).await.map_err(|_| JsValue::from_str("Playback retirement is still pending. Reload before starting another source."))?
}
#[wasm_bindgen(js_name = webMeshLogout)]
pub async fn logout(handle: String) -> Result<(), JsValue> {
    // Keep the service owned until it has finished. A timeout must not drop it
    // and permit a second service while delayed cleanup can still run.
    let playback = PLAYER.with(|player| {
        let player = player.borrow();
        player.as_ref().filter(|(current,_)| current == &handle).map(|(_,service)| {
            service.send(Command::Logout);
            service.status.clone()
        })
    });
    if let Some(status) = playback {
        let result = n0_future::time::timeout(Duration::from_secs(45),async {
            loop {
                let state = status.lock().unwrap().clone();
                if state.finished {
                    break if state.revoked { Ok(()) } else { Err(JsValue::from_str(&state.message)) };
                }
                n0_future::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.map_err(|_| JsValue::from_str("Local playback stopped. Remote sign-out is not confirmed."))?;
        PLAYER.with(|player| {
            let mut player=player.borrow_mut();
            if player.as_ref().is_some_and(|(current,service)| current == &handle && service.snapshot().finished) {
                player.take();
            }
        });
        result
    } else { mesh::logout(&handle).await.map_err(error) }
}
#[wasm_bindgen(js_name = webMeshSuspend)]
pub async fn suspend(handle: String) -> Result<(), JsValue> { mesh::suspend(&handle).await.map_err(error) }
#[wasm_bindgen(js_name = webMeshPageExit)]
pub fn page_exit() {
    PLAYER.with(|player| {
        if let Some((_,service))=player.borrow().as_ref() {service.send(Command::Stop);}
    });
    mesh::retire_page();
}

/// Keeps ordinary HTTPS room traffic inside the existing same-origin account
/// adapter, including its per-tab device identity and cancellation checks.
#[wasm_bindgen(js_name = webRoomHttp)]
pub async fn room_http(adapter: js_sys::Function, request: String) -> Result<String, JsValue> {
    if request.len() > 4096 {
        return Err(JsValue::from_str("Room request limit."));
    }
    let operation =
        serde_json::from_str(&request).map_err(|_| JsValue::from_str("Invalid room request."))?;
    crate::sync_play::request_using(operation, &mut move |method, path, body| {
        let adapter = adapter.clone();
        Box::pin(async move {
            let result = adapter
                .call3(
                    &JsValue::UNDEFINED,
                    &JsValue::from_str(method),
                    &JsValue::from_str(&path),
                    &body.map_or(JsValue::UNDEFINED, |body| {
                        JsValue::from_str(&body.to_string())
                    }),
                )
                .map_err(|value| {
                    value
                        .as_string()
                        .unwrap_or_else(|| "Room request failed.".into())
                })?;
            let result = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&result))
                .await
                .map_err(|value| {
                    value
                        .as_string()
                        .unwrap_or_else(|| "Room connection failed.".into())
                })?;
            serde_json::from_str(&result.as_string().ok_or("Invalid room response.")?)
                .map_err(|_| "Invalid room response.".into())
        })
    })
    .await
    .map(serialize)
    .map_err(error)
}

#[wasm_bindgen(js_name = webRoomProjection)]
pub fn room_projection(observation: &str) -> Result<String, JsValue> {
    use bluefin_sync_play::{ClockExchange, RoomClock, TimelineState};
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Observation {
        local_sent: u64,
        local_received: u64,
        server_received: i64,
        server_sent: i64,
        local_now: u64,
        anchor_ms: i64,
        position_ticks: u64,
        player_ticks: u64,
        playing: bool,
    }
    if observation.len() > 1024 {
        return Err(JsValue::from_str("Room observation limit."));
    }
    let input: Observation = serde_json::from_str(observation)
        .map_err(|_| JsValue::from_str("Invalid room clock observation."))?;
    let target = RoomClock::measure(ClockExchange {
        local_sent: input.local_sent,
        local_received: input.local_received,
        server_received: input.server_received,
        server_sent: input.server_sent,
    })
    .and_then(|clock| {
        TimelineState {
            position_ticks: input.position_ticks,
            anchor_ms: input.anchor_ms,
            playing: input.playing,
        }
        .target(clock, input.local_now, input.player_ticks)
    });
    let target =
        target.ok_or_else(|| JsValue::from_str("The room clock could not be synchronized."))?;
    Ok(serialize(json!({
        "positionTicks":target.position_ticks, "serverNowMs":target.server_now_ms,
        "paused":target.paused, "seek":target.seek,
    })))
}
