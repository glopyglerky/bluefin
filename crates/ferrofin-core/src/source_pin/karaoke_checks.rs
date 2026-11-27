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

#[tokio::test]
async fn lyric_sidecar_metadata_never_becomes_a_decoder_input() {
    use std::io::Read as _;
    let f = Fixture::new().await;
    let repository = FerrofinMediaStreamRepository::new(f.db.clone());
    let mut rows = vec![
        MediaStreamInfoEntity {
            stream_index: 0,
            stream_type: 0,
            codec: Some("pcm_s16le".into()),
            sample_rate: Some(8000),
            channels: Some(1),
            ..Default::default()
        },
        MediaStreamInfoEntity {
            stream_index: 1,
            stream_type: 5,
            codec: Some("lrc".into()),
            is_external: true,
            path: Some("https://untrusted.example/never-open-this.lrc".into()),
            ..Default::default()
        },
    ];
    repository.save_media_streams(f.item, &rows).await.unwrap();
    let ordinary = f.manager.get_media_streams(f.item).await.unwrap();
    assert!(ordinary.iter().any(|stream| {
        stream.stream_type == ferrofin_model::entities::MediaStreamType::Lyric
            && stream.path.is_some()
    }));

    let pin = f.pin().await;
    let mut evidence = pin.current_local_audio(f.item, f.viewer).await.unwrap();
    let mut bytes = Vec::new();
    evidence.input.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, wav([0, 0]));
    let execution = pin
        .get_stream_media_source(f.item, None, None)
        .await
        .unwrap();
    assert_eq!(execution.media_streams.len(), 1);
    assert!(execution.media_streams.iter().all(|stream| {
        stream.stream_type == ferrofin_model::entities::MediaStreamType::Audio
            && !stream.is_external
            && stream.path.is_none()
    }));
    assert_eq!(std::fs::read(execution.path.unwrap()).unwrap(), wav([0, 0]));
    assert_eq!(f.manager.get_media_streams(f.item).await.unwrap().len(), 2);

    // Ignoring a lyric as input does not erase it from the frozen metadata.
    rows[1].path = Some("/changed-sidecar.lrc".into());
    repository.save_media_streams(f.item, &rows).await.unwrap();
    assert!(pin.current_local_audio(f.item, f.viewer).await.is_err());
}

#[tokio::test]
async fn external_execution_streams_still_fail_alongside_embedded_audio() {
    let f = Fixture::new().await;
    let repository = FerrofinMediaStreamRepository::new(f.db.clone());
    // Audio, video, subtitle, embedded image, data, and unknown stream types.
    for stream_type in [0, 1, 2, 3, 4, 99] {
        repository
            .save_media_streams(
                f.item,
                &[
                    MediaStreamInfoEntity {
                        stream_index: 0,
                        stream_type: 0,
                        codec: Some("pcm_s16le".into()),
                        sample_rate: Some(8000),
                        channels: Some(1),
                        ..Default::default()
                    },
                    MediaStreamInfoEntity {
                        stream_index: 1,
                        stream_type,
                        is_external: true,
                        path: Some("https://untrusted.example/never-open-this-input".into()),
                        ..Default::default()
                    },
                ],
            )
            .await
            .unwrap();
        assert!(
            f.manager
                .pin_local_source(f.item, &f.item.simple().to_string(), &f.original, f.viewer,)
                .await
                .is_err(),
            "external stream type {stream_type}"
        );
    }
}
