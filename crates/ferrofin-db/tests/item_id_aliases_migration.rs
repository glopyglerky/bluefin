//! The deployed item-alias migration retains its shape and foreign-key boundary.
use ferrofin_db::Database;
#[tokio::test]
async fn item_aliases_have_the_deployed_owner_and_reject_self_aliases() {
    let db=Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let columns:Vec<(String,String,bool,i64)>=sqlx::query_as(
        "SELECT name,type,\"notnull\",pk FROM pragma_table_info('ItemIdAliases') ORDER BY cid"
    ).fetch_all(db.pool()).await.unwrap();
    assert_eq!(columns,[
        ("OldItemId".into(),"TEXT".into(),true,1),
        ("CanonicalItemId".into(),"TEXT".into(),true,0),
        ("ItemType".into(),"TEXT".into(),true,0),
    ]);
    let foreign:Vec<(String,String,String,String)>=sqlx::query_as(
        "SELECT \"table\",\"from\",\"to\",on_delete FROM pragma_foreign_key_list('ItemIdAliases')"
    ).fetch_all(db.pool()).await.unwrap();
    assert_eq!(foreign,[("BaseItems".into(),"CanonicalItemId".into(),"Id".into(),"CASCADE".into())]);
    assert!(sqlx::query("INSERT INTO ItemIdAliases VALUES('A','a','Audio')")
        .execute(db.writer()).await.is_err());
    assert!(sqlx::query("INSERT INTO ItemIdAliases VALUES('old','missing','Audio')")
        .execute(db.writer()).await.is_err());
    let index:Vec<String>=sqlx::query_scalar(
        "SELECT name FROM pragma_index_info('IX_ItemIdAliases_CanonicalItemId') ORDER BY seqno"
    ).fetch_all(db.pool()).await.unwrap();
    assert_eq!(index,["CanonicalItemId"]);
}
