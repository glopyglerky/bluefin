//! Optional server-owned discovery. Native clients depend only on wire types.
pub mod config;
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
#[derive(Clone, Copy)]
pub struct Viewer {
    pub user_id: Uuid,
    pub administrator: bool,
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
}

impl DiscoveryService {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            path: path.to_owned(),
            config: RwLock::new(config::load(path)?),
            saves: Mutex::new(()),
            tickets: Mutex::new(HashMap::new()),
            submissions: Mutex::new(()),
            network: Semaphore::new(8),
            upstream: upstream::Upstream::new()?,
        }))
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
        viewer: Viewer,
        audience: Audience,
        request: bool,
    ) -> Result<ViewerGrant> {
        if !config.enabled || (audience == Audience::Client && !config.expose_to_clients) {
            return Err(Error::Unavailable);
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
        Self::grant(&config, viewer, audience, false)
            .ok()
            .map(|g| DiscoveryCapabilities {
                search: true,
                requests: g.requests,
            })
    }

    async fn ticket(&self, id: Uuid, viewer: Viewer, audience: Audience) -> Result<Ticket> {
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
        let grant = Self::grant(&config, viewer, audience, false)?;
        let permissions = self
            .upstream
            .identity(&config, &grant, viewer.user_id, viewer.administrator)
            .await?;
        let response = self
            .upstream
            .search(&config, &grant, query.trim(), page)
            .await?;
        let entries = response["results"]
            .as_array()
            .filter(|a| a.len() <= 100)
            .ok_or(Error::Upstream)?;
        let current = self.config.read().await;
        if current.revision != config.revision {
            return Err(Error::Conflict);
        }
        Self::grant(&current, viewer, audience, false)?;
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
            let movie = match entry["mediaType"]
                .as_str()
                .or_else(|| entry["media_type"].as_str())
            {
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
        let ticket = self.ticket(id, viewer, audience).await?;
        let config = self.config.read().await.clone();
        let grant = Self::grant(&config, viewer, audience, true)?;
        let permissions = self
            .upstream
            .identity(&config, &grant, viewer.user_id, viewer.administrator)
            .await?;
        if !upstream::can_request(permissions, ticket.movie) {
            return Err(Error::Forbidden);
        }
        let seasons = if ticket.movie {
            Vec::new()
        } else {
            self.upstream
                .seasons(&config, &grant, ticket.upstream_id)
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
        let ticket = self.ticket(input.action_id, viewer, audience).await?;
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
        let grant = Self::grant(&config, viewer, audience, true)?;
        let permissions = self
            .upstream
            .identity(&config, &grant, viewer.user_id, viewer.administrator)
            .await?;
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
            .submit(&config, &grant, ticket.upstream_id, ticket.movie, seasons)
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
