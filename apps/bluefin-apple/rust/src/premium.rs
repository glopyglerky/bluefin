use super::*;
pub(super) const FIELDS: &str = "PrimaryImageAspectRatio,Overview,Genres,DateCreated,MediaStreams,Chapters,MediaSources,AlbumNormalizationGain";
fn optional(
    v: &Value,
    q: &mut Vec<(&'static str, String)>,
    names: &[(&str, &'static str)],
) -> Result<()> {
    for (input, output) in names {
        if v.get(input).is_some_and(|x| !x.is_null()) {
            q.push((output, text(v, input)?.into()));
        }
    }
    Ok(())
}
fn boolean(
    v: &Value,
    q: &mut Vec<(&'static str, String)>,
    input: &str,
    output: &'static str,
) -> Result<()> {
    if let Some(x) = v.get(input).filter(|x| !x.is_null()) {
        q.push((
            output,
            x.as_bool()
                .ok_or_else(|| err("Invalid boolean filter"))?
                .to_string(),
        ));
    }
    Ok(())
}
pub(super) fn ids(v: &Value, key: &str, empty: bool) -> Result<Vec<String>> {
    let a = v[key].as_array().ok_or_else(|| format!("Invalid {key}"))?;
    if a.len() > 200 || (!empty && a.is_empty()) {
        return Err(err("Invalid identifier count"));
    }
    a.iter()
        .map(|x| {
            let s = x.as_str().ok_or_else(|| err("Invalid identifier"))?;
            if s.is_empty()
                || s.len() > 128
                || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(err("Invalid identifier"));
            }
            Ok(s.into())
        })
        .collect()
}
// Explicit metadata projection excludes server paths, stream URLs and secret query strings.
pub(super) fn project(v: &Value) -> Value {
    let mut out = json!({});
    for k in [
        "Id",
        "Name",
        "Type",
        "MediaType",
        "Overview",
        "RunTimeTicks",
        "IndexNumber",
        "ParentIndexNumber",
        "ParentId",
        "SeriesId",
        "Number",
        "EpisodeTitle",
        "NormalizationGain",
        "AlbumNormalizationGain",
        "Album",
        "AlbumId",
        "Artists",
        "ArtistItems",
        "AlbumArtists",
        "AlbumArtist",
        "ArtistCount",
        "AlbumCount",
        "SongCount",
        "ChildCount",
        "RecursiveItemCount",
        "ProductionYear",
        "PremiereDate",
        "DateCreated",
        "Genres",
        "ImageTags",
        "BackdropImageTags",
        "PrimaryImageAspectRatio",
        "UserData",
        "PlaylistItemId",
        "HasLyrics",
        "ChannelId",
        "ChannelName",
        "ChannelNumber",
        "StartDate",
        "EndDate",
        "IsLive",
        "IsNews",
        "IsSports",
        "IsSeries",
        "IsMovie",
    ] {
        if let Some(x) = v.get(k) {
            out[k] = x.clone();
        }
    }
    for (key, fields) in [
        ("Chapters", &["Name", "StartPositionTicks", "ImageTag"][..]),
        (
            "MediaStreams",
            &[
                "Type",
                "Index",
                "Codec",
                "Language",
                "DisplayTitle",
                "Title",
                "Channels",
                "SampleRate",
                "BitDepth",
                "BitRate",
                "Profile",
                "VideoRange",
                "VideoRangeType",
                "Width",
                "Height",
                "DvProfile",
                "DvBlSignalCompatibilityId",
                "IsDefault",
                "IsForced",
                "IsExternal",
            ][..],
        ),
    ] {
        if let Some(a) = v[key].as_array() {
            out[key] = Value::Array(
                a.iter()
                    .take(256)
                    .map(|s| {
                        let mut x = json!({});
                        for k in fields {
                            if let Some(v) = s.get(k) {
                                x[*k] = v.clone();
                            }
                        }
                        x
                    })
                    .collect(),
            );
        }
    }
    if let Some(links) = v["ExternalUrls"].as_array() {
        out["ExternalUrls"] = Value::Array(
            links
                .iter()
                .filter_map(|link| {
                    let raw = link["Url"].as_str()?;
                    if raw.len() > 2048 {
                        return None;
                    }
                    let url = Url::parse(raw).ok()?;
                    if url.scheme() != "https"
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.port().is_some()
                        || url.fragment().is_some()
                    {
                        return None;
                    }
                    let host = url.host_str()?;
                    let p: Vec<_> = url.path_segments()?.collect();
                    let valid = match host {
                        "music.apple.com" => {
                            p.len() == 4
                                && p[0].len() == 2
                                && ["album", "song", "artist", "playlist"].contains(&p[1])
                                && !p[3].is_empty()
                                && p[3]
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                                && url.query_pairs().all(|(key, value)| {
                                    key == "i"
                                        && !value.is_empty()
                                        && value.bytes().all(|b| b.is_ascii_digit())
                                })
                        }
                        "open.spotify.com" => {
                            url.query().is_none()
                                && p.len() == 2
                                && ["track", "album", "artist", "playlist", "episode", "show"]
                                    .contains(&p[0])
                                && p[1].len() == 22
                                && p[1].bytes().all(|b| b.is_ascii_alphanumeric())
                        }
                        "tv.apple.com" => {
                            url.query().is_none()
                                && p.len() == 4
                                && p[0].len() == 2
                                && ["movie", "show", "episode", "season"].contains(&p[1])
                                && p[3].starts_with("umc.")
                                && p[3].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
                        }
                        _ => false,
                    };
                    valid.then(|| json!({"Url":raw}))
                })
                .take(8)
                .collect(),
        );
    }
    if v["CurrentProgram"].is_object() {
        let mut p = v["CurrentProgram"].clone();
        p.as_object_mut().unwrap().remove("CurrentProgram");
        out["CurrentProgram"] = project(&p);
    }
    out
}
fn page(v: Value) -> Result<Value> {
    let a = v["Items"]
        .as_array()
        .ok_or_else(|| err("Invalid items response"))?;
    let total = v["TotalRecordCount"]
        .as_u64()
        .ok_or_else(|| err("Invalid item count"))?;
    if a.len() > 200 {
        return Err(err("Server page exceeds limit"));
    }
    Ok(json!({"Items":a.iter().map(project).collect::<Vec<_>>(),"TotalRecordCount":total}))
}
pub(super) fn call(api: &Api<'_>, op: &str) -> Result<Value> {
    let v = api.v;
    let user = text(v, "userId")?;
    let mut q = vec![("userId", user.into())];
    match op {
        "musicItems" | "liveChannels" | "livePrograms" => {
            q.extend([
                (
                    "startIndex",
                    count(v, "startIndex", 0, 1000000)?.to_string(),
                ),
                ("limit", count(v, "limit", 50, 200)?.to_string()),
                ("fields", FIELDS.into()),
                ("enableUserData", "true".into()),
            ]);
            boolean(v, &mut q, "isFavorite", "isFavorite")?;
            let result = if op == "musicItems" {
                let kind = v["kind"].as_str().unwrap_or("songs");
                if kind == "playlistItems" {
                    api.get(&["Playlists", text(v, "playlistId")?, "Items"], q)?
                } else {
                    let sort = v["sortBy"].as_str().unwrap_or("SortName");
                    if ![
                        "SortName",
                        "DateCreated",
                        "PremiereDate",
                        "ProductionYear",
                        "Album",
                        "AlbumArtist",
                        "Artist",
                        "IndexNumber",
                        "ParentIndexNumber",
                        "ParentIndexNumber,IndexNumber",
                        "Random",
                    ]
                    .contains(&sort)
                    {
                        return Err(err("Invalid music sort"));
                    }
                    let order = v["sortOrder"].as_str().unwrap_or("Ascending");
                    if !["Ascending", "Descending"].contains(&order) {
                        return Err(err("Invalid sort order"));
                    }
                    q.extend([
                        ("sortBy", sort.into()),
                        ("sortOrder", order.into()),
                        ("recursive", "true".into()),
                    ]);
                    optional(
                        v,
                        &mut q,
                        &[
                            ("searchTerm", "searchTerm"),
                            ("nameStartsWithOrGreater", "nameStartsWithOrGreater"),
                            ("parentId", "parentId"),
                            ("artistId", "artistIds"),
                            ("albumArtistId", "albumArtistIds"),
                            ("genres", "genres"),
                            ("years", "years"),
                        ],
                    )?;
                    if v["albumId"].is_string() {
                        q.retain(|(k, _)| *k != "parentId");
                        q.push(("parentId", text(v, "albumId")?.into()));
                    }
                    match kind {
                        "artists" => api.get(&["Artists"], q)?,
                        "albumArtists" => api.get(&["Artists", "AlbumArtists"], q)?,
                        "songs" | "albums" | "playlists" => {
                            q.push((
                                "includeItemTypes",
                                match kind {
                                    "albums" => "MusicAlbum",
                                    "playlists" => "Playlist",
                                    _ => "Audio",
                                }
                                .into(),
                            ));
                            api.get(&["Users", user, "Items"], q)?
                        }
                        _ => return Err(err("Invalid music kind")),
                    }
                }
            } else if op == "liveChannels" {
                q.push(("addCurrentProgram", "true".into()));
                if v.get("type").is_some() {
                    let t = text(v, "type")?;
                    if !["TV", "Radio"].contains(&t) {
                        return Err(err("Invalid channel type"));
                    }
                    q.push(("type", t.into()));
                }
                api.get(&["LiveTv", "Channels"], q)?
            } else {
                q.extend([
                    ("channelIds", ids(v, "channelIds", false)?.join(",")),
                    ("sortBy", "StartDate".into()),
                    ("sortOrder", "Ascending".into()),
                ]);
                optional(
                    v,
                    &mut q,
                    &[
                        ("minStartDate", "minStartDate"),
                        ("maxStartDate", "maxStartDate"),
                        ("minEndDate", "minEndDate"),
                        ("maxEndDate", "maxEndDate"),
                    ],
                )?;
                boolean(v, &mut q, "isAiring", "isAiring")?;
                api.get(&["LiveTv", "Programs"], q)?
            };
            page(result)
        }
        "lyrics" => {
            let r = api.get(&["Audio", text(v, "itemId")?, "Lyrics"], vec![])?;
            if !r["Metadata"].is_object() || !r["Lyrics"].is_array() {
                return Err(err("Invalid lyrics response"));
            }
            Ok(json!({"Metadata":r["Metadata"],"Lyrics":r["Lyrics"]}))
        }
        "createPlaylist" => {
            let items = if v.get("itemIds").is_some() {
                ids(v, "itemIds", true)?
            } else {
                vec![]
            };
            let r = api.post(
                &["Playlists"],
                json!({"Name":text(v,"name")?,"Ids":items,"MediaType":"Audio","IsPublic":false}),
            )?;
            Ok(json!({"Id":text(&r,"Id")?}))
        }
        "addPlaylistItems" | "removePlaylistItems" | "renamePlaylist" | "deletePlaylist" => {
            let id = text(v, "playlistId")?;
            match op {
                "addPlaylistItems" => {
                    q.clear();
                    q.push(("ids", ids(v, "itemIds", false)?.join(",")));
                    if v.get("position").is_some() {
                        q.push((
                            "position",
                            count(v, "position", 0, i32::MAX as u64)?.to_string(),
                        ));
                    }
                    api.request(Method::POST, &["Playlists", id, "Items"], q, None, true)?;
                }
                "removePlaylistItems" => {
                    api.request(
                        Method::DELETE,
                        &["Playlists", id, "Items"],
                        vec![("entryIds", ids(v, "entryIds", false)?.join(","))],
                        None,
                        true,
                    )?;
                }
                "renamePlaylist" => {
                    api.post(&["Playlists", id], json!({"Name":text(v,"name")?}))?;
                }
                _ => {
                    let item = api.get(&["Users", user, "Items", id], vec![])?;
                    if item["Type"] != "Playlist" {
                        return Err(err("Refusing to delete a non-playlist item"));
                    }
                    api.request(Method::DELETE, &["Items", id], vec![], None, true)?;
                }
            }
            Ok(json!({"updated":true}))
        }
        _ => Err(err("Unknown premium operation")),
    }
}

#[cfg(test)]
mod integration_regressions {
    use super::*;
    #[test]
    fn preserves_native_gain_and_exact_public_provider_links() {
        let value = project(&json!({
            "Id":"item", "Type":"Audio", "SeriesId":"series", "NormalizationGain":-4.5,
            "AlbumNormalizationGain":-2.0,
            "ExternalUrls":[
                {"Url":"https://music.apple.com/us/album/example/123?i=456"},
                {"Url":"https://private.example/media?token=secret"},
                {"Url":"https://open.spotify.com/track/0123456789ABCDEFGHIJKL?token=secret"}
            ]
        }));
        assert_eq!(value["NormalizationGain"], -4.5);
        assert_eq!(value["AlbumNormalizationGain"], -2.0);
        assert_eq!(value["SeriesId"], "series");
        assert_eq!(
            value["ExternalUrls"],
            json!([{"Url":"https://music.apple.com/us/album/example/123?i=456"}])
        );
        assert!(!value.to_string().contains("secret"));
    }
}
