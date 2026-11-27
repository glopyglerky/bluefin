//! Normalize bounded primary-provider payloads without inventing missing values.
use super::{Kind, logo_id, logo_url, valid_event_id};
use bluefin_contract::sports::{
    Competitor, Event, EventDetail, EventStat, EventStatus, Play, Scoreboard, Standing, Standings,
    StandingsGroup, Team,
};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug)]
pub(super) struct InvalidFeed;
pub(super) struct Mapped {
    pub payload: Value,
    pub logos: HashMap<String, String>,
}
type Result<T> = std::result::Result<T, InvalidFeed>;

fn text(value: &Value, max: usize) -> Option<String> {
    let value = value.as_str()?.trim();
    (!value.is_empty() && value.len() <= max && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}
fn scalar(value: &Value, max: usize) -> Option<String> {
    text(value, max).or_else(|| {
        value.as_number().and_then(|number| {
            let value = number.to_string();
            (value.len() <= max).then_some(value)
        })
    })
}
fn integer(value: &Value) -> Option<i32> {
    value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .or_else(|| value.as_str()?.parse().ok())
}
fn timestamp(value: &Value) -> Option<String> {
    let raw = text(value, 48)?;
    let date = DateTime::parse_from_rfc3339(&raw)
        .map(|date| date.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(&raw, "%Y-%m-%dT%H:%MZ").map(|date| date.and_utc())
        })
        .ok()?;
    Some(date.to_rfc3339_opts(SecondsFormat::Secs, true))
}
fn team(raw: &Value, logos: &mut HashMap<String, String>) -> Result<Team> {
    let id = scalar(&raw["id"], 20)
        .filter(|id| valid_event_id(id))
        .ok_or(InvalidFeed)?;
    let name = text(&raw["displayName"], 256)
        .or_else(|| text(&raw["name"], 256))
        .ok_or(InvalidFeed)?;
    let logo = raw["logo"]
        .as_str()
        .into_iter()
        .chain(
            raw["logos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry["href"].as_str()),
        )
        .find_map(logo_url)
        .map(|url| {
            let id = logo_id(&url);
            logos.insert(id.clone(), url);
            format!("/sports/api/v1/logos/{id}")
        });
    Ok(Team {
        id,
        name,
        short_name: text(&raw["shortDisplayName"], 128)
            .or_else(|| text(&raw["nickname"], 128))
            .or_else(|| text(&raw["name"], 128)),
        abbreviation: text(&raw["abbreviation"], 24),
        logo,
        color: text(&raw["color"], 6)
            .filter(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit())),
    })
}
fn competitor(raw: &Value, logos: &mut HashMap<String, String>) -> Result<Competitor> {
    let records = raw["records"]
        .as_array()
        .or_else(|| raw["record"].as_array());
    let record = records.and_then(|records| {
        records
            .iter()
            .find(|entry| entry["type"] == "total" || entry["name"] == "overall")
            .or_else(|| records.first())
            .and_then(|entry| text(&entry["summary"], 128))
    });
    Ok(Competitor {
        team: team(&raw["team"], logos)?,
        score: scalar(&raw["score"], 32).or_else(|| scalar(&raw["score"]["displayValue"], 32)),
        record,
    })
}
fn broadcasts(competition: &Value, summary: &Value) -> Vec<String> {
    let mut names = BTreeSet::new();
    for source in [competition, summary] {
        for entry in source["broadcasts"]
            .as_array()
            .into_iter()
            .flatten()
            .take(32)
        {
            for name in entry["names"].as_array().into_iter().flatten().take(16) {
                if let Some(name) = text(name, 128) {
                    names.insert(name);
                }
            }
            for name in [&entry["media"]["shortName"], &entry["station"]] {
                if let Some(name) = text(name, 128) {
                    names.insert(name);
                }
            }
        }
    }
    names.into_iter().take(32).collect()
}
fn statistics(raw: &Value) -> Vec<(String, String, String)> {
    raw.as_array()
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|entry| {
            let name = text(&entry["name"], 128)?;
            let label = text(&entry["label"], 256)
                .or_else(|| text(&entry["displayName"], 256))
                .unwrap_or_else(|| name.clone());
            let value =
                scalar(&entry["displayValue"], 256).or_else(|| scalar(&entry["value"], 256))?;
            Some((name, label, value))
        })
        .collect()
}
fn event_stats(home: &Value, away: &Value, summary: &Value) -> Vec<EventStat> {
    let side = |competitor: &Value| {
        summary["boxscore"]["teams"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["team"]["id"] == competitor["team"]["id"])
            .map_or_else(
                || statistics(&competitor["statistics"]),
                |entry| statistics(&entry["statistics"]),
            )
    };
    let mut stats: Vec<EventStat> = Vec::new();
    for (is_home, values) in [(true, side(home)), (false, side(away))] {
        for (name, label, value) in values {
            let position = stats.iter().position(|stat| stat.name == name);
            let index = position.unwrap_or_else(|| {
                stats.push(EventStat {
                    name,
                    label,
                    home: None,
                    away: None,
                });
                stats.len() - 1
            });
            if is_home {
                stats[index].home = Some(value);
            } else {
                stats[index].away = Some(value);
            }
        }
    }
    stats
}
fn plays(summary: &Value) -> Result<Vec<Play>> {
    let scoring: BTreeSet<_> = summary["scoringPlays"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|play| scalar(&play["id"], 128))
        .collect();
    let mut candidates = Vec::new();
    candidates.extend(summary["plays"].as_array().into_iter().flatten());
    for drive in summary["drives"]["previous"]
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
    {
        candidates.extend(drive["plays"].as_array().into_iter().flatten());
    }
    candidates.extend(
        summary["drives"]["current"]["plays"]
            .as_array()
            .into_iter()
            .flatten(),
    );
    candidates.extend(summary["scoringPlays"].as_array().into_iter().flatten());
    if candidates.len() > 4096 {
        return Err(InvalidFeed);
    }
    let mut result: Vec<Play> = Vec::new();
    let mut seen = HashMap::new();
    for raw in candidates {
        let Some(id) = scalar(&raw["id"], 128) else {
            continue;
        };
        let scoring = raw["scoringPlay"].as_bool() == Some(true) || scoring.contains(&id);
        if let Some(&index) = seen.get(&id) {
            let previous: &mut Play = &mut result[index];
            previous.scoring |= scoring;
            if previous.team_id.is_none() {
                previous.team_id = scalar(&raw["team"]["id"], 20).filter(|id| valid_event_id(id));
            }
            continue;
        }
        let Some(text) = text(&raw["text"], 4096).or_else(|| text(&raw["shortText"], 4096)) else {
            continue;
        };
        seen.insert(id.clone(), result.len());
        result.push(Play {
            id,
            text,
            period: integer(&raw["period"]["number"]).or_else(|| integer(&raw["period"])),
            clock: scalar(&raw["clock"]["displayValue"], 32).or_else(|| scalar(&raw["clock"], 32)),
            home_score: scalar(&raw["homeScore"], 32),
            away_score: scalar(&raw["awayScore"], 32),
            scoring,
            team_id: scalar(&raw["team"]["id"], 20).filter(|id| valid_event_id(id)),
        });
    }
    if result.len() > 2048 {
        return Err(InvalidFeed);
    }
    Ok(result)
}
fn event(
    league: &str,
    raw: &Value,
    summary: &Value,
    logos: &mut HashMap<String, String>,
) -> Result<Event> {
    let id = scalar(&raw["id"], 20)
        .filter(|id| valid_event_id(id))
        .ok_or(InvalidFeed)?;
    let competition = raw["competitions"]
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or(InvalidFeed)?;
    let competitors = competition["competitors"].as_array().ok_or(InvalidFeed)?;
    if competitors.len() != 2 {
        return Err(InvalidFeed);
    }
    let home_raw = competitors
        .iter()
        .find(|entry| entry["homeAway"] == "home")
        .ok_or(InvalidFeed)?;
    let away_raw = competitors
        .iter()
        .find(|entry| entry["homeAway"] == "away")
        .ok_or(InvalidFeed)?;
    let home = competitor(home_raw, logos)?;
    let away = competitor(away_raw, logos)?;
    if home.team.id == away.team.id {
        return Err(InvalidFeed);
    }
    let status = if competition["status"].is_object() {
        &competition["status"]
    } else {
        &raw["status"]
    };
    Ok(Event {
        id,
        league: league.to_owned(),
        name: text(&raw["name"], 512)
            .unwrap_or_else(|| format!("{} at {}", away.team.name, home.team.name)),
        short_name: text(&raw["shortName"], 128),
        date: timestamp(&competition["date"]).or_else(|| timestamp(&raw["date"])),
        status: EventStatus {
            state: text(&status["type"]["state"], 24),
            detail: text(&status["type"]["shortDetail"], 256)
                .or_else(|| text(&status["type"]["detail"], 256)),
            period: integer(&status["period"]),
            clock: scalar(&status["displayClock"], 32),
        },
        home,
        away,
        venue: text(&competition["venue"]["fullName"], 256)
            .or_else(|| text(&summary["gameInfo"]["venue"]["fullName"], 256)),
        broadcasts: broadcasts(competition, summary),
        stats: event_stats(home_raw, away_raw, summary),
        plays: plays(summary)?,
    })
}
fn standings(
    raw: &Value,
    depth: usize,
    groups: &mut Vec<StandingsGroup>,
    logos: &mut HashMap<String, String>,
) -> Result<()> {
    if depth > 3 {
        return Err(InvalidFeed);
    }
    if let Some(rows) = raw["standings"]["entries"].as_array() {
        if groups.len() >= 64 || rows.len() > 256 {
            return Err(InvalidFeed);
        }
        let entries = rows
            .iter()
            .map(|row| {
                let team = team(&row["team"], logos)?;
                let stats: BTreeMap<_, _> = statistics(&row["stats"])
                    .into_iter()
                    .map(|(name, _, value)| (name, value))
                    .collect();
                let rank = ["rank", "playoffSeed"]
                    .iter()
                    .find_map(|key| stats.get(*key).and_then(|value| value.parse::<i32>().ok()))
                    .filter(|rank| *rank > 0);
                Ok(Standing { team, rank, stats })
            })
            .collect::<Result<Vec<_>>>()?;
        groups.push(StandingsGroup {
            id: scalar(&raw["id"], 128).ok_or(InvalidFeed)?,
            name: text(&raw["name"], 256).ok_or(InvalidFeed)?,
            entries,
        });
    }
    if let Some(children) = raw["children"].as_array() {
        if children.len() > 64 {
            return Err(InvalidFeed);
        }
        for child in children {
            standings(child, depth + 1, groups, logos)?;
        }
    }
    Ok(())
}

pub(super) fn normalize(league: &str, kind: &Kind, raw: &Value) -> Result<Mapped> {
    let mut logos = HashMap::new();
    let payload = match kind {
        Kind::Scoreboard => {
            let rows = raw["events"].as_array().ok_or(InvalidFeed)?;
            if rows.len() > 128 {
                return Err(InvalidFeed);
            }
            let mut events = Vec::new();
            let mut ids = BTreeSet::new();
            for row in rows {
                let mapped = event(league, row, &Value::Null, &mut logos)?;
                if !ids.insert(mapped.id.clone()) {
                    return Err(InvalidFeed);
                }
                events.push(mapped);
            }
            serde_json::to_value(Scoreboard { events })
        }
        Kind::Event(expected) => {
            let event = event(league, &raw["header"], raw, &mut logos)?;
            if &event.id != expected {
                return Err(InvalidFeed);
            }
            serde_json::to_value(EventDetail { event })
        }
        Kind::Standings => {
            let mut groups = Vec::new();
            standings(raw, 0, &mut groups, &mut logos)?;
            if groups.is_empty() {
                return Err(InvalidFeed);
            }
            serde_json::to_value(Standings { groups })
        }
    }
    .map_err(|_| InvalidFeed)?;
    Ok(Mapped { payload, logos })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn header() -> Value {
        json!({"id":"123","competitions":[{"date":"2026-11-27T01:00Z","competitors":[
            {"homeAway":"home","team":{"id":"1","displayName":"Home","logo":"https://a.espncdn.com/i/teamlogos/nhl/500/home.png","color":"12aBff"},"score":"4"},
            {"homeAway":"away","team":{"id":"2","displayName":"Away"}}
        ],"status":{"type":{"state":"post","shortDetail":"Final"}}}]})
    }
    #[test]
    fn missing_values_stay_null_and_scores_keep_provider_precision() {
        let raw = json!({"header":header(),"boxscore":{"teams":[
            {"team":{"id":"1"},"statistics":[{"name":"shots","label":"Shots","displayValue":"20"}]},
            {"team":{"id":"2"},"statistics":[{"name":"hits","label":"Hits","displayValue":"7"}]}
        ]}});
        let mapped = normalize("nhl", &Kind::Event("123".into()), &raw).unwrap();
        let event = &mapped.payload["event"];
        assert_eq!(event["home"]["score"], "4");
        assert!(event["away"]["score"].is_null());
        assert!(event["status"]["period"].is_null());
        assert!(event["status"]["clock"].is_null());
        assert_eq!(event["date"], "2026-11-27T01:00:00Z");
        assert_eq!(event["stats"][0]["home"], "20");
        assert!(event["stats"][0]["away"].is_null());
        assert!(event["stats"][1]["home"].is_null());
        assert_eq!(event["stats"][1]["away"], "7");
        assert!(
            event["home"]["logo"]
                .as_str()
                .unwrap()
                .starts_with("/sports/api/v1/logos/")
        );
        assert_eq!(mapped.logos.len(), 1);
    }
    #[test]
    fn football_drives_and_scoring_plays_are_deduplicated() {
        let play = json!({"id":"p1","text":"Original scoring fixture","period":{"number":2},
            "clock":{"displayValue":"4:04"},"homeScore":7,"awayScore":3,"team":{"id":"1"}});
        let mut drive_play = play.clone();
        drive_play["team"] = Value::Null;
        let raw = json!({"header":header(),"drives":{"previous":[{"plays":[drive_play]}],
            "current":{"plays":[{"id":"p2","text":"Original next play fixture"}]}},
            "scoringPlays":[play]});
        let mapped = normalize("nfl", &Kind::Event("123".into()), &raw).unwrap();
        let plays = mapped.payload["event"]["plays"].as_array().unwrap();
        assert_eq!(plays.len(), 2);
        assert_eq!(plays[0]["homeScore"], "7");
        assert_eq!(plays[0]["teamId"], "1");
        assert_eq!(plays[0]["period"], 2);
        assert_eq!(plays[0]["scoring"], true);
        assert_eq!(plays[1]["scoring"], false);
        assert!(plays[1]["clock"].is_null());
    }
    #[test]
    fn standings_use_actual_groups_and_never_array_position_as_rank() {
        let raw = json!({"children":[{"id":"east","name":"Actual Group","standings":{"entries":[
            {"team":{"id":"1","displayName":"Unranked"},"stats":[{"name":"wins","displayValue":"9"}]},
            {"team":{"id":"2","displayName":"Ranked"},"stats":[
                {"name":"playoffSeed","displayValue":"3"},{"name":"overtimeLosses","displayValue":"2"}]}
        ]}}]});
        let mapped = normalize("nhl", &Kind::Standings, &raw).unwrap();
        let entries = &mapped.payload["groups"][0]["entries"];
        assert!(entries[0]["rank"].is_null());
        assert!(entries[0]["stats"].get("losses").is_none());
        assert_eq!(entries[1]["rank"], 3);
        assert_eq!(entries[1]["stats"]["overtimeLosses"], "2");
    }
    #[test]
    fn total_standings_group_limit_applies_across_nested_branches() {
        for count in [64, 65] {
            let groups: Vec<_> = (0..count)
                .map(|index| {
                    json!({
                        "id": index.to_string(), "name": format!("Group {index}"),
                        "standings": {"entries": []}
                    })
                })
                .collect();
            let raw = json!({"children": [
                {"children": &groups[..32]}, {"children": &groups[32..]}
            ]});
            assert_eq!(
                normalize("nhl", &Kind::Standings, &raw).is_ok(),
                count <= 64
            );
        }
    }
    #[test]
    fn malformed_or_wrong_event_responses_are_not_empty_successes() {
        assert!(normalize("nhl", &Kind::Scoreboard, &json!({})).is_err());
        assert!(normalize("nhl", &Kind::Scoreboard, &json!({"events":[]})).is_ok());
        assert!(
            normalize(
                "nhl",
                &Kind::Event("456".into()),
                &json!({"header":header()})
            )
            .is_err()
        );
        let mut missing_team = header();
        missing_team["competitions"][0]["competitors"][1]["homeAway"] = json!("home");
        assert!(
            normalize(
                "nhl",
                &Kind::Event("123".into()),
                &json!({"header":missing_team})
            )
            .is_err()
        );
        assert!(normalize("nhl", &Kind::Standings, &json!({"children":[]})).is_err());
    }
    #[test]
    #[ignore = "requires saved primary feed qualification in BLUEFIN_SPORTS_FIXTURE_DIR"]
    fn all_advertised_leagues_map_saved_primary_feeds() {
        let root =
            std::path::PathBuf::from(std::env::var_os("BLUEFIN_SPORTS_FIXTURE_DIR").unwrap());
        for league in ["nhl", "nfl", "nba", "mlb", "wnba", "epl", "mls"] {
            for (name, kind) in [
                ("scoreboard", Kind::Scoreboard),
                ("standings", Kind::Standings),
            ] {
                let raw = serde_json::from_slice(
                    &std::fs::read(root.join(format!("{league}-{name}.json"))).unwrap(),
                )
                .unwrap();
                let mapped = normalize(league, &kind, &raw).unwrap();
                assert!(mapped.payload.is_object());
                println!("{league} {name}: {} mapped logo assets", mapped.logos.len());
            }
        }
        for (league, id) in [("nhl", "401892781"), ("nfl", "401873081")] {
            let raw = serde_json::from_slice(
                &std::fs::read(root.join(format!("{league}-summary.json"))).unwrap(),
            )
            .unwrap();
            let mapped = normalize(league, &Kind::Event(id.into()), &raw).unwrap();
            assert!(mapped.payload["event"]["plays"].as_array().unwrap().len() > 100);
            assert!(
                !mapped.payload["event"]["stats"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
