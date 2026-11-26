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
    metadata(v, q)
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
    let truncated = studios["TotalRecordCount"].as_u64().unwrap_or(0) > 200
        || collections["TotalRecordCount"].as_u64().unwrap_or(0) > 200 || all_tags.len() > 512;
    Ok(json!({"studios":named(&studios)?, "tags":tags, "collections":named(&collections)?, "truncated":truncated}))
}

#[cfg(test)]
mod tests {
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
