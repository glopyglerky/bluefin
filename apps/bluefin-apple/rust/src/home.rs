//! Bounded account-owned Home discovery. All requests use the session's existing
//! authenticated HTTP or mesh route; this module owns no account or history.
use crate::{err, text, Api, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashSet;

const HOME: &[&str] = &["Bluefin", "Home", "Recommendations"];
const SEARCHES: &[&str] = &["Bluefin", "Home", "Searches"];

fn unsupported(error: &str) -> bool {
    matches!(error, "Server returned HTTP 404" | "Server returned HTTP 501"
        | "The server refused this request (HTTP 404)." | "The server refused this request (HTTP 501).")
}

fn guid(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() != 32 && bytes.len() != 36 { return None; }
    if bytes.len() == 36 && [8, 13, 18, 23].iter().any(|i| bytes[*i] != b'-') { return None; }
    let compact: String = value.chars().filter(|c| *c != '-').collect();
    (compact.len() == 32 && compact.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| compact.to_ascii_lowercase())
}
fn item_key(value: &str) -> String { guid(value).unwrap_or_else(|| value.into()) }

fn bounded(mut page: Value) -> Result<Value> {
    let shelves = page["recommendations"].as_array().ok_or_else(|| err("Invalid Home recommendations"))?;
    let mut seen = HashSet::new();
    let mut shelf_ids = HashSet::new();
    let mut bounded = Vec::new();
    for shelf in shelves.iter().take(4) {
        let id = text(shelf, "id")?;
        let title = text(shelf, "title")?;
        if !shelf_ids.insert(id.to_owned()) { continue; }
        let candidates = shelf["items"].as_array().ok_or_else(|| err("Invalid Home shelf"))?;
        let mut items = Vec::new();
        for item in candidates {
            let id = text(item, "Id")?;
            if seen.insert(item_key(id)) { items.push(item.clone()); }
            if items.len() == 12 { break; }
        }
        if !items.is_empty() {
            bounded.push(json!({"id":id,"title":title.chars().take(180).collect::<String>(),
                "items":items,"reason":shelf.get("reason").and_then(Value::as_str)}));
        }
    }
    page["recommendations"] = Value::Array(bounded);
    if !page["searchHistorySupported"].is_boolean() { return Err(err("Invalid Home history support")); }
    if page["searchHistorySupported"] == true && page["searchHistoryGeneration"].as_u64().is_none() {
        return Err(err("Invalid Home history generation"));
    }
    Ok(page)
}

fn legacy(api: &Api<'_>) -> Result<Value> {
    let response = api.get(&["Movies", "Recommendations"], vec![
        ("userId", text(api.v, "userId")?.into()),
        ("categoryLimit", "4".into()), ("itemLimit", "12".into()),
        ("fields", "PrimaryImageAspectRatio,Overview".into()),
    ]);
    let response = match response {
        Err(error) if unsupported(&error) => Value::Array(Vec::new()),
        result => result?,
    };
    let categories = response.as_array().ok_or_else(|| err("Invalid movie recommendations"))?;
    let shelves: Vec<_> = categories.iter().take(4).enumerate().map(|(index, category)| {
        let baseline = category["BaselineItemName"].as_str().unwrap_or("").trim();
        let (prefix, reason) = match category["RecommendationType"].as_str().unwrap_or("") {
            "SimilarToRecentlyPlayed" => ("Because you watched", "watched"),
            "SimilarToLikedItem" => ("Because you like", "liked"),
            "HasDirectorFromRecentlyPlayed" => ("More from", "watched"),
            "HasActorFromRecentlyPlayed" => ("More with", "watched"),
            "HasLikedDirector" => ("From a director you like", "liked"),
            "HasLikedActor" => ("From an actor you like", "liked"),
            _ => ("Picked for you", "recommended"),
        };
        let title = if baseline.is_empty() { prefix.to_owned() } else { format!("{prefix} {baseline}") };
        let id = category["CategoryId"].as_str().map(str::to_owned).unwrap_or_else(|| format!("legacy-{index}"));
        json!({"id":id,"title":title,"reason":reason,"items":category["Items"].as_array().cloned().unwrap_or_default()})
    }).collect();
    bounded(json!({"recommendations":shelves,"searchHistorySupported":false,"searchHistoryGeneration":null}))
}

/// Run an account-owned Home operation through the existing authenticated route.
pub(crate) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    match operation {
        "homeRecommendations" => match api.get(HOME, vec![]) {
            Ok(page) => bounded(page),
            Err(error) if unsupported(&error) => legacy(api),
            Err(error) => Err(error),
        },
        "homeSearchRecord" => {
            let event = api.v.get("event").ok_or_else(|| err("Missing Home search event"))?;
            let event_id = text(event, "eventId")?;
            guid(event_id).ok_or_else(|| err("Invalid search event identifier"))?;
            let query = text(event, "query")?;
            if query.chars().count() > 128 { return Err(err("Search query is too long")); }
            if event["generation"].as_u64().is_none() || event["occurredAtUnixMs"].as_i64().is_none() {
                return Err(err("Invalid search event generation or time"));
            }
            let mut body = json!({"eventId":event_id,"query":query,"generation":event["generation"],
                "occurredAtUnixMs":event["occurredAtUnixMs"]});
            if let Some(id) = event.get("selectedItemId").filter(|id| !id.is_null()) {
                let id = id.as_str().ok_or_else(|| err("Invalid selected item"))?;
                guid(id).ok_or_else(|| err("Invalid selected item"))?;
                body["selectedItemId"] = json!(id);
            }
            api.post(SEARCHES, body)
        }
        "homeSearchClear" => {
            let generation = api.v["generation"].as_u64().ok_or_else(|| err("Invalid history generation"))?;
            api.request(Method::DELETE, SEARCHES, vec![], Some(json!({"generation":generation})), true)
        }
        _ => Err(err("Unknown Home operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::mock;

    #[test]
    fn home_bounded_deduplicates_uuid_spellings_and_keeps_rank_order() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let page = bounded(json!({"searchHistorySupported":false,"recommendations":[
            {"id":"a","title":"Because you watched a film","items":[{"Id":id},{"Id":"second"}]},
            {"id":"b","title":"Because you like a film","items":[{"Id":"12345678123412341234123456789ABC"},{"Id":"third"}]}
        ]})).unwrap();
        assert_eq!(page["recommendations"][0]["items"][0]["Id"], id);
        assert_eq!(page["recommendations"][1]["items"], json!([{"Id":"third"}]));
        let many: Vec<_> = (0..80).map(|i| json!({"Id":format!("item-{i}")})).collect();
        let page = bounded(json!({"searchHistorySupported":false,"recommendations":[
            {"id":"a","title":"For you","items":many}]})).unwrap();
        assert_eq!(page["recommendations"][0]["items"].as_array().unwrap().len(), 12);
    }

    #[test]
    fn home_auth_denial_is_never_downgraded_to_legacy_discovery() {
        for status in ["401 Unauthorized", "403 Forbidden", "500 Internal Server Error"] {
            let (server, request) = mock(status, "{}");
            let value = json!({"operation":"homeRecommendations","server":server,"allowLocalHttp":true,
                "token":"viewer-token","userId":"viewer"});
            let error = crate::call(&value).unwrap_err();
            assert!(error.starts_with("Server returned HTTP "));
            let request = request.join().unwrap();
            assert!(request.starts_with("GET /jellyfin/Bluefin/Home/Recommendations"));
            assert!(request.contains("viewer-token"));
        }
    }

    #[test]
    fn home_search_is_an_explicit_authenticated_event_without_user_selection() {
        let (server, request) = mock("200 OK", r#"{"generation":4}"#);
        let event = json!({"eventId":"12345678-1234-1234-1234-123456789abc","query":"Arrival","generation":4,"occurredAtUnixMs":123,
            "userId":"a-different-user"});
        crate::call(&json!({"operation":"homeSearchRecord","server":server,"allowLocalHttp":true,
            "token":"viewer-token","userId":"viewer","event":event})).unwrap();
        let request = request.join().unwrap();
        assert!(request.starts_with("POST /jellyfin/Bluefin/Home/Searches"));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(body.get("userId").is_none());
        assert_eq!(body["query"], "Arrival");
        assert_eq!(body["eventId"], event["eventId"]);
    }

    #[test]
    fn only_absent_or_unimplemented_endpoints_allow_legacy_fallback() {
        for error in ["Server returned HTTP 404", "Server returned HTTP 501",
            "The server refused this request (HTTP 404)."] { assert!(unsupported(error)); }
        for error in ["Server returned HTTP 403", "Server request failed or timed out",
            "Invalid server JSON", "The server refused this request (HTTP 401)."] { assert!(!unsupported(error)); }
    }
}
