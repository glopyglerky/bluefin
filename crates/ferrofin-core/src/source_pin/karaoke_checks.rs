//! The stricter preparation accessor never changes frozen TV dispatch.
use super::*;
#[tokio::test]
async fn current_audio_refuses_changed_path_while_tv_retains_opened_generation() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let before = pin.current_local_audio(f.item, f.viewer).await.unwrap();
    let other = f.directory.path().join("new.wav");
    std::fs::write(&other, wav([1, 0])).unwrap();
    f.replace(other.to_str().unwrap()).await;
    assert!(pin.current_local_audio(f.item, f.viewer).await.is_err());
    let frozen = pin
        .get_stream_media_source(f.item, None, None)
        .await
        .unwrap();
    assert_eq!(std::fs::read(frozen.path.unwrap()).unwrap(), wav([0, 0]));
    assert_eq!(before.generation.len(), 64);
}
#[tokio::test]
async fn current_audio_fingerprints_in_place_changes_and_rejects_stream_generation() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    let before = pin
        .current_local_audio(f.item, f.viewer)
        .await
        .unwrap()
        .generation;
    std::fs::write(&f.original, wav([2, 0])).unwrap();
    let after = pin
        .current_local_audio(f.item, f.viewer)
        .await
        .unwrap()
        .generation;
    assert_ne!(before, after);
    f.change_audio_codec("changed").await;
    assert!(pin.current_local_audio(f.item, f.viewer).await.is_err());
}
#[tokio::test]
async fn current_audio_rechecks_permission_and_real_library_anchors() {
    let f = Fixture::new().await;
    let pin = f.pin().await;
    pin.current_local_audio(f.item, f.viewer).await.unwrap();
    f.change_permission(ferrofin_db::enums::PermissionKind::EnableAllFolders, false)
        .await;
    assert!(pin.current_local_audio(f.item, f.viewer).await.is_err());
}
