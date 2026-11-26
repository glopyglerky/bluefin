//! Real persistence and production manager fixtures for opened generations.
use super::tests::{StubEncoder, StubProvider, user_data_manager};
use super::*;
use crate::media_attachment_repository::FerrofinMediaAttachmentRepository;
use crate::media_stream_repository::FerrofinMediaStreamRepository;
use crate::test_support::{
    fetch_item, item_repository_over, save_item, seed_item, seed_user_with_defaults,
};
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_model::data::BaseItemKind;
use ferrofin_traits::library::UserManager;

#[path = "karaoke_checks.rs"]
mod karaoke_checks;

struct Fixture {
    directory: tempfile::TempDir,
    db: Database,
    manager: FerrofinMediaSourceManager,
    item: Uuid,
    library: Uuid,
    viewer: Uuid,
    original: String,
}

fn wav(sample: [u8; 2]) -> Vec<u8> {
    let size = 16_000_u32;
    let mut data = b"RIFF".to_vec();
    data.extend_from_slice(&(size + 36).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16_u32.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&8000_u32.to_le_bytes());
    data.extend_from_slice(&16000_u32.to_le_bytes());
    data.extend_from_slice(&2_u16.to_le_bytes());
    data.extend_from_slice(&16_u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&size.to_le_bytes());
    for _ in 0..8000 {
        data.extend_from_slice(&sample);
    }
    data
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("fixture.db");
        let db = Database::connect_sized(&format!("sqlite://{}", database.display()), Some(1))
            .await
            .unwrap();
        let viewer = Uuid::new_v4();
        seed_user_with_defaults(&db, viewer).await;
        let library = Uuid::new_v4();
        seed_item(&db, library, BaseItemKind::CollectionFolder).await;
        let mut library_row = fetch_item(&db, library).await;
        library_row.is_folder = true;
        library_row.path = Some(
            directory
                .path()
                .join("library-metadata")
                .to_str()
                .unwrap()
                .into(),
        );
        library_row.data = Some(
            serde_json::json!({"PhysicalLocationsList":[directory.path().to_str().unwrap()]})
                .to_string(),
        );
        save_item(&db, &library_row).await;
        let item = Uuid::new_v4();
        let original = directory
            .path()
            .join("original.wav")
            .to_str()
            .unwrap()
            .to_owned();
        std::fs::write(&original, wav([0, 0])).unwrap();
        seed_item(&db, item, BaseItemKind::Audio).await;
        let mut row = fetch_item(&db, item).await;
        row.path = Some(original.clone());
        row.media_type = Some("Audio".into());
        row.run_time_ticks = Some(10_000_000);
        row.data = Some("{}".into());
        save_item(&db, &row).await;
        let streams = Arc::new(FerrofinMediaStreamRepository::new(db.clone()));
        streams
            .save_media_streams(
                item,
                &[MediaStreamInfoEntity {
                    stream_index: 0,
                    stream_type: 0,
                    codec: Some("pcm_s16le".into()),
                    sample_rate: Some(8000),
                    channels: Some(1),
                    ..Default::default()
                }],
            )
            .await
            .unwrap();
        let attachments = Arc::new(FerrofinMediaAttachmentRepository::new(db.clone()));
        attachments
            .save_media_attachments(
                item,
                &[AttachmentStreamInfoEntity {
                    item_id: String::new(),
                    index: 0,
                    filename: Some("original.png".into()),
                    codec: None,
                    codec_tag: None,
                    comment: None,
                    mime_type: None,
                }],
            )
            .await
            .unwrap();
        let items = item_repository_over(db.clone());
        let visibility = Arc::new(crate::item_visibility::ItemVisibility::new(
            db.clone(),
            Arc::clone(&items),
            Arc::new(crate::localization_manager::LocalizationManager::default()),
            String::new(),
        ));
        let manager = FerrofinMediaSourceManager::new(
            items,
            streams,
            attachments,
            Arc::new(StubEncoder),
            Arc::new(StubProvider),
        )
        .with_users(Arc::new(crate::user_manager::FerrofinUserManager::new(
            db.clone(),
        )))
        .with_visibility(visibility)
        .with_user_data(user_data_manager(&db));
        Self {
            directory,
            db,
            manager,
            item,
            library,
            viewer,
            original,
        }
    }

    async fn pin(&self) -> Arc<dyn MediaSourceManager> {
        self.manager
            .pin_local_source(
                self.item,
                &self.item.simple().to_string(),
                &self.original,
                self.viewer,
            )
            .await
            .unwrap()
    }

    async fn independent_writer(&self) -> sqlx::SqlitePool {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!(
                "sqlite://{}",
                self.directory.path().join("fixture.db").display()
            ))
            .await
            .unwrap()
    }

    async fn change_audio_codec(&self, codec: &str) {
        sqlx::query(r#"UPDATE "MediaStreamInfos" SET "Codec"=?2 WHERE "ItemId"=?1"#)
            .bind(guid_to_db(self.item))
            .bind(codec)
            .execute(&self.independent_writer().await)
            .await
            .unwrap();
    }

    async fn change_permission(&self, kind: ferrofin_db::enums::PermissionKind, value: bool) {
        sqlx::query(r#"UPDATE "Permissions" SET "Value"=?3 WHERE "UserId"=?1 AND "Kind"=?2"#)
            .bind(guid_to_db(self.viewer))
            .bind(i32::from(kind))
            .bind(value)
            .execute(&self.independent_writer().await)
            .await
            .unwrap();
    }

    async fn replace(&self, path: &str) {
        sqlx::query(r#"UPDATE "BaseItems" SET "Path"=? WHERE "Id"=?"#)
            .bind(path)
            .bind(guid_to_db(self.item))
            .execute(&self.independent_writer().await)
            .await
            .unwrap();
    }
}

async fn assert_captured(
    f: &Fixture,
    manager: &dyn MediaSourceManager,
    known: &BaseItemEntity,
) -> String {
    let source = f.item.simple().to_string();
    let mut all = manager
        .get_static_media_sources(f.item, true, Some(f.viewer))
        .await
        .unwrap();
    all.extend(
        manager
            .get_static_media_sources_for(known, true, Some(f.viewer))
            .await
            .unwrap(),
    );
    all.extend(
        manager
            .get_playback_media_sources(f.item, f.viewer, true, true)
            .await
            .unwrap(),
    );
    all.extend(
        manager
            .get_playback_media_sources_for(known, None, f.viewer, true, true)
            .await
            .unwrap(),
    );
    all.push(
        manager
            .get_stream_media_source(f.item, Some(&source), None)
            .await
            .unwrap(),
    );
    all.push(
        manager
            .get_media_source(known, &source, None, true)
            .await
            .unwrap()
            .unwrap(),
    );
    let input = all[0].path.clone().unwrap();
    assert!(input.starts_with("/proc/"));
    for resolved in all {
        assert_eq!(resolved.path.as_deref(), Some(input.as_str()));
        assert_eq!(resolved.protocol, MediaProtocol::File);
        assert_eq!(resolved.id.as_deref(), Some(source.as_str()));
        assert_eq!(
            resolved.media_streams[0].codec.as_deref(),
            Some("pcm_s16le")
        );
        assert_eq!(
            resolved.media_attachments[0].file_name.as_deref(),
            Some("original.png")
        );
        assert_eq!(
            tokio::fs::read(resolved.path.unwrap()).await.unwrap(),
            wav([0, 0])
        );
    }
    assert_captured_auxiliary(f, manager).await;
    assert!(
        manager
            .get_stream_media_source(f.item, Some(&Uuid::new_v4().to_string()), None)
            .await
            .is_err()
    );
    assert!(
        manager
            .get_stream_media_source(f.item, Some(&source), Some("live-token"))
            .await
            .is_err()
    );
    assert!(
        manager
            .get_static_media_sources(Uuid::new_v4(), false, None)
            .await
            .is_err()
    );
    input
}

async fn assert_captured_auxiliary(f: &Fixture, manager: &dyn MediaSourceManager) {
    assert_eq!(
        manager.get_media_streams(f.item).await.unwrap()[0]
            .codec
            .as_deref(),
        Some("pcm_s16le")
    );
    assert_eq!(
        manager.get_media_streams_batch(&[f.item]).await.unwrap()[&f.item][0]
            .codec
            .as_deref(),
        Some("pcm_s16le")
    );
    assert_eq!(
        manager.get_media_attachments(f.item).await.unwrap()[0]
            .file_name
            .as_deref(),
        Some("original.png")
    );
    assert_eq!(
        manager
            .get_media_attachments_batch(&[f.item])
            .await
            .unwrap()[&f.item][0]
            .file_name
            .as_deref(),
        Some("original.png")
    );
}

#[tokio::test]
async fn source_pin_freezes_every_resolution_across_independent_sql_writes() {
    let f = Fixture::new().await;
    let mut known = fetch_item(&f.db, f.item).await;
    let pin = f.pin().await;
    let replacement = f.directory.path().join("replacement.wav");
    std::fs::write(&replacement, wav([1, 0])).unwrap();
    f.replace(replacement.to_str().unwrap()).await;
    // A stale or caller-supplied known row cannot replace the opened input.
    known.path = Some("https://untrusted.example/replaced.mp3".into());
    let input = assert_captured(&f, pin.as_ref(), &known).await;
    f.replace("https://untrusted.example/replaced.mp3").await;
    assert_eq!(assert_captured(&f, pin.as_ref(), &known).await, input);
    // Stored stream/attachment replacement is also independent of this scope.
    FerrofinMediaStreamRepository::new(f.db.clone())
        .save_media_streams(f.item, &[])
        .await
        .unwrap();
    FerrofinMediaAttachmentRepository::new(f.db.clone())
        .save_media_attachments(f.item, &[])
        .await
        .unwrap();
    assert_captured(&f, pin.as_ref(), &known).await;
    std::fs::rename(&f.original, f.directory.path().join("moved.wav")).unwrap();
    std::fs::write(&f.original, wav([2, 0])).unwrap();
    assert_captured(&f, pin.as_ref(), &known).await;
    // Ordinary manager state is deliberately outside the frozen playback scope.
    assert_eq!(
        f.manager
            .get_stream_media_source(f.item, None, None)
            .await
            .unwrap()
            .path
            .as_deref(),
        Some("https://untrusted.example/replaced.mp3")
    );
}

#[tokio::test]
async fn source_pin_other_item_and_metadata_writes_continue_and_last_clone_releases() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let survivor = Arc::clone(&pin);
    let weak = Arc::downgrade(&pin);
    let other = Uuid::new_v4();
    seed_item(&f.db, other, BaseItemKind::Audio).await;
    let mut row = fetch_item(&f.db, other).await;
    row.path = Some("/another/local.wav".into());
    save_item(&f.db, &row).await;
    assert_eq!(fetch_item(&f.db, other).await.path, row.path);
    let mut row = fetch_item(&f.db, f.item).await;
    row.name = Some("Updated title".into());
    save_item(&f.db, &row).await;
    drop(pin);
    assert!(weak.upgrade().is_some());
    assert!(
        survivor
            .get_stream_media_source(f.item, None, None)
            .await
            .is_ok()
    );
    drop(survivor);
    assert!(weak.upgrade().is_none());
    f.replace("/replacement/after-release.wav").await;
    assert_eq!(
        f.manager
            .get_stream_media_source(f.item, None, None)
            .await
            .unwrap()
            .path
            .as_deref(),
        Some("/replacement/after-release.wav")
    );
}

#[tokio::test]
async fn source_pin_revocations_use_current_rows_even_with_row_cache_enabled() {
    let f = Fixture::new().await;
    f.db.enable_row_cache(std::time::Duration::from_secs(600));
    let pin = f.pin().await;
    let users = crate::user_manager::FerrofinUserManager::new(f.db.clone());
    let user = users.get_user_by_id(f.viewer).await.unwrap().unwrap();
    let mut policy = users
        .get_user_dto(&user, None)
        .await
        .unwrap()
        .policy
        .unwrap();
    policy.blocked_tags = vec!["blocked".into()];
    users.update_policy(f.viewer, &policy).await.unwrap();
    let mut row = fetch_item(&f.db, f.item).await;
    row.tags = Some("blocked".into());
    save_item(&f.db, &row).await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
    row.tags = None;
    save_item(&f.db, &row).await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_ok()
    );
    // Independent old-writer policy revocation must bypass the ordinary TTL.
    sqlx::query(r#"UPDATE "Permissions" SET "Value"=0 WHERE "UserId"=? AND "Kind"=?"#)
        .bind(guid_to_db(f.viewer))
        .bind(i32::from(
            ferrofin_db::enums::PermissionKind::EnableMediaPlayback,
        ))
        .execute(&f.independent_writer().await)
        .await
        .unwrap();
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_pin_deletion_and_rekey_refuse_even_known_rows() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let known = fetch_item(&f.db, f.item).await;
    let writer = f.independent_writer().await;
    sqlx::query(r#"DELETE FROM "BaseItems" WHERE "Id"=?"#)
        .bind(guid_to_db(f.item))
        .execute(&writer)
        .await
        .unwrap();
    assert!(
        pin.get_static_media_sources_for(&known, false, None)
            .await
            .is_err()
    );
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
    let mut rekeyed = known;
    rekeyed.id = guid_to_db(Uuid::new_v4());
    save_item(&f.db, &rekeyed).await;
    assert!(
        pin.get_static_media_sources_for(&rekeyed, false, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_pin_failed_and_canceled_capture_release_bounded_admission() {
    let f = Fixture::new().await;
    for _ in 0..40 {
        assert!(
            f.manager
                .pin_local_source(f.item, &f.item.to_string(), "/wrong.wav", f.viewer)
                .await
                .is_err()
        );
        assert!(
            f.manager
                .pin_local_source(f.item, "not-a-guid", &f.original, f.viewer)
                .await
                .is_err()
        );
        assert!(
            f.manager
                .pin_local_source(f.item, &f.item.to_string(), &f.original, Uuid::new_v4())
                .await
                .is_err()
        );
    }
    let held_writer = f.db.writer().acquire().await.unwrap();
    {
        let id = f.item.to_string();
        let canceled = f
            .manager
            .pin_local_source(f.item, &id, &f.original, f.viewer);
        tokio::pin!(canceled);
        assert!(futures_util::poll!(&mut canceled).is_pending());
    }
    drop(held_writer);
    let mut pins = Vec::new();
    for _ in 0..ferrofin_db::source_pin::LOCAL_SOURCE_PIN_LIMIT {
        pins.push(f.pin().await);
    }
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
    let kept = Arc::clone(&pins[0]);
    pins.remove(0);
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
    drop(kept);
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_ok()
    );
}

#[tokio::test]
#[ignore = "requires installed ffmpeg; run explicitly on the qualified build host"]
async fn source_pin_ffmpeg_opens_the_retained_input_after_path_replacement() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let input = pin
        .get_stream_media_source(f.item, None, None)
        .await
        .unwrap()
        .path
        .unwrap();
    std::fs::rename(&f.original, f.directory.path().join("unlinked.wav")).unwrap();
    std::fs::write(&f.original, wav([1, 0])).unwrap();
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i", &input, "-f", "md5", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let old = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            f.directory.path().join("unlinked.wav").to_str().unwrap(),
            "-f",
            "md5",
            "-",
        ])
        .output()
        .unwrap();
    assert!(old.status.success());
    assert_eq!(output.stdout, old.stdout);
}

#[tokio::test]
async fn source_pin_alternate_version_ids_and_relations_cannot_substitute_input() {
    let f = Fixture::new().await;
    let alternate = Uuid::new_v4();
    seed_item(&f.db, alternate, BaseItemKind::Audio).await;
    let mut row = fetch_item(&f.db, alternate).await;
    row.path = Some(f.original.clone());
    row.media_type = Some("Audio".into());
    row.run_time_ticks = Some(10_000_000);
    row.primary_version_id = Some(guid_to_db(f.item));
    save_item(&f.db, &row).await;
    let streams = FerrofinMediaStreamRepository::new(f.db.clone());
    streams
        .save_media_streams(
            alternate,
            &f.db
                .capture_local_source(f.item, f.item, &f.original)
                .await
                .unwrap()
                .streams,
        )
        .await
        .unwrap();
    assert_eq!(
        f.manager
            .get_static_media_sources(f.item, false, None)
            .await
            .unwrap()
            .len(),
        2
    );
    let spelled = alternate.hyphenated().to_string().to_ascii_uppercase();
    let pin = f
        .manager
        .pin_local_source(f.item, &spelled, &f.original, f.viewer)
        .await
        .unwrap();
    let selected = pin
        .get_stream_media_source(f.item, Some(&spelled), None)
        .await
        .unwrap();
    assert_eq!(
        selected.id.as_deref(),
        Some(alternate.simple().to_string().as_str())
    );
    let input = selected.path.unwrap();
    // Unlink the version and change both paths through an independent handle.
    sqlx::query(r#"UPDATE "BaseItems" SET "PrimaryVersionId"=NULL,"Path"='https://untrusted.example/other' WHERE "Id" IN (?,?)"#)
        .bind(guid_to_db(f.item)).bind(guid_to_db(alternate)).execute(&f.independent_writer().await).await.unwrap();
    let resolved = pin
        .get_stream_media_source(f.item, Some(&alternate.simple().to_string()), None)
        .await
        .unwrap();
    assert_eq!(resolved.path.as_deref(), Some(input.as_str()));
    assert_eq!(
        pin.get_stream_media_source(alternate, None, None)
            .await
            .unwrap()
            .path
            .as_deref(),
        Some(input.as_str())
    );
    assert!(
        pin.get_static_media_sources(f.item, false, Some(Uuid::new_v4()))
            .await
            .is_err()
    );
    assert!(
        f.manager
            .pin_local_source(f.item, &alternate.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_pin_current_folders_parental_rating_and_schedule_can_revoke() {
    let f = Fixture::new().await;
    let folder = Uuid::new_v4();
    seed_item(&f.db, folder, BaseItemKind::CollectionFolder).await;
    let mut row = fetch_item(&f.db, f.item).await;
    row.parent_id = Some(guid_to_db(folder));
    row.top_parent_id = Some(guid_to_db(folder));
    save_item(&f.db, &row).await;
    let pin = f.pin().await;
    let users = crate::user_manager::FerrofinUserManager::new(f.db.clone());
    let user = users.get_user_by_id(f.viewer).await.unwrap().unwrap();
    let mut policy = users
        .get_user_dto(&user, None)
        .await
        .unwrap()
        .policy
        .unwrap();
    crate::user_entity_ext::set_preference(
        &f.db,
        &user.id,
        ferrofin_db::enums::PreferenceKind::BlockedMediaFolders,
        &[folder.to_string()],
    )
    .await
    .unwrap();
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
    crate::user_entity_ext::set_preference(
        &f.db,
        &user.id,
        ferrofin_db::enums::PreferenceKind::BlockedMediaFolders,
        &[],
    )
    .await
    .unwrap();
    policy.max_parental_rating = Some(1);
    users.update_policy(f.viewer, &policy).await.unwrap();
    row.official_rating = Some("R".into());
    save_item(&f.db, &row).await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
    row.official_rating = None;
    save_item(&f.db, &row).await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_ok()
    );
    // An empty interval can never authorize the current time.
    sqlx::query(r#"INSERT INTO "AccessSchedules" ("UserId","DayOfWeek","StartHour","EndHour") VALUES (?,0,25,26)"#)
        .bind(guid_to_db(f.viewer)).execute(&f.independent_writer().await).await.unwrap();
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_pin_original_visibility_is_checked_after_current_metadata_changes() {
    let f = Fixture::new().await;
    let mut row = fetch_item(&f.db, f.item).await;
    row.tags = Some("original-classification".into());
    save_item(&f.db, &row).await;
    let pin = f.pin().await;
    row.tags = None;
    save_item(&f.db, &row).await;
    crate::user_entity_ext::set_preference(
        &f.db,
        &guid_to_db(f.viewer),
        ferrofin_db::enums::PreferenceKind::BlockedTags,
        &["original-classification".into()],
    )
    .await
    .unwrap();
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_pin_rejects_remote_shortcut_live_unbounded_and_external_inputs() {
    let f = Fixture::new().await;
    let mut row = fetch_item(&f.db, f.item).await;
    for path in ["https://untrusted.example/remote.mp3", "/missing.wav"] {
        row.path = Some(path.into());
        save_item(&f.db, &row).await;
        assert!(
            f.manager
                .pin_local_source(f.item, &f.item.to_string(), path, f.viewer)
                .await
                .is_err()
        );
    }
    row.path = Some(f.original.clone());
    row.run_time_ticks = None;
    save_item(&f.db, &row).await;
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
    row.run_time_ticks = Some(10_000_000);
    row.data =
        Some(r#"{"IsShortcut":true,"ShortcutPath":"https://untrusted.example/movie"}"#.into());
    save_item(&f.db, &row).await;
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
    row.data = Some("{}".into());
    row.type_ = crate::item_type_lookup::stored_type_name(BaseItemKind::LiveTvChannel)
        .unwrap()
        .into();
    save_item(&f.db, &row).await;
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
    row.type_ = crate::item_type_lookup::stored_type_name(BaseItemKind::Audio)
        .unwrap()
        .into();
    save_item(&f.db, &row).await;
    let streams = FerrofinMediaStreamRepository::new(f.db.clone());
    streams
        .save_media_streams(
            f.item,
            &[MediaStreamInfoEntity {
                stream_type: 0,
                is_external: true,
                path: Some("/outside.wav".into()),
                ..Default::default()
            }],
        )
        .await
        .unwrap();
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
    let shortcut = f.directory.path().join("shortcut.strm");
    std::fs::write(&shortcut, "https://untrusted.example/movie").unwrap();
    row.path = Some(shortcut.to_str().unwrap().into());
    save_item(&f.db, &row).await;
    assert!(
        f.manager
            .pin_local_source(
                f.item,
                &f.item.to_string(),
                shortcut.to_str().unwrap(),
                f.viewer
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn source_pin_backup_and_an_independent_database_owner_remain_usable() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let backup = f.directory.path().join("backup.db");
    f.db.snapshot_to(&backup).await.unwrap();
    let restored = Database::connect(&format!("sqlite://{}", backup.display()))
        .await
        .unwrap();
    assert_eq!(
        fetch_item(&restored, f.item).await.path.as_deref(),
        Some(f.original.as_str())
    );
    let other = Database::connect(&format!(
        "sqlite://{}",
        f.directory.path().join("fixture.db").display()
    ))
    .await
    .unwrap();
    let mut row = fetch_item(&other, f.item).await;
    row.path = Some("/independent-replacement.wav".into());
    save_item(&other, &row).await;
    let input = pin
        .get_stream_media_source(f.item, None, None)
        .await
        .unwrap()
        .path
        .unwrap();
    assert_eq!(tokio::fs::read(input).await.unwrap(), wav([0, 0]));
    let persisted_pins: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name LIKE '%SourcePin%'")
            .fetch_one(other.pool())
            .await
            .unwrap();
    assert_eq!(
        persisted_pins, 0,
        "backup/restart must not retain process leases"
    );
}

#[test]
#[ignore = "subprocess fixture invoked by source_pin_process_exit_needs_no_lease_cleanup"]
fn source_pin_process_exit_child() {
    let database = std::env::var("BLUEFIN_SOURCE_PIN_FIXTURE_DB").unwrap();
    let path = std::env::var("BLUEFIN_SOURCE_PIN_FIXTURE_PATH").unwrap();
    let item = Uuid::parse_str(&std::env::var("BLUEFIN_SOURCE_PIN_FIXTURE_ITEM").unwrap()).unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let db = Database::connect(&format!("sqlite://{database}"))
            .await
            .unwrap();
        let _opened = db.capture_local_source(item, item, &path).await.unwrap();
        // Exit while the owner and pools are live. Rust destructors do not run.
        std::process::exit(0);
    });
}

#[tokio::test]
async fn source_pin_process_exit_needs_no_lease_cleanup() {
    let f = Fixture::new().await;
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "media_source_manager::source_pin_tests::source_pin_process_exit_child",
        ])
        .env(
            "BLUEFIN_SOURCE_PIN_FIXTURE_DB",
            f.directory.path().join("fixture.db"),
        )
        .env("BLUEFIN_SOURCE_PIN_FIXTURE_PATH", &f.original)
        .env("BLUEFIN_SOURCE_PIN_FIXTURE_ITEM", f.item.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    f.replace("/after-crash.wav").await;
    assert_eq!(
        fetch_item(&f.db, f.item).await.path.as_deref(),
        Some("/after-crash.wav")
    );
}

struct UnsupportedPins;

#[async_trait]
impl MediaSourceManager for UnsupportedPins {
    async fn get_media_streams(&self, _: Uuid) -> Result<Vec<MediaStream>, ServiceError> {
        unreachable!()
    }
    async fn get_media_attachments(&self, _: Uuid) -> Result<Vec<MediaAttachment>, ServiceError> {
        unreachable!()
    }
    async fn get_playback_media_sources(
        &self,
        _: Uuid,
        _: Uuid,
        _: bool,
        _: bool,
    ) -> Result<Vec<MediaSourceInfo>, ServiceError> {
        unreachable!()
    }
    async fn get_static_media_sources(
        &self,
        _: Uuid,
        _: bool,
        _: Option<Uuid>,
    ) -> Result<Vec<MediaSourceInfo>, ServiceError> {
        unreachable!()
    }
    async fn open_live_stream(
        &self,
        _: &LiveStreamRequest,
    ) -> Result<MediaSourceInfo, ServiceError> {
        unreachable!()
    }
    async fn get_live_stream(&self, _: &str) -> Result<MediaSourceInfo, ServiceError> {
        unreachable!()
    }
    async fn close_live_stream(&self, _: &str) -> Result<(), ServiceError> {
        unreachable!()
    }
}

#[tokio::test]
async fn source_pin_custom_managers_default_to_unsupported() {
    assert!(matches!(
        UnsupportedPins
            .pin_local_source(Uuid::nil(), "source", "/local.wav", Uuid::nil())
            .await,
        Err(ServiceError::Unavailable(_))
    ));
}

#[tokio::test]
async fn source_pin_resolution_survives_overlapping_old_writer_replacements() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let writer = f.independent_writer().await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let ready = Arc::clone(&barrier);
    let item = f.item;
    let replace = tokio::spawn(async move {
        ready.wait().await;
        for n in 0..32 {
            sqlx::query(r#"UPDATE "BaseItems" SET "Path"=? WHERE "Id"=?"#)
                .bind(if n % 2 == 0 {
                    "/replacement/other.wav"
                } else {
                    "https://untrusted.example/replacement.mp3"
                })
                .bind(guid_to_db(item))
                .execute(&writer)
                .await
                .unwrap();
            tokio::task::yield_now().await;
        }
    });
    barrier.wait().await;
    for _ in 0..12 {
        let source = pin
            .get_stream_media_source(f.item, None, None)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(source.path.unwrap()).await.unwrap(),
            wav([0, 0])
        );
        let source = pin
            .get_playback_media_sources(f.item, f.viewer, true, true)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(
            tokio::fs::read(source.path.unwrap()).await.unwrap(),
            wav([0, 0])
        );
    }
    replace.await.unwrap();
}

#[tokio::test]
#[ignore = "requires installed ffmpeg; run explicitly on the qualified build host"]
async fn source_pin_ffmpeg_video_uses_the_captured_generation() {
    let f = Fixture::new().await;
    let video = f.directory.path().join("synthetic.mp4");
    let generated = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=16x16:r=10:d=1",
            "-c:v",
            "libx264",
            "-threads",
            "1",
            "-y",
            video.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let mut row = fetch_item(&f.db, f.item).await;
    row.type_ = crate::item_type_lookup::stored_type_name(BaseItemKind::Movie)
        .unwrap()
        .into();
    row.media_type = Some("Video".into());
    row.path = Some(video.to_str().unwrap().into());
    save_item(&f.db, &row).await;
    FerrofinMediaStreamRepository::new(f.db.clone())
        .save_media_streams(
            f.item,
            &[MediaStreamInfoEntity {
                stream_type: 1,
                codec: Some("h264".into()),
                width: Some(16),
                height: Some(16),
                ..Default::default()
            }],
        )
        .await
        .unwrap();
    let pin = f
        .manager
        .pin_local_source(
            f.item,
            &f.item.to_string(),
            video.to_str().unwrap(),
            f.viewer,
        )
        .await
        .unwrap();
    let source = pin
        .get_playback_media_sources(f.item, f.viewer, true, true)
        .await
        .unwrap()
        .remove(0);
    let frozen = source.path.unwrap();
    assert!(source.supports_transcoding);
    assert_eq!(source.media_streams[0].stream_type, MediaStreamType::Video);
    f.replace("https://untrusted.example/other.mp4").await;
    let old = f.directory.path().join("old.mp4");
    std::fs::rename(&video, &old).unwrap();
    std::fs::write(&video, b"replacement must never be decoded").unwrap();
    let digest = |input: &str| {
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i", input, "-f", "md5", "-"])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        decoded.stdout
    };
    assert_eq!(digest(&frozen), digest(old.to_str().unwrap()));
    crate::user_entity_ext::set_permission(
        &f.db,
        &guid_to_db(f.viewer),
        ferrofin_db::enums::PermissionKind::EnableVideoPlaybackTranscoding,
        false,
    )
    .await
    .unwrap();
    assert!(
        !pin.get_playback_media_sources(f.item, f.viewer, true, true)
            .await
            .unwrap()[0]
            .supports_transcoding
    );
}

#[tokio::test]
async fn source_pin_actual_scan_file_facts_and_rekey_boundaries_preserve_or_refuse() {
    use ferrofin_traits::persistence::{ItemPersistenceService, ItemRekey};
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let known = fetch_item(&f.db, f.item).await;
    let persistence =
        crate::item_persistence_service::FerrofinItemPersistenceService::new(f.db.clone());
    let mut updated = known.clone();
    updated.path = Some("/scan/replacement.wav".into());
    assert!(persistence.update_file_facts(&updated, None).await.unwrap());
    let source = pin
        .get_stream_media_source(f.item, None, None)
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read(source.path.unwrap()).await.unwrap(),
        wav([0, 0])
    );
    let movement = ItemRekey {
        from: f.item,
        to: Uuid::new_v4(),
        type_name: known.type_.clone(),
        dirs: Vec::new(),
        merged: Vec::new(),
        extra: false,
    };
    assert_eq!(
        persistence
            .rekey_items(std::slice::from_ref(&movement))
            .await
            .unwrap(),
        *std::slice::from_ref(&movement)
    );
    assert!(
        pin.get_static_media_sources_for(&known, false, None)
            .await
            .is_err()
    );
    assert!(
        pin.get_stream_media_source(movement.to, None, None)
            .await
            .is_err()
    );
    assert_eq!(
        persistence.delete_items(&[movement.to]).await.unwrap(),
        [movement.to]
    );
}

#[test]
fn source_pin_cancellation_after_begin_releases_the_actual_sqlite_lock() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let f = Fixture::new().await;
        let writer = f.independent_writer().await;
        sqlx::query("PRAGMA busy_timeout=0")
            .execute(&writer)
            .await
            .unwrap();
        // Occupy the blocking pool so compare/read reaches the actual open
        // boundary while its immediate transaction is still held.
        let (release, held) = std::sync::mpsc::channel();
        let (ready, started) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            ready.send(()).unwrap();
            let _ = held.recv();
        });
        started.await.unwrap();
        let manager = f.manager.clone();
        let (item, viewer, path) = (f.item, f.viewer, f.original.clone());
        let capture = tokio::spawn(async move {
            manager
                .pin_local_source(item, &item.to_string(), &path, viewer)
                .await
        });
        let observed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match sqlx::query("BEGIN IMMEDIATE").execute(&writer).await {
                    Ok(_) => {
                        sqlx::query("ROLLBACK").execute(&writer).await.unwrap();
                    }
                    Err(error)
                        if error
                            .as_database_error()
                            .and_then(sqlx::error::DatabaseError::code)
                            .is_some_and(|code| code == "5") =>
                    {
                        break;
                    }
                    Err(error) => panic!("unexpected writer error: {error}"),
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        capture.abort();
        let Err(error) = capture.await else {
            panic!("capture unexpectedly completed");
        };
        assert!(error.is_cancelled());
        release.send(()).unwrap();
        blocker.await.unwrap();
        observed.expect("capture must actually hold SQLite's write lock");
        sqlx::query("PRAGMA busy_timeout=1000")
            .execute(&writer)
            .await
            .unwrap();
        sqlx::query(r#"UPDATE "BaseItems" SET "Name"='after cancellation' WHERE "Id"=?"#)
            .bind(guid_to_db(f.item))
            .execute(&writer)
            .await
            .unwrap();
        assert!(
            f.manager
                .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
                .await
                .is_ok()
        );
    });
}

#[tokio::test]
async fn source_pin_busy_old_writer_fails_promptly_without_holding_normal_writer() {
    let f = Fixture::new().await;
    let old = f.independent_writer().await;
    let transaction = old.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let capture = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer),
    )
    .await;
    assert!(
        capture
            .expect("admission must not inherit the writer's 30-second busy wait")
            .is_err()
    );
    transaction.rollback().await.unwrap();
    let pin = f.pin().await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn source_pin_original_library_removal_and_reclassification_refuse() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let original = fetch_item(&f.db, f.library).await;
    let mut changed = original.clone();
    changed.data = Some(r#"{"PhysicalLocationsList":["/different/library"]}"#.into());
    save_item(&f.db, &changed).await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
    save_item(&f.db, &original).await;
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_ok()
    );
    sqlx::query(r#"DELETE FROM "BaseItems" WHERE "Id"=?"#)
        .bind(guid_to_db(f.library))
        .execute(&f.independent_writer().await)
        .await
        .unwrap();
    assert!(
        pin.get_stream_media_source(f.item, None, None)
            .await
            .is_err()
    );
    assert!(
        f.manager
            .pin_local_source(f.item, &f.item.to_string(), &f.original, f.viewer)
            .await
            .is_err()
    );
}
