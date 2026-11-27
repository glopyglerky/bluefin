use std::sync::Arc;

use ferrofin_core::{
    FerrofinItemCountService, FerrofinItemPersistenceService, FerrofinItemRepository,
    FerrofinLibraryManager, FerrofinSyncPlayManager, FerrofinUserManager,
    item_type_lookup::{ItemTypeLookup, stored_type_name},
    item_visibility::ItemVisibility,
    localization_manager::LocalizationManager,
    people_repository::FerrofinPeopleRepository,
    session_bus::FerrofinSessionMessageBus,
};
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_model::{data::BaseItemKind, sync_play::GroupStateType};
use ferrofin_traits::{
    library::{LibraryManager, UserManager},
    options::DeleteOptions,
    stubs::{PlaybackRequest, SyncPlayManager, SyncPlaySession},
};
use uuid::Uuid;

#[tokio::test]
async fn current_room_is_scoped_to_the_joined_device_and_visible_item() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let users = Arc::new(FerrofinUserManager::new(db.clone()));
    let host = users.create_user("host").await.unwrap();
    let guest = users.create_user("guest").await.unwrap();
    let host = SyncPlaySession {
        session_id: "host".into(),
        user_id: Uuid::parse_str(&host.id).unwrap(),
        user_name: Some(host.username),
    };
    let guest = SyncPlaySession {
        session_id: "guest".into(),
        user_id: Uuid::parse_str(&guest.id).unwrap(),
        user_name: Some(guest.username),
    };

    let items = Arc::new(FerrofinItemRepository::new(
        db.clone(),
        Arc::new(ItemTypeLookup::new()),
    ));
    let visibility = Arc::new(ItemVisibility::new(
        db.clone(),
        items.clone(),
        Arc::new(LocalizationManager::new("US")),
        "/server/data".to_owned(),
    ));
    let library = Arc::new(
        FerrofinLibraryManager::new(
            items,
            Arc::new(FerrofinItemCountService::new(db.clone())),
            Arc::new(FerrofinItemPersistenceService::new(db.clone())),
            Arc::new(FerrofinPeopleRepository::new(db.clone())),
        )
        .with_visibility(visibility),
    );
    let manager = FerrofinSyncPlayManager::new(Arc::new(FerrofinSessionMessageBus::new()))
        .with_library_access(users, library.clone());

    let film = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO "BaseItems"
           ("Id", "Type", "Name", "IsFolder", "IsInMixedFolder", "IsLocked", "IsMovie",
            "IsRepeat", "IsSeries", "IsVirtualItem")
           VALUES (?1, ?2, 'Fixture movie', 0, 0, 0, 1, 0, 0, 0)"#,
    )
    .bind(guid_to_db(film))
    .bind(stored_type_name(BaseItemKind::Movie).unwrap())
    .execute(db.writer())
    .await
    .unwrap();

    let group = manager
        .new_group(&host, "Watch together", None)
        .await
        .unwrap();
    manager
        .handle_request(
            &host,
            PlaybackRequest::Play {
                playing_queue: vec![film],
                playing_item_position: 0,
                start_position_ticks: 20_000_000,
            },
        )
        .await
        .unwrap();

    assert_eq!(manager.current_playback(&guest).await.unwrap(), None);
    manager.join_group(&guest, group.group_id).await.unwrap();
    let room = manager.current_playback(&guest).await.unwrap().unwrap();
    assert_eq!(room.group_id, group.group_id);
    assert_eq!(room.item.unwrap().item_id, film);
    assert_eq!(room.state, GroupStateType::Waiting);
    assert_eq!(room.position_ticks, 20_000_000);

    let same_user_new_device = SyncPlaySession {
        session_id: "unjoined device".into(),
        ..guest.clone()
    };
    assert_eq!(
        manager
            .current_playback(&same_user_new_device)
            .await
            .unwrap(),
        None
    );
    library
        .delete_item(
            film,
            &DeleteOptions {
                delete_file_location: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(manager.current_playback(&guest).await.unwrap(), None);
    manager.leave_group(&guest).await.unwrap();
    assert_eq!(manager.current_playback(&guest).await.unwrap(), None);
}
