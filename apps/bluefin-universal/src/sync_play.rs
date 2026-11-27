//! Shared authenticated room protocol over the universal transport.
use crate::protocol::{self, Result, Session};
pub use bluefin_sync_play::requests::*;
use serde_json::Value;

pub async fn request(session: &Session, operation: RoomRequest) -> Result<Value> {
    let session = session.clone();
    request_using(operation, &mut move |method, path, body| {
        let session = session.clone();
        Box::pin(async move {
            protocol::request(&session.base, &session.token, method, &path, &[], body).await
        })
    })
    .await
}
