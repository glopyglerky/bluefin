//! Uses the same typed contract as Web; transport retains account credentials.
use super::*;
use bluefin_sync_play::requests::{RoomRequest, request_using};

pub(super) fn call(v: &Value) -> Result<Value> {
    if v.get("mesh").is_some_and(|mesh| !mesh.is_null() && !mesh.is_object()) {
        return Err(err("Invalid mesh account transport"));
    }
    // Never admit a shared fallback device identity to a room session.
    let device = text(v, "deviceId")?;
    let (_, _, fallback) = client_platform::Platform::read(v)?.identity();
    if device == fallback {
        return Err(err("Sign in with this device before joining a room."));
    }
    let api = Api::new(v)?;
    api.auth(true)?;
    let operation: RoomRequest =
        serde_json::from_value(v["request"].clone()).map_err(|_| err("Invalid room request"))?;
    let mut send = move |method: &'static str, path: String, body: Option<Value>| {
        let result = Method::from_bytes(method.as_bytes())
            .map_err(|_| err("Invalid room method"))
            .and_then(|method| {
                api.request(
                    method,
                    &path.split('/').collect::<Vec<_>>(),
                    vec![],
                    body,
                    true,
                )
            });
        Box::pin(std::future::ready(result))
            as std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>>>>
    };
    // All host I/O completes in the callback; this future never yields Pending.
    let mut future = Box::pin(request_using(operation, &mut send));
    let waker = std::task::Waker::noop();
    match std::future::Future::poll(future.as_mut(), &mut std::task::Context::from_waker(waker)) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => Err(err("Room transport unexpectedly deferred")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_mesh_cannot_route_room_credentials_over_http() {
        for mesh in [json!("invalid"), json!([]), json!(0), json!(true)] {
            let result = call(&json!({
                "deviceId":"room-fixture-device", "token":"room-fixture-token",
                "server":"not an HTTP origin", "mesh":mesh, "request":{"kind":"list"}
            }));
            assert_eq!(result.unwrap_err(), "Invalid mesh account transport");
        }
    }
}
