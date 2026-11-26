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
