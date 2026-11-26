//! Account-owned catalog queries shared by the canvas and semantic browser UI.
use crate::protocol::{self, Item, Result, Session};
use serde_json::{Value, json};
use uuid::Uuid;

pub const PAGE_SIZE: usize = 60;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    All,
    TvShows,
    Movies,
    Music,
    LiveTv,
}
impl SearchScope {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::TvShows,
        Self::Movies,
        Self::Music,
        Self::LiveTv,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::All => "All Media",
            Self::TvShows => "TV Shows",
            Self::Movies => "Movies",
            Self::Music => "Music",
            Self::LiveTv => "Live TV",
        }
    }
    pub fn types(self) -> &'static str {
        match self {
            Self::All => "",
            Self::TvShows => "Series,Season,Episode",
            Self::Movies => "Movie",
            Self::Music => "Audio,MusicAlbum,MusicArtist,MusicVideo,Playlist",
            Self::LiveTv => "TvChannel,LiveTvProgram",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub scope: SearchScope,
    pub studio: Option<Uuid>,
    pub tag: String,
    pub collection: Option<Uuid>,
}
impl SearchFilters {
    pub fn active(&self) -> bool {
        self.studio.is_some() || !self.tag.is_empty() || self.collection.is_some()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facet {
    pub id: Uuid,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct Facets {
    pub studios: Vec<Facet>,
    pub tags: Vec<String>,
    pub collections: Vec<Facet>,
    pub truncated: bool,
    pub warning: String,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub path: String,
    pub query: Vec<(&'static str, String)>,
}
impl Request {
    pub fn items(user: Uuid, types: &str, parent: Option<Uuid>) -> Self {
        let mut query = vec![
            ("UserId", user.to_string()),
            ("Recursive", "true".into()),
            ("Fields", "Overview,RunTimeTicks".into()),
        ];
        if !types.is_empty() {
            query.push(("IncludeItemTypes", types.into()));
        }
        if let Some(parent) = parent {
            query.push(("ParentId", parent.to_string()));
        }
        Self {
            path: "Items".into(),
            query,
        }
    }
    pub fn children(user: Uuid, item: &Item) -> Self {
        if item.kind == "MusicArtist" {
            let mut request = Self::items(user, "MusicAlbum,Audio,MusicVideo", None);
            request.query.push(("ArtistIds", item.id.to_string()));
            request
        } else {
            Self::items(
                user,
                match item.kind.as_str() {
                    "Series" | "Season" => "Episode",
                    "MusicAlbum" | "Playlist" => "Audio",
                    _ => "",
                },
                Some(item.id),
            )
        }
    }
    pub fn page_query(&self, start: usize) -> Vec<(&'static str, String)> {
        let mut query = self.query.clone();
        query.retain(|(key, _)| !matches!(*key, "Limit" | "StartIndex"));
        query.extend([
            ("Limit", PAGE_SIZE.to_string()),
            ("StartIndex", start.to_string()),
        ]);
        query
    }
}
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: usize,
    pub total: usize,
}
impl Page {
    pub fn more(&self) -> bool {
        self.next < self.total
    }
    pub fn append(&mut self, page: Self) {
        let mut seen: std::collections::HashSet<_> =
            self.items.iter().map(|item| item.id).collect();
        self.items
            .extend(page.items.into_iter().filter(|item| seen.insert(item.id)));
        self.next = page.next;
        self.total = page.total;
    }
    pub fn decode(value: Value, start: usize) -> Result<Self> {
        let total = value.get("TotalRecordCount").and_then(Value::as_u64);
        let items = protocol::items(value)?;
        let next = start.saturating_add(items.len());
        // Advance by rows consumed, not unique IDs. An empty page terminates a
        // stale total so the user cannot get stuck requesting the same page.
        let total = if items.is_empty() {
            next
        } else {
            total.map_or(next, |n| (n as usize).max(next))
        };
        Ok(Self { items, next, total })
    }
}
pub async fn page(session: &Session, request: &Request, start: usize) -> Result<Page> {
    Page::decode(
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            &request.path,
            &request.page_query(start),
            None,
        )
        .await?,
        start,
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Library,
    Artists,
    Programs,
    Channels,
}
impl Lane {
    pub fn title(self) -> &'static str {
        match self {
            Self::Library => "Your library",
            Self::Artists => "Artists",
            Self::Programs => "On now and upcoming",
            Self::Channels => "Channels",
        }
    }
}
pub fn search_requests(user: Uuid, term: &str, filters: &SearchFilters) -> Vec<(Lane, Request)> {
    let mut lanes = vec![Lane::Library];
    if !filters.active() {
        match filters.scope {
            SearchScope::All => lanes.extend([Lane::Artists, Lane::Programs, Lane::Channels]),
            SearchScope::Music => lanes.push(Lane::Artists),
            SearchScope::LiveTv => lanes = vec![Lane::Programs, Lane::Channels],
            _ => {}
        }
    }
    lanes
        .into_iter()
        .map(|lane| {
            let mut request = Request::items(user, filters.scope.types(), filters.collection);
            if lane != Lane::Library {
                request.path = match lane {
                    Lane::Artists => "Artists",
                    Lane::Programs => "LiveTv/Programs",
                    Lane::Channels => "LiveTv/Channels",
                    _ => unreachable!(),
                }
                .into();
                request
                    .query
                    .retain(|(key, _)| !matches!(*key, "IncludeItemTypes" | "ParentId"));
            } else {
                if let Some(studio) = filters.studio {
                    request.query.push(("StudioIds", studio.to_string()));
                }
                if !filters.tag.is_empty() {
                    request.query.push(("Tags", filters.tag.clone()));
                }
            }
            if !term.trim().is_empty() {
                request
                    .query
                    .push(("SearchTerm", term.trim().chars().take(256).collect()));
            }
            (lane, request)
        })
        .collect()
}
#[derive(Clone, Debug)]
pub struct SearchShelf {
    pub lane: Lane,
    pub request: Request,
    pub page: Page,
    pub warning: String,
}
#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub shelves: Vec<SearchShelf>,
}
pub async fn search(
    session: &Session,
    term: &str,
    filters: &SearchFilters,
) -> Result<SearchResults> {
    if term.trim().is_empty() && filters.scope == SearchScope::All && !filters.active() {
        return Ok(SearchResults::default());
    }
    let requests = search_requests(session.user, term, filters);
    let pages = futures_util::future::join_all(
        requests
            .iter()
            .map(|(_, request)| page(session, request, 0)),
    )
    .await;
    let mut shelves = vec![];
    for ((lane, request), result) in requests.into_iter().zip(pages) {
        let (page, warning) = match result {
            Ok(page) => (page, String::new()),
            Err(error) if access_denied(&error) => return Err(error),
            Err(_) => (Page::default(), "This source could not refresh.".into()),
        };
        shelves.push(SearchShelf {
            lane,
            request,
            page,
            warning,
        });
    }
    Ok(SearchResults { shelves })
}
fn access_denied(error: &str) -> bool {
    error.starts_with("Account access denied")
}
fn missing(error: &str) -> bool {
    matches!(
        error,
        "Server returned HTTP 404" | "The peer returned HTTP 404."
    )
}
fn facets_from_items(value: Value) -> Result<(Vec<Facet>, bool)> {
    let total = value["TotalRecordCount"].as_u64().unwrap_or(0);
    let items = protocol::items(value)?;
    let truncated = total > items.len() as u64;
    Ok((
        items
            .into_iter()
            .map(|item| Facet {
                id: item.id,
                name: item.name,
            })
            .collect(),
        truncated,
    ))
}
pub async fn facets(session: &Session) -> Result<Facets> {
    let user = session.user.to_string();
    let studio_query = [("UserId", user.clone()), ("Limit", "200".into())];
    let tags_query = [("UserId", user.clone())];
    let collection_query = [
        ("UserId", user),
        ("Recursive", "true".into()),
        ("IncludeItemTypes", "BoxSet".into()),
        ("Limit", "200".into()),
    ];
    let (studios, tags, collections) = futures_util::join!(
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            "Studios",
            &studio_query,
            None
        ),
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            "Items/Filters2",
            &tags_query,
            None
        ),
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            "Items",
            &collection_query,
            None
        )
    );
    let mut result = Facets::default();
    for error in [&studios, &tags, &collections]
        .into_iter()
        .filter_map(|result| result.as_ref().err())
    {
        if access_denied(error) {
            return Err(error.clone());
        }
        result.warning = "Some refinement choices could not refresh.".into();
    }
    if let Ok(value) = studios {
        let (items, truncated) = facets_from_items(value)?;
        result.studios = items;
        result.truncated |= truncated;
    }
    if let Ok(value) = collections {
        let (items, truncated) = facets_from_items(value)?;
        result.collections = items;
        result.truncated |= truncated;
    }
    if let Ok(value) = tags {
        if let Some(tags) = value["Tags"].as_array() {
            result.truncated |= tags.len() > 512;
            result.tags = tags
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 512)
                .take(512)
                .map(str::to_owned)
                .collect();
        }
    }
    Ok(result)
}
#[derive(Clone, Debug, Default)]
pub struct Shelf {
    pub title: String,
    pub reason: String,
    pub items: Vec<Item>,
}
#[derive(Clone, Debug, Default)]
pub struct Home {
    pub shelves: Vec<Shelf>,
    pub featured: Option<Item>,
    pub history_generation: Option<u64>,
    pub warning: String,
}
fn new_recommendations(value: Value) -> Result<(Vec<Shelf>, Option<u64>)> {
    let generation = (value["searchHistorySupported"] == true)
        .then(|| value["searchHistoryGeneration"].as_u64())
        .flatten();
    let mut shelves = vec![];
    for shelf in value["recommendations"]
        .as_array()
        .ok_or("Invalid Home recommendations")?
        .iter()
        .take(4)
    {
        let title = shelf["title"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 720)
            .ok_or("Invalid Home section")?;
        shelves.push(Shelf {
            title: title.into(),
            reason: shelf["reason"].as_str().unwrap_or("").into(),
            items: protocol::items(shelf["items"].clone())?,
        });
    }
    Ok((shelves, generation))
}
fn legacy_recommendations(value: Value) -> Result<(Vec<Shelf>, Option<u64>)> {
    let mut shelves = vec![];
    for shelf in value
        .as_array()
        .ok_or("Invalid recommendations")?
        .iter()
        .take(4)
    {
        let prefix = match shelf["RecommendationType"].as_str().unwrap_or("") {
            "SimilarToRecentlyPlayed" => "Because you watched",
            "SimilarToLikedItem" => "Because you like",
            "HasDirectorFromRecentlyPlayed" => "More from",
            "HasActorFromRecentlyPlayed" => "More with",
            "HasLikedDirector" => "From a director you like:",
            "HasLikedActor" => "With an actor you like:",
            _ => continue,
        };
        let baseline = shelf["BaselineItemName"].as_str().unwrap_or("");
        if baseline.is_empty() {
            continue;
        }
        shelves.push(Shelf {
            title: format!(
                "{prefix} {}",
                baseline.chars().take(128).collect::<String>()
            ),
            reason: "watching".into(),
            items: protocol::items(shelf["Items"].clone())?,
        });
    }
    Ok((shelves, None))
}
async fn recommendations(session: &Session) -> Result<(Vec<Shelf>, Option<u64>)> {
    match protocol::request(
        &session.base,
        &session.token,
        "GET",
        "Bluefin/Home/Recommendations",
        &[],
        None,
    )
    .await
    {
        Ok(value) => new_recommendations(value),
        Err(error) if missing(&error) => legacy_recommendations(
            protocol::request(
                &session.base,
                &session.token,
                "GET",
                "Movies/Recommendations",
                &[
                    ("UserId", session.user.to_string()),
                    ("CategoryLimit", "4".into()),
                    ("ItemLimit", "12".into()),
                    ("Fields", "Overview,RunTimeTicks".into()),
                ],
                None,
            )
            .await?,
        ),
        Err(error) => Err(error),
    }
}
pub async fn home(session: &Session) -> Result<Home> {
    let user = session.user.to_string();
    let query = [
        ("Limit", "18".into()),
        ("Fields", "Overview,RunTimeTicks".into()),
    ];
    let resume_path = format!("Users/{user}/Items/Resume");
    let latest_path = format!("Users/{user}/Items/Latest");
    let next_query = [
        ("UserId", user.clone()),
        ("Limit", "18".into()),
        ("Fields", "Overview,RunTimeTicks".into()),
    ];
    let (resume, next, latest, recommendations) = futures_util::join!(
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            &resume_path,
            &query,
            None
        ),
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            "Shows/NextUp",
            &next_query,
            None
        ),
        protocol::request(
            &session.base,
            &session.token,
            "GET",
            &latest_path,
            &query,
            None
        ),
        recommendations(session)
    );
    let mut result = Home::default();
    for (title, value) in [
        ("Continue watching", resume),
        ("Up next", next),
        ("Recently added", latest),
    ] {
        match value.and_then(protocol::items) {
            Ok(items) => result.shelves.push(Shelf {
                title: title.into(),
                items,
                ..Default::default()
            }),
            Err(error) if access_denied(&error) => return Err(error),
            Err(_) => result.warning = "Some Home sections could not refresh.".into(),
        }
    }
    match recommendations {
        Ok((shelves, generation)) => {
            result.history_generation = generation;
            let insert = result
                .shelves
                .iter()
                .position(|s| s.title == "Recently added")
                .unwrap_or(result.shelves.len());
            result.shelves.splice(insert..insert, shelves);
        }
        Err(error) if access_denied(&error) => return Err(error),
        Err(_) => result.warning = "Some Home sections could not refresh.".into(),
    }
    result.shelves.retain(|shelf| !shelf.items.is_empty());
    result.featured = result
        .shelves
        .iter()
        .find_map(|shelf| shelf.items.first())
        .cloned();
    Ok(result)
}
pub async fn sports(session: &Session) -> Result<Page> {
    let result = protocol::request(
        &session.base,
        &session.token,
        "GET",
        "sports/api/v1/home",
        &[],
        None,
    )
    .await;
    match result {
        Err(error) if missing(&error) => {
            page(
                session,
                &Request {
                    path: "LiveTv/Programs".into(),
                    query: vec![
                        ("UserId", session.user.to_string()),
                        ("IsSports", "true".into()),
                    ],
                },
                0,
            )
            .await
        }
        Err(error) => Err(error),
        Ok(value) => {
            let mut items = vec![];
            for entry in ["games", "local_broadcasts"]
                .into_iter()
                .flat_map(|key| value[key].as_array().into_iter().flatten())
                .take(200)
            {
                let Some(channel) = entry["channel_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .filter(|id| !id.is_nil())
                else {
                    continue;
                };
                let name = entry["event"]["name"]
                    .as_str()
                    .or(entry["title"].as_str())
                    .unwrap_or("Sports broadcast");
                if !items.iter().any(|item: &Item| item.id == channel) {
                    items.push(Item {
                        id: channel,
                        kind: "TvChannel".into(),
                        name: name.chars().take(512).collect(),
                        ..Default::default()
                    });
                }
            }
            let total = items.len();
            Ok(Page {
                items,
                next: total,
                total,
            })
        }
    }
}
pub async fn record_search(
    session: &Session,
    generation: u64,
    query: &str,
    selected: Option<Uuid>,
) {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if !(2..=128).contains(&query.chars().count()) || query.chars().any(char::is_control) {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    let occurred = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64);
    #[cfg(target_arch = "wasm32")]
    let occurred = js_sys::Date::now() as u64;
    let mut event = json!({"generation":generation, "query":query, "eventId":Uuid::new_v4(), "occurredAtUnixMs":occurred});
    if let Some(item) = selected {
        event["selectedItemId"] = json!(item);
    }
    // A stale generation is deliberately not retried after history is cleared.
    let _ = protocol::request(
        &session.base,
        &session.token,
        "POST",
        "Bluefin/Home/Searches",
        &[],
        Some(event),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_and_facets_are_applied_to_every_server_page() {
        let user = Uuid::from_u128(1);
        let studio = Uuid::from_u128(2);
        let collection = Uuid::from_u128(3);
        let filters = SearchFilters {
            scope: SearchScope::TvShows,
            studio: Some(studio),
            tag: "My service / favorites".into(),
            collection: Some(collection),
        };
        let requests = search_requests(user, "harbor", &filters);
        assert_eq!(requests.len(), 1);
        for offset in [0, 60, 120] {
            let query = requests[0].1.page_query(offset);
            for (key, value) in [
                ("SearchTerm", "harbor".to_owned()),
                ("IncludeItemTypes", "Series,Season,Episode".into()),
                ("StudioIds", studio.to_string()),
                ("ParentId", collection.to_string()),
                ("Tags", filters.tag.clone()),
                ("StartIndex", offset.to_string()),
            ] {
                assert!(query.contains(&(key, value)));
            }
        }
        let broad = search_requests(user, "harbor", &SearchFilters::default());
        assert_eq!(
            broad.iter().map(|(lane, _)| *lane).collect::<Vec<_>>(),
            [Lane::Library, Lane::Artists, Lane::Programs, Lane::Channels]
        );
        assert!(
            !broad[0]
                .1
                .query
                .iter()
                .any(|(key, _)| *key == "IncludeItemTypes")
        );
    }
    #[test]
    fn artist_navigation_uses_credits_and_preserves_them_across_pages() {
        let item = Item {
            id: Uuid::from_u128(7),
            kind: "MusicArtist".into(),
            ..Default::default()
        };
        let request = Request::children(Uuid::from_u128(1), &item);
        for start in [0, 60] {
            let query = request.page_query(start);
            assert!(query.contains(&("ArtistIds", item.id.to_string())));
            assert!(!query.iter().any(|(key, _)| *key == "ParentId"));
            assert!(query.contains(&("StartIndex", start.to_string())));
        }
    }
    #[test]
    fn duplicate_rows_advance_cursor_and_an_empty_page_stops() {
        let item = json!({"Id": Uuid::from_u128(1), "Name":"Real item", "Type":"Movie"});
        let mut page =
            Page::decode(json!({"Items":[item.clone()],"TotalRecordCount":3}), 0).unwrap();
        page.append(
            Page::decode(json!({"Items":[item.clone(),item],"TotalRecordCount":4}), 1).unwrap(),
        );
        assert_eq!((page.items.len(), page.next, page.total), (1, 3, 4));
        page.append(Page::decode(json!({"Items":[],"TotalRecordCount":4}), 3).unwrap());
        assert!(!page.more());
    }
    #[test]
    fn fallback_is_only_for_an_absent_extension() {
        assert!(missing("Server returned HTTP 404"));
        assert!(missing("The peer returned HTTP 404."));
        for error in [
            "Server returned HTTP 403",
            "Server returned HTTP 500",
            "Account access denied. Sign in again.",
            "Connection failed",
        ] {
            assert!(!missing(error));
        }
        let (shelves, generation) = new_recommendations(
            json!({"recommendations":[],"searchHistorySupported":true,"searchHistoryGeneration":7}),
        )
        .unwrap();
        assert!(shelves.is_empty());
        assert_eq!(generation, Some(7));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod http_tests {
    use super::*;
    #[test]
    fn only_http_404_retries_the_legacy_recommendation_route() {
        for status in [404, 403, 503] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let base = url::Url::parse(&format!("http://{}/", server.server_addr())).unwrap();
            let worker = std::thread::spawn(move || {
                let first = server
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap()
                    .unwrap();
                assert_eq!(first.url(), "/Bluefin/Home/Recommendations");
                first.respond(tiny_http::Response::empty(status)).unwrap();
                if status == 404 {
                    let second = server
                        .recv_timeout(std::time::Duration::from_secs(3))
                        .unwrap()
                        .unwrap();
                    assert!(second.url().starts_with("/Movies/Recommendations?"));
                    assert!(
                        second
                            .headers()
                            .iter()
                            .any(|header| header.field.equiv("Authorization")
                                && header.value.as_str().contains(r#"Token="catalog-test""#))
                    );
                    second
                        .respond(tiny_http::Response::from_string("[]"))
                        .unwrap();
                }
            });
            let session = Session {
                base,
                token: "catalog-test".into(),
                user: Uuid::from_u128(1),
            };
            let result = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(recommendations(&session));
            assert_eq!(result.is_ok(), status == 404);
            worker.join().unwrap();
        }
    }
}
