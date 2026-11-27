//! Sports presentation preferences. Opaque selectors confer no authority.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_BODY_BYTES: usize = 60_000;
pub const MAX_SELECTIONS: usize = 256;
pub const MAX_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub schema: u32,
    pub sports: BTreeSet<String>,
    pub leagues: BTreeSet<String>,
    pub conferences: BTreeSet<String>,
    pub teams: BTreeSet<String>,
    pub players: BTreeSet<String>,
    pub all_conferences: bool,
    pub all_teams: bool,
    pub all_players: bool,
    pub home: bool,
    pub away: bool,
    pub native_home: bool,
    pub auto_record: bool,
    pub sports_only: bool,
    pub completed: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema: 1,
            sports: BTreeSet::new(),
            leagues: BTreeSet::new(),
            conferences: BTreeSet::new(),
            teams: BTreeSet::new(),
            players: BTreeSet::new(),
            all_conferences: true,
            all_teams: true,
            all_players: true,
            home: true,
            away: true,
            native_home: true,
            auto_record: false,
            sports_only: false,
            completed: false,
        }
    }
}
impl Preferences {
    pub fn is_valid(&self) -> bool {
        self.schema == 1
            && (self.home || self.away)
            && [
                &self.sports,
                &self.leagues,
                &self.conferences,
                &self.teams,
                &self.players,
            ]
            .iter()
            .all(|ids| {
                ids.len() <= MAX_SELECTIONS
                    && ids
                        .iter()
                        .all(|id| !id.is_empty() && id.len() <= MAX_ID_BYTES)
            })
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    pub version: u64,
    pub preferences: Preferences,
}
/// Deliberately contains no verified event or provider identity.
#[derive(Debug, Clone, Serialize)]
pub struct LocalBroadcast {
    pub kind: &'static str,
    pub item_id: String,
    pub channel_id: String,
    pub channel_name: Option<String>,
    pub native_title: Option<String>,
    pub native_start: Option<String>,
    pub native_end: Option<String>,
    pub provenance: &'static str,
    pub playback_verified: bool,
}
#[derive(Debug, Serialize)]
pub struct Home {
    pub schema: u32,
    pub sports_only: bool,
    pub games: Vec<LocalBroadcast>,
    pub local_broadcasts: Vec<LocalBroadcast>,
    pub presentation_only: bool,
    pub providers: Vec<Provider>,
}
#[derive(Debug, Serialize)]
pub struct Provider {
    pub name: String,
    pub url: String,
    pub kind: String,
    pub access: String,
    pub action: String,
}

/// Common metadata for an authenticated view of public provider data.
/// Favorites and local playback authorization remain separate account state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed<T> {
    pub schema: u32,
    pub provider: String,
    pub fetched_at: String,
    pub stale: bool,
    pub league: String,
    #[serde(flatten)]
    pub data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scoreboard {
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDetail {
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Standings {
    pub groups: Vec<StandingsGroup>,
}

/// Provider team identity. Logo is an authenticated same-origin proxy path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: String,
    pub name: String,
    pub short_name: Option<String>,
    pub abbreviation: Option<String>,
    pub logo: Option<String>,
    /// Six hexadecimal RGB digits without a leading hash, if supplied.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Competitor {
    #[serde(flatten)]
    pub team: Team,
    pub score: Option<String>,
    pub record: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventStatus {
    pub state: Option<String>,
    pub detail: Option<String>,
    pub period: Option<i32>,
    pub clock: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub league: String,
    pub name: String,
    pub short_name: Option<String>,
    pub date: Option<String>,
    pub status: EventStatus,
    pub home: Competitor,
    pub away: Competitor,
    pub venue: Option<String>,
    pub broadcasts: Vec<String>,
    pub stats: Vec<EventStat>,
    pub plays: Vec<Play>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventStat {
    pub name: String,
    pub label: String,
    pub home: Option<String>,
    pub away: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Play {
    pub id: String,
    pub text: String,
    pub period: Option<i32>,
    pub clock: Option<String>,
    pub home_score: Option<String>,
    pub away_score: Option<String>,
    pub scoring: bool,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandingsGroup {
    pub id: String,
    pub name: String,
    pub entries: Vec<Standing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Standing {
    pub team: Team,
    /// Only an explicit provider rank/seed; never inferred from array position.
    pub rank: Option<i32>,
    /// Original provider stat names and displayed values, including missing keys.
    pub stats: std::collections::BTreeMap<String, String>,
}
