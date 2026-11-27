//! Account-scoped catalog facets. Filtering is performed by the owning server.
use super::*;
use bluefin_contract::search::SearchScope;

/// Apply a validated media-type scope and metadata facets to every server page.
pub(super) fn query(v: &Value, q: &mut Vec<(&'static str, String)>) -> Result<()> {
    if let Some(scope) = v.get("searchScope").filter(|x| !x.is_null()) {
        let scope: SearchScope = serde_json::from_value(scope.clone())
            .map_err(|_| err("Invalid search category"))?;
        q.retain(|(name, _)| !name.eq_ignore_ascii_case("IncludeItemTypes"));
        if let Some(types) = scope.library_item_types() {
            q.push(("IncludeItemTypes", types.into()));
        }
    }
    metadata(v, q)?;
    ordering(v, q)
}

/// Forward only supported ordering controls. SearchOrder is an explicit Bluefin
/// extension; ordinary Jellyfin clients keep relevance-first search behavior.
pub(super) fn ordering(v: &Value, q: &mut Vec<(&'static str, String)>) -> Result<()> {
    if let Some(sort) = v.get("sortBy").filter(|value| !value.is_null()) {
        let sort = sort.as_str().ok_or_else(|| err("Invalid search sort"))?;
        if sort.is_empty() || !sort.split(',').all(|key| {
            ["SortName", "Name", "DateCreated", "PremiereDate", "ProductionYear"]
                .iter().any(|allowed| key.eq_ignore_ascii_case(allowed))
        }) {
            return Err(err("Unsupported search sort"));
        }
        q.push(("SortBy", sort.into()));
    }
    if let Some(order) = v.get("sortOrder").filter(|value| !value.is_null()) {
        let order = order.as_str().ok_or_else(|| err("Invalid sort direction"))?;
        if !order.eq_ignore_ascii_case("Ascending") && !order.eq_ignore_ascii_case("Descending") {
            return Err(err("Invalid sort direction"));
        }
        q.push(("SortOrder", order.into()));
    }
    if let Some(mode) = v.get("searchOrder").filter(|value| !value.is_null()) {
        if mode.as_str() != Some("SortBy") || !q.iter().any(|(key, _)| *key == "SortBy") {
            return Err(err("SearchOrder=SortBy requires a supported sort"));
        }
        q.push(("SearchOrder", "SortBy".into()));
    }
    Ok(())
}

/// Validate delimiter-safe studio and tag selections before forwarding them.
pub(super) fn metadata(v: &Value, q: &mut Vec<(&'static str, String)>) -> Result<()> {
    if v.get("studioIds").is_some_and(|x| !x.is_null()) {
        let values = premium::ids(v, "studioIds", true)?;
        if values.len() > 16 { return Err(err("Too many studio filters")); }
        if !values.is_empty() { q.push(("StudioIds", values.join(","))); }
    }
    if let Some(tags) = v.get("tags").filter(|x| !x.is_null()) {
        let tags = tags.as_array().ok_or_else(|| err("Invalid tag filters"))?;
        if tags.len() > 16 { return Err(err("Too many tag filters")); }
        let tags = tags.iter().map(|value| {
            let tag = value.as_str().ok_or_else(|| err("Invalid tag"))?;
            if tag.is_empty() || tag.len() > 256 || tag.contains('|') || tag.chars().any(char::is_control) {
                return Err(err("Invalid tag"));
            }
            Ok(tag)
        }).collect::<Result<Vec<_>>>()?;
        if !tags.is_empty() { q.push(("Tags", tags.join("|"))); }
    }
    for (input, output) in [("isPlayed", "IsPlayed"), ("isFavorite", "IsFavorite")] {
        if let Some(value) = v.get(input).filter(|value| !value.is_null()) {
            q.push((output, value.as_bool().ok_or_else(|| err("Invalid library filter"))?.to_string()));
        }
    }
    if let Some(values) = v.get("genres").filter(|value| !value.is_null()) {
        let values = values.as_array().ok_or_else(|| err("Invalid genres"))?;
        if values.len() > 16 { return Err(err("Too many genre filters")); }
        let genres = values.iter().map(|value| {
            let name = value.as_str().ok_or_else(|| err("Invalid genre"))?;
            if name.is_empty() || name.len() > 256 || name.contains('|') || name.chars().any(char::is_control) {
                return Err(err("Invalid genre"));
            }
            Ok(name)
        }).collect::<Result<Vec<_>>>()?;
        if !genres.is_empty() { q.push(("Genres", genres.join("|"))); }
    }
    if let Some(values) = v.get("years").filter(|value| !value.is_null()) {
        let values = values.as_array().ok_or_else(|| err("Invalid years"))?;
        if values.len() > 32 { return Err(err("Too many release years")); }
        let years = values.iter().map(|value| {
            let year = value.as_i64().filter(|year| (1..=9999).contains(year))
                .ok_or_else(|| err("Invalid release year"))?;
            Ok(year.to_string())
        }).collect::<Result<Vec<_>>>()?;
        if !years.is_empty() { q.push(("Years", years.join(","))); }
    }

    Ok(())
}

/// Return bounded studio, tag, and collection choices visible to this account.
pub(super) fn facets(api: &Api<'_>) -> Result<Value> {
    let user = text(api.v, "userId")?;
    let studios = api.get(&["Studios"], vec![
        ("userId", user.into()), ("limit", "200".into()), ("recursive", "true".into()),
        ("sortBy", "SortName".into()), ("sortOrder", "Ascending".into())
    ])?;
    let filters = api.get(&["Items", "Filters2"], vec![("userId", user.into())])?;
    let collections = api.get(&["Users", user, "Items"], vec![
        ("IncludeItemTypes", "BoxSet".into()), ("Recursive", "true".into()),
        ("Limit", "200".into()), ("SortBy", "SortName".into())
    ])?;
    let named = |value: &Value| -> Result<Vec<Value>> {
        let rows = value["Items"].as_array().ok_or_else(|| err("Invalid search filters"))?;
        if rows.len() > 200 { return Err(err("Search filter page exceeds limit")); }
        Ok(rows.iter().filter_map(|row| {
            let id = row["Id"].as_str()?;
            let name = row["Name"].as_str()?;
            if id.is_empty() || id.len() > 128 || !id.bytes().all(|x| x.is_ascii_alphanumeric() || x == b'-')
                || name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) { return None; }
            Some(json!({"id":id,"name":name}))
        }).collect())
    };
    let all_tags = filters["Tags"].as_array().ok_or_else(|| err("Invalid tag filters"))?;
    let tags: Vec<_> = all_tags.iter().filter_map(Value::as_str)
        .filter(|tag| !tag.trim().is_empty() && tag.len() <= 256 && !tag.contains('|') && !tag.chars().any(char::is_control))
        .take(512).collect();
    let mut genres: Vec<_> = filters["Genres"].as_array().into_iter().flatten()
        .filter_map(|value| value.as_str().or_else(|| value["Name"].as_str()))
        .filter(|name| !name.trim().is_empty() && name.len() <= 256 && !name.contains('|') && !name.chars().any(char::is_control))
        .take(512).collect();
    genres.sort_unstable(); genres.dedup();
    let mut years: Vec<_> = filters["Years"].as_array().into_iter().flatten()
        .filter_map(Value::as_i64).filter(|year| (1..=9999).contains(year)).take(512).collect();
    years.sort_unstable_by(|a, b| b.cmp(a)); years.dedup();
    let truncated = studios["TotalRecordCount"].as_u64().unwrap_or(0) > 200
        || collections["TotalRecordCount"].as_u64().unwrap_or(0) > 200 || all_tags.len() > 512;
    Ok(json!({"studios":named(&studios)?, "tags":tags, "collections":named(&collections)?, "genres":genres, "years":years, "truncated":truncated}))
}

#[cfg(test)]
mod tests {

    #[test]
    fn explicit_search_sort_and_refinements_are_retained_for_every_page() {
        let mut query_fields = vec![("StartIndex", "120".into()), ("Limit", "60".into())];
        query(&json!({
            "searchScope":"movies", "sortBy":"PremiereDate", "sortOrder":"Descending",
            "searchOrder":"SortBy", "isPlayed":false, "isFavorite":true,
            "genres":["Drama"], "years":[1997]
        }), &mut query_fields).unwrap();
        for pair in [
            ("StartIndex", "120"), ("SortBy", "PremiereDate"), ("SortOrder", "Descending"),
            ("SearchOrder", "SortBy"), ("IsPlayed", "false"), ("IsFavorite", "true"),
            ("Genres", "Drama"), ("Years", "1997")
        ] {
            assert!(query_fields.iter().any(|(key, value)| *key == pair.0 && value == pair.1));
        }
        let mut relevance = Vec::new();
        query(&json!({"searchScope":"all"}), &mut relevance).unwrap();
        assert!(!relevance.iter().any(|(key, _)| matches!(*key, "SortBy" | "SortOrder" | "SearchOrder")));
    }

    #[test]
    fn malformed_sort_and_refinement_values_fail_before_a_request() {
        for value in [
            json!({"searchOrder":"SortBy"}),
            json!({"sortBy":"Random"}),
            json!({"sortBy":"SortName","sortOrder":"Unknown"}),
            json!({"sortBy":"SortName","searchOrder":"Unknown"}),
            json!({"genres":["Drama|Comedy"]}),
            json!({"years":[-1]}),
            json!({"years":["2026"]}),
            json!({"isPlayed":"false"})
        ] {
            assert!(query(&value, &mut Vec::new()).is_err(), "{value}");
        }
    }

    use super::*;
    #[test]
    fn broad_search_removes_stale_type_and_facets_remain_server_queries() {
        let mut q = vec![("includeItemTypes", "Movie".into()), ("SearchTerm", "ocean".into())];
        query(&json!({"searchScope":"all","studioIds":["abc-123"],"tags":["My service"]}), &mut q).unwrap();
        assert!(!q.iter().any(|(key,_)| key.eq_ignore_ascii_case("IncludeItemTypes")));
        assert!(q.contains(&("SearchTerm","ocean".into())));
        assert!(q.contains(&("StudioIds","abc-123".into())));
        assert!(q.contains(&("Tags","My service".into())));
    }
    #[test]
    fn broad_live_programme_search_does_not_require_a_selected_channel() {
        let (server, request) = crate::tests::mock("200 OK", r#"{"Items":[],"TotalRecordCount":0}"#);
        let value = crate::call(&json!({"operation":"livePrograms","server":server,"allowLocalHttp":true,
            "userId":"viewer","token":"token","searchTerm":"science","startIndex":60,"limit":60})).unwrap();
        assert_eq!(value["TotalRecordCount"], 0);
        let request = request.join().unwrap();
        assert!(request.starts_with("GET /jellyfin/LiveTv/Programs?"));
        assert!(request.contains("searchTerm=science"));
        assert!(request.contains("startIndex=60"));
        assert!(!request.contains("channelIds="));
    }
    #[test]
    fn tag_cannot_inject_another_filter_or_control_character() {
        for tag in ["first|second", "line\\nnext", ""] {
            let tag = tag.replace("\\n", "\n");
            assert!(query(&json!({"tags":[tag]}), &mut vec![]).is_err());
        }
        assert!(query(&json!({"studioIds":["one,two"]}), &mut vec![]).is_err());
        assert!(query(&json!({"searchScope":"invalid"}), &mut vec![]).is_err());
    }
}
