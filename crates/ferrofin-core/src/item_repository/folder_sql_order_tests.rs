//! A folder listing sorted by a key the in-memory sort does not cover
//! (`IsFolder,SortName`, jellyfin-web's default for a folder view) takes its
//! order from SQL. The children are already loaded, so SQL returns only their
//! ids: the rows handed in come back in SQL's order, less the ids SQL does not
//! return, and no row is decoded a second time (gl-1260).

use super::*;
use crate::latest_tv_repository::explain;
use crate::test_support::{
    fetch_item, save_item, seed_folder_item, seed_named_item, seed_user_with_defaults, test_db,
};
use ferrofin_model::dto::SortOrder;
use ferrofin_model::live_tv::ItemSortBy;

#[tokio::test]
async fn sql_ordered_folder_children_keep_the_rows_handed_in() {
    let db = test_db().await;
    let repo = FerrofinItemRepository::new(
        db.clone(),
        Arc::new(crate::item_type_lookup::ItemTypeLookup::new()),
    );
    let mut rows = Vec::new();
    for (kind, name) in [
        (BaseItemKind::Movie, "Charlie"),
        (BaseItemKind::Folder, "Bravo"),
        (BaseItemKind::Movie, "Alpha"),
    ] {
        let id = Uuid::new_v4();
        if kind == BaseItemKind::Folder {
            seed_folder_item(&db, id, kind, name, None).await;
        } else {
            seed_named_item(&db, id, kind, name).await;
        }
        let mut row = fetch_item(&db, id).await;
        row.sort_name = Some(name.to_lowercase());
        save_item(&db, &row).await;
        rows.push(fetch_item(&db, id).await);
    }
    // What the caller loaded, not what is stored: the sort must hand it back.
    for row in &mut rows {
        row.overview = Some(format!(
            "loaded {}",
            row.name.as_deref().unwrap_or_default()
        ));
    }
    // An id SQL does not return is left out, as the re-read left it out.
    rows.push(BaseItemEntity {
        id: Uuid::new_v4().to_string(),
        name: Some("Unstored".to_owned()),
        ..Default::default()
    });
    let query = InternalItemsQuery {
        order_by: vec![
            (ItemSortBy::IsFolder, SortOrder::Ascending),
            (ItemSortBy::SortName, SortOrder::Ascending),
        ],
        ..Default::default()
    };
    let sorted = repo.sort_folder_children(rows, &query).await.unwrap();
    let names: Vec<_> = sorted
        .iter()
        .filter_map(|row| row.name.as_deref())
        .collect();
    assert_eq!(names, ["Alpha", "Charlie", "Bravo"]);
    for row in &sorted {
        assert_eq!(
            row.overview.as_deref(),
            Some(format!("loaded {}", row.name.as_deref().unwrap_or_default()).as_str())
        );
    }
}

/// Reading only the ids must not change the plan, so SQL keeps the order, ties
/// included, that the full-row read it replaced gave.
#[tokio::test]
async fn reading_only_the_ids_keeps_the_full_row_plan() {
    let db = test_db().await;
    let user = Arc::new(seed_user_with_defaults(&db, Uuid::new_v4()).await);
    let rows: Vec<_> = (0..200)
        .map(|_| BaseItemEntity {
            id: Uuid::new_v4().to_string(),
            ..Default::default()
        })
        .collect();
    for order_by in [
        vec![
            (ItemSortBy::IsFolder, SortOrder::Ascending),
            (ItemSortBy::SortName, SortOrder::Ascending),
        ],
        vec![
            (ItemSortBy::DateCreated, SortOrder::Descending),
            (ItemSortBy::SortName, SortOrder::Ascending),
        ],
    ] {
        let filter = InternalItemsQuery {
            user: Some(Arc::clone(&user)),
            order_by,
            ..Default::default()
        };
        let query = folder_sql_order_query(&rows, &filter);
        let ids = explain(&db, build_query(&query, QueryShape::IdsOnly)).await;
        let full = explain(&db, build_query(&query, QueryShape::FullRows)).await;
        assert_eq!(ids, full);
        assert!(ids.contains("USE TEMP B-TREE FOR ORDER BY"), "{ids}");
    }
}


#[tokio::test]
async fn bluefin_explicit_folder_order_retains_identity_ties_before_paging() {
    let db = test_db().await;
    let repo = FerrofinItemRepository::new(
        db.clone(),
        Arc::new(crate::item_type_lookup::ItemTypeLookup::new()),
    );
    let low = Uuid::from_u128(0xB501);
    let high = Uuid::from_u128(0xB50F);
    let mut rows = Vec::new();
    for (id, name) in [(high, "Blade Alpha"), (low, "Blade Zebra")] {
        seed_named_item(&db, id, BaseItemKind::Movie, name).await;
        let mut row = fetch_item(&db, id).await;
        row.sort_name = Some(name.to_lowercase());
        save_item(&db, &row).await;
        rows.push(fetch_item(&db, id).await);
    }
    sqlx::query(r#"UPDATE "BaseItems" SET "DateCreated" = '2020-01-01 00:00:00'"#)
        .execute(db.writer()).await.unwrap();
    let filter = InternalItemsQuery {
        search_order_by_requested: true,
        search_term: Some("blade".into()),
        order_by: vec![(ItemSortBy::DateCreated, SortOrder::Descending)],
        limit: Some(1),
        ..Default::default()
    };
    let sorted = repo.sort_folder_children(rows, &filter).await.unwrap();
    let first = page(sorted.clone(), &filter);
    let second = page(sorted, &InternalItemsQuery { start_index: Some(1), ..filter });
    assert_eq!(Uuid::parse_str(&first[0].id).unwrap(), low);
    assert_eq!(Uuid::parse_str(&second[0].id).unwrap(), high);

    // Equal title keys formerly stayed in the incoming materialized order.
    for id in [low, high] {
        let mut row = fetch_item(&db, id).await;
        row.name = Some("Blade Same".into());
        row.clean_name = Some("blade same".into());
        row.sort_name = Some("blade same".into());
        save_item(&db, &row).await;
    }
    for key in [ItemSortBy::Name, ItemSortBy::SortName] {
        let rows = vec![fetch_item(&db, high).await, fetch_item(&db, low).await];
        let sorted = repo.sort_folder_children(rows, &InternalItemsQuery {
            search_order_by_requested: true,
            order_by: vec![(key, SortOrder::Ascending)],
            ..Default::default()
        }).await.unwrap();
        assert_eq!(sorted.iter().map(|row| Uuid::parse_str(&row.id).unwrap()).collect::<Vec<_>>(),
                   [low, high]);
    }
}
