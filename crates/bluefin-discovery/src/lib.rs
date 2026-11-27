//! Optional server-owned discovery. Native clients depend only on wire types.
pub mod config;
/// Public catalog access and regional offer adapters.
pub mod catalog;
/// Portable catalog request and response contracts.
pub use bluefin_catalog as catalog_types;
mod upstream;

/// Sanitized errors. Provider response bodies, addresses and secrets stay private.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum Error {
    #[error("Discovery is unavailable")]
    Unavailable,
    #[error("This account cannot use this discovery action")]
    Forbidden,
    #[error("Invalid discovery input")]
    Invalid,
    #[error("Discovery changed; reload and try again")]
    Conflict,
    #[error("The discovery action expired; search again")]
    Expired,
    #[error("The connected catalog could not complete this operation")]
    Upstream,
    #[error("The request result is uncertain; check its status before requesting again")]
    Uncertain,
    #[error("Discovery is busy; try again shortly")]
    Busy,
    #[error("This title has already been requested")]
    AlreadyRequested,
    #[error("Discovery settings could not be saved; reload before retrying")]
    Storage,
}

pub type Result<T> = std::result::Result<T, Error>;

use bluefin_contract::discovery::{
    ActionChoice, ActionForm, ActionReceipt, DiscoveryAction, DiscoveryCapabilities, DiscoveryItem,
    DiscoveryPage, SubmitAction,
};
use bluefin_contract::search::SearchScope;
use config::{SaveSettings, SettingsSnapshot, StoredSettings, ViewerGrant};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock, Semaphore};
use uuid::Uuid;

const TICKET_LIFETIME: Duration = Duration::from_secs(600);
const MAX_TICKETS: usize = 2048;

/// Context is chosen by the server route, never from request input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    Client,
    Web,
}

/// Authenticated identity projected by the HTTP layer, never decoded from JSON.
#[derive(Clone)]
pub struct Viewer {
    pub user_id: Uuid,
    pub administrator: bool,
    session_authorization: Option<reqwest::header::HeaderValue>,
}

impl Viewer {
    pub fn new(user_id: Uuid, administrator: bool) -> Self {
        Self {
            user_id,
            administrator,
            session_authorization: None,
        }
    }

    /// Give the local catalog adapter the same authenticated session as the caller.
    /// The header lives only for this operation and is never retained in tickets.
    pub fn with_session_token(mut self, token: &str) -> Self {
        if !token.is_empty()
            && token.len() <= 512
            && token
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
            && let Ok(mut value) =
                reqwest::header::HeaderValue::from_str(&format!("MediaBrowser Token=\"{token}\""))
        {
            value.set_sensitive(true);
            self.session_authorization = Some(value);
        }
        self
    }
}

#[derive(Clone)]
struct Ticket {
    owner: Uuid,
    audience: Audience,
    revision: u64,
    created: Instant,
    upstream_id: u64,
    movie: bool,
    title: String,
    seasons: Option<Vec<u32>>,
    state: TicketState,
}

#[derive(Clone)]
enum TicketState {
    Ready,
    InFlight,
    Completed(ActionReceipt),
    Uncertain,
}

/// One server process owns the private configuration file and short-lived actions.
pub struct DiscoveryService {
    path: PathBuf,
    config: RwLock<StoredSettings>,
    saves: Mutex<()>,
    tickets: Mutex<HashMap<Uuid, Ticket>>,
    submissions: Mutex<()>,
    network: Semaphore,
    upstream: upstream::Upstream,
    catalog: Option<Arc<catalog::CatalogService>>,
}

impl DiscoveryService {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        // Optional public configuration never replaces private discovery authority.
        let catalog = catalog::CatalogService::open(&path.with_file_name("bluefin-catalog.json"))
            .ok()
            .map(Arc::new);
        Self::open_with_catalog(path, catalog)
    }

    /// Uses the process-owned public reader while loading private discovery grants normally.
    pub fn open_with_catalog(
        path: &Path,
        catalog: Option<Arc<catalog::CatalogService>>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            path: path.to_owned(),
            config: RwLock::new(config::load(path)?),
            saves: Mutex::new(()),
            tickets: Mutex::new(HashMap::new()),
            submissions: Mutex::new(()),
            network: Semaphore::new(8),
            upstream: upstream::Upstream::new()?,
            catalog,
        }))
    }

    /// Public catalog reads are independent of optional library/request grants.
    /// Invalid optional configuration disables this adapter alone.
    pub fn catalog(&self) -> Option<&Arc<catalog::CatalogService>> {
        self.catalog.as_ref()
    }

    pub async fn settings(&self) -> SettingsSnapshot {
        self.config.read().await.snapshot()
    }

    /// Detached ownership keeps disk and memory updates together if HTTP is cancelled.
    pub async fn save(self: &Arc<Self>, update: SaveSettings) -> Result<SettingsSnapshot> {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let _save = service.saves.lock().await;
            let next = service.config.read().await.replace(update)?;
            let copy = next.clone();
            let path = service.path.clone();
            let directory_synced =
                tokio::task::spawn_blocking(move || config::persist(&path, &copy))
                    .await
                    .map_err(|_| Error::Storage)??;
            let snapshot = next.snapshot();
            // Hold the write lock through ticket invalidation. Requests already
            // admitted finish before disabling returns; subsequent calls are denied.
            let mut current = service.config.write().await;
            *current = next;
            service.tickets.lock().await.clear();
            if !directory_synced {
                return Err(Error::Storage);
            }
            Ok(snapshot)
        })
        .await
        .map_err(|_| Error::Storage)?
    }

    fn grant(
        config: &StoredSettings,
        viewer: &Viewer,
        audience: Audience,
        request: bool,
    ) -> Result<ViewerGrant> {
        if !config.enabled || (audience == Audience::Client && !config.expose_to_clients) {
            return Err(Error::Unavailable);
        }
        if config.session_catalog && viewer.session_authorization.is_none() {
            return Err(Error::Forbidden);
        }
        config
            .viewers
            .iter()
            .find(|g| g.user_id == viewer.user_id && g.search && (!request || g.requests))
            .cloned()
            .ok_or(Error::Forbidden)
    }

    pub async fn capabilities(
        &self,
        viewer: Viewer,
        audience: Audience,
    ) -> Option<DiscoveryCapabilities> {
        let config = self.config.read().await;
        Self::grant(&config, &viewer, audience, false)
            .ok()
            .map(|g| DiscoveryCapabilities {
                search: true,
                requests: g.requests,
            })
    }

    async fn ticket(&self, id: Uuid, viewer: &Viewer, audience: Audience) -> Result<Ticket> {
        let config = self.config.read().await;
        Self::grant(&config, viewer, audience, true)?;
        self.tickets
            .lock()
            .await
            .get(&id)
            .filter(|t| {
                t.owner == viewer.user_id
                    && t.audience == audience
                    && t.revision == config.revision
                    && t.created.elapsed() < TICKET_LIFETIME
            })
            .cloned()
            .ok_or(Error::Expired)
    }

    pub async fn search(
        &self,
        viewer: Viewer,
        audience: Audience,
        query: &str,
        page: u32,
    ) -> Result<DiscoveryPage> {
        self.search_scope(viewer, audience, query, page, SearchScope::All)
            .await
    }

    pub async fn search_scope(
        &self,
        viewer: Viewer,
        audience: Audience,
        query: &str,
        page: u32,
        scope: SearchScope,
    ) -> Result<DiscoveryPage> {
        if viewer.user_id.is_nil()
            || query.trim().is_empty()
            || query.chars().count() > 160
            || query.chars().any(char::is_control)
            || !(1..=1000).contains(&page)
        {
            return Err(Error::Invalid);
        }
        let _network = self.network.try_acquire().map_err(|_| Error::Busy)?;
        let config = self.config.read().await.clone();
        let grant = Self::grant(&config, &viewer, audience, false)?;
        if scope == SearchScope::LiveTv || (scope == SearchScope::Music && !config.session_catalog)
        {
            return Err(Error::Unavailable);
        }
        let permissions = self.upstream.identity(&config, &grant, &viewer).await?;
        if scope == SearchScope::Music {
            let results = if page == 1 {
                let response = self
                    .upstream
                    .music(&config, &grant, &viewer, query.trim())
                    .await?;
                project_music(&response)?
            } else {
                Vec::new()
            };
            if self.config.read().await.revision != config.revision {
                return Err(Error::Conflict);
            }
            return Ok(DiscoveryPage {
                results,
                page,
                total_pages: 1,
            });
        }
        let response = self
            .upstream
            .search(&config, &grant, &viewer, query.trim(), page, scope)
            .await?;
        let entries = response["results"]
            .as_array()
            .filter(|a| a.len() <= 100)
            .ok_or(Error::Upstream)?;
        let current = self.config.read().await;
        if current.revision != config.revision {
            return Err(Error::Conflict);
        }
        Self::grant(&current, &viewer, audience, false)?;
        let mut tickets = self.tickets.lock().await;
        tickets.retain(|_, t| t.created.elapsed() < TICKET_LIFETIME);
        // Per-viewer and global bounds prevent retaining an unbounded catalog.
        let mut owned: Vec<_> = tickets
            .iter()
            .filter(|(_, t)| t.owner == viewer.user_id)
            .map(|(id, t)| (*id, t.created))
            .collect();
        owned.sort_by_key(|(_, time)| *time);
        for (id, _) in owned.into_iter().rev().skip(80) {
            tickets.remove(&id);
        }
        if tickets.len() + entries.len() > MAX_TICKETS {
            let mut oldest: Vec<_> = tickets.iter().map(|(id, t)| (*id, t.created)).collect();
            oldest.sort_by_key(|(_, time)| *time);
            let count = (tickets.len() + entries.len()).saturating_sub(MAX_TICKETS);
            for (id, _) in oldest.into_iter().take(count) {
                tickets.remove(&id);
            }
        }
        let mut results = Vec::new();
        for entry in entries {
            let media_type = entry["mediaType"]
                .as_str()
                .or_else(|| entry["media_type"].as_str());
            if (scope == SearchScope::Movies && media_type != Some("movie"))
                || (scope == SearchScope::TvShows && media_type != Some("tv"))
            {
                continue;
            }
            if config.session_catalog && media_type == Some("person") {
                if let Some(name) = entry["name"]
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                {
                    results.push(DiscoveryItem {
                        id: Uuid::new_v4(),
                        title: text(name, 256),
                        overview: text(entry["knownForDepartment"].as_str().unwrap_or(""), 256),
                        year: None,
                        kind: "person".into(),
                        availability: "unknown".into(),
                        actions: Vec::new(),
                    });
                }
                continue;
            }
            let movie = match media_type {
                Some("movie") => true,
                Some("tv") => false,
                _ => continue,
            };
            let Some(upstream_id) = entry["id"]
                .as_u64()
                .filter(|id| (1..=i32::MAX as u64).contains(id))
            else {
                continue;
            };
            let title = text(
                entry[if movie { "title" } else { "name" }]
                    .as_str()
                    .unwrap_or("Untitled"),
                256,
            );
            let availability = match entry["mediaInfo"]["status"].as_u64() {
                Some(2 | 3) => "requested",
                Some(4) => "partiallyAvailable",
                Some(5) => "available",
                _ => "unknown",
            };
            let id = Uuid::new_v4();
            let mut actions = Vec::new();
            if grant.requests
                && upstream::can_request(permissions, movie)
                && availability != "available"
                && availability != "requested"
            {
                let action = Uuid::new_v4();
                tickets.insert(
                    action,
                    Ticket {
                        owner: viewer.user_id,
                        audience,
                        revision: config.revision,
                        created: Instant::now(),
                        upstream_id,
                        movie,
                        title: title.clone(),
                        seasons: None,
                        state: TicketState::Ready,
                    },
                );
                actions.push(DiscoveryAction {
                    id: action,
                    label: "Request".into(),
                });
            }
            let date = entry[if movie { "releaseDate" } else { "firstAirDate" }]
                .as_str()
                .unwrap_or("");
            let year = date
                .get(..4)
                .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
                .map(str::to_owned);
            results.push(DiscoveryItem {
                id,
                title,
                overview: text(entry["overview"].as_str().unwrap_or(""), 4000),
                year,
                kind: if movie { "movie" } else { "series" }.into(),
                availability: availability.into(),
                actions,
            });
        }
        Ok(DiscoveryPage {
            results,
            page,
            total_pages: response["totalPages"]
                .as_u64()
                .or_else(|| response["total_pages"].as_u64())
                .unwrap_or(u64::from(page))
                .clamp(1, 1000) as u32,
        })
    }

    pub async fn action(&self, viewer: Viewer, audience: Audience, id: Uuid) -> Result<ActionForm> {
        let _network = self.network.try_acquire().map_err(|_| Error::Busy)?;
        let ticket = self.ticket(id, &viewer, audience).await?;
        let config = self.config.read().await.clone();
        let grant = Self::grant(&config, &viewer, audience, true)?;
        let permissions = self.upstream.identity(&config, &grant, &viewer).await?;
        if !upstream::can_request(permissions, ticket.movie) {
            return Err(Error::Forbidden);
        }
        let seasons = if ticket.movie {
            Vec::new()
        } else {
            self.upstream
                .seasons(&config, &grant, &viewer, ticket.upstream_id)
                .await?
        };
        let current = self.config.read().await;
        if current.revision != ticket.revision {
            return Err(Error::Conflict);
        }
        let mut tickets = self.tickets.lock().await;
        let stored = tickets.get_mut(&id).ok_or(Error::Expired)?;
        stored.seasons = Some(seasons.clone());
        Ok(ActionForm { id, title: ticket.title,
            notice: "Request only media you are authorized to access. The connected service applies its own permissions, quotas and approval rules.".into(),
            choices: seasons.into_iter().map(|n| ActionChoice { id: n.to_string(), label: if n == 0 { "Specials".into() } else { format!("Season {n}") } }).collect(),
            requires_selection: !ticket.movie })
    }

    pub async fn submit(
        self: &Arc<Self>,
        viewer: Viewer,
        audience: Audience,
        input: SubmitAction,
    ) -> Result<ActionReceipt> {
        if !input.authorized_use_confirmed || input.selections.len() > 256 {
            return Err(Error::Invalid);
        }
        let service = Arc::clone(self);
        // Once admitted, disconnecting HTTP cannot leave a second submission
        // racing an unknown first result. Never automatically retry POST.
        tokio::spawn(async move { service.submit_owned(viewer, audience, input).await })
            .await
            .map_err(|_| Error::Uncertain)?
    }

    async fn submit_owned(
        &self,
        viewer: Viewer,
        audience: Audience,
        input: SubmitAction,
    ) -> Result<ActionReceipt> {
        let _submission = self.submissions.try_lock().map_err(|_| Error::Busy)?;
        let _network = self.network.try_acquire().map_err(|_| Error::Busy)?;
        let ticket = self.ticket(input.action_id, &viewer, audience).await?;
        match ticket.state {
            TicketState::Completed(receipt) => return Ok(receipt),
            TicketState::InFlight => return Err(Error::Busy),
            TicketState::Uncertain => return Err(Error::Uncertain),
            TicketState::Ready => {}
        }
        let selections: HashSet<u32> = input
            .selections
            .iter()
            .map(|s| s.parse().map_err(|_| Error::Invalid))
            .collect::<Result<_>>()?;
        if selections.len() != input.selections.len()
            || (ticket.movie && !selections.is_empty())
            || (!ticket.movie
                && (selections.is_empty()
                    || ticket
                        .seasons
                        .as_ref()
                        .is_none_or(|allowed| !selections.iter().all(|n| allowed.contains(n)))))
        {
            return Err(Error::Invalid);
        }
        let config = self.config.read().await;
        if config.revision != ticket.revision {
            return Err(Error::Conflict);
        }
        let grant = Self::grant(&config, &viewer, audience, true)?;
        let permissions = self.upstream.identity(&config, &grant, &viewer).await?;
        if !upstream::can_request(permissions, ticket.movie) {
            return Err(Error::Forbidden);
        }
        self.tickets
            .lock()
            .await
            .get_mut(&input.action_id)
            .ok_or(Error::Expired)?
            .state = TicketState::InFlight;
        let mut seasons: Vec<_> = selections.into_iter().collect();
        seasons.sort_unstable();
        let result = self
            .upstream
            .submit(
                &config,
                &grant,
                &viewer,
                ticket.upstream_id,
                ticket.movie,
                seasons,
            )
            .await;
        let (state, receipt) = match result {
            Ok(_) => {
                let receipt = ActionReceipt { status: "submitted".into(), message: "Request submitted. The connected service determines approval and availability.".into() };
                (TicketState::Completed(receipt.clone()), Ok(receipt))
            }
            Err(Error::AlreadyRequested) => {
                let receipt = ActionReceipt {
                    status: "alreadyRequested".into(),
                    message: "This title has already been requested.".into(),
                };
                (TicketState::Completed(receipt.clone()), Ok(receipt))
            }
            Err(Error::Forbidden) => (TicketState::Ready, Err(Error::Forbidden)),
            Err(_) => (TicketState::Uncertain, Err(Error::Uncertain)),
        };
        if let Some(ticket) = self.tickets.lock().await.get_mut(&input.action_id) {
            ticket.state = state;
        }
        receipt
    }
}

fn text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(limit)
        .collect()
}

fn project_music(response: &serde_json::Value) -> Result<Vec<DiscoveryItem>> {
    let mut results = Vec::new();
    for (field, kind, title_key) in [
        ("artists", "artist", "name"),
        ("albums", "album", "albumName"),
        ("tracks", "track", "title"),
    ] {
        let entries = response[field].as_array().ok_or(Error::Upstream)?;
        for entry in entries.iter().take(32) {
            let Some(title) = entry[title_key]
                .as_str()
                .filter(|title| !title.trim().is_empty())
            else {
                continue;
            };
            let date = entry["releaseDate"].as_str().unwrap_or("");
            results.push(DiscoveryItem {
                id: Uuid::new_v4(),
                title: text(title, 256),
                overview: text(entry["artistName"].as_str().unwrap_or(""), 256),
                year: date
                    .get(..4)
                    .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()))
                    .map(str::to_owned),
                kind: kind.into(),
                availability: if entry["playbackAvailable"].as_bool() == Some(true) {
                    "available"
                } else {
                    "unknown"
                }
                .into(),
                actions: Vec::new(),
            });
        }
    }
    Ok(results)
}
