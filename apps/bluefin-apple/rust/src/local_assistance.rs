//! Safe typed local operations reuse the existing JSON bridge; no new FFI.
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use serde::Deserialize;
use serde_json::{Value, json};
use bluefin_local_assist::{AssistanceController, AssistanceInput, Ticket};
use crate::Result;

static OWNER: OnceLock<Mutex<AssistanceController>> = OnceLock::new();
static CLOCK: OnceLock<Instant> = OnceLock::new();

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare { context_generation: String, input: AssistanceInput }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Complete { context_generation: String, ticket: Ticket, model_output: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel { context_generation: String, ticket: Ticket }

pub(crate) fn call(value: &Value, operation: &str) -> Result<Value> {
    let mut fields = value.as_object().ok_or("Invalid local assistance request")?.clone();
    fields.remove("operation");
    // AccountSession fields, URLs and tokens fail this strict typed decode.
    let input = Value::Object(fields);
    let now = u64::try_from(CLOCK.get_or_init(Instant::now).elapsed().as_millis())
        .map_err(|_| "Local assistance clock unavailable")?;
    let mut owner = OWNER.get_or_init(|| Mutex::new(AssistanceController::default()))
        .lock().map_err(|_| "Local assistance unavailable")?;
    match operation {
        "localAssistPrepare" => {
            let request: Prepare = serde_json::from_value(input).map_err(|_| "Invalid local assistance request")?;
            let prepared = owner.prepare(&request.context_generation, request.input, now).map_err(|error| error.to_string())?;
            serde_json::to_value(prepared).map_err(|_| "Local assistance encoding failed".into())
        }
        "localAssistComplete" => {
            let request: Complete = serde_json::from_value(input).map_err(|_| "Invalid local assistance completion")?;
            let result = owner.complete(&request.context_generation, request.ticket,
                request.model_output.as_bytes(), now).map_err(|error| error.to_string())?;
            serde_json::to_value(result).map_err(|_| "Local assistance encoding failed".into())
        }
        "localAssistCancel" => {
            let request: Cancel = serde_json::from_value(input).map_err(|_| "Invalid local assistance cancellation")?;
            let _ = owner.cancel(&request.context_generation, request.ticket);
            Ok(json!({"cancellation_requested":true}))
        }
        _ => Err("Unknown local assistance operation".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_and_network_fields_never_enter_local_model_preparation() {
        let request = json!({"operation":"localAssistPrepare","context_generation":"test-scope",
            "input":{"task":"playback","status":"buffering"},"token":"not-a-model-input","server":"https://example.invalid"});
        assert!(call(&request,"localAssistPrepare").is_err());
    }
    #[test]
    fn read_only_operation_is_reachable_without_an_api_or_account_session() {
        let request = json!({"operation":"localAssistPrepare","context_generation":"local-read-only",
            "input":{"task":"playback","status":"paused"}});
        let prepared = crate::call(&request).unwrap();
        assert_eq!(prepared["provenance"]["on_device"],true);
        assert_eq!(prepared["provenance"]["no_training"],true);
        assert!(prepared["prompt"].as_str().unwrap().contains("paused"));
        let cancel=json!({"operation":"localAssistCancel","context_generation":"local-read-only","ticket":prepared["ticket"]});
        assert_eq!(crate::call(&cancel).unwrap()["cancellation_requested"],true);
    }
}
