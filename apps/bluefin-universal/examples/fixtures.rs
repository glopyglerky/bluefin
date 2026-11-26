//! Generate only original test patterns and synthesized tones. Requires ffmpeg
//! on PATH. Outputs belong in a disposable, task-owned build/cache directory.
use std::{path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(root.join("hls")).unwrap();
    let inputs = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=24",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "40",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-g",
        "48",
        "-c:a",
        "aac",
        "-b:a",
        "64k",
        "-af",
        "volume=0.05",
    ];
    let mut mp4 = Command::new("ffmpeg");
    mp4.args(inputs)
        .args(["-movflags", "+faststart"])
        .arg(root.join("pattern.mp4"));
    assert!(mp4.status().unwrap().success());
    let mut hls = Command::new("ffmpeg");
    hls.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(root.join("pattern.mp4"))
        .args([
            "-c",
            "copy",
            "-f",
            "hls",
            "-hls_time",
            "2",
            "-hls_playlist_type",
            "vod",
            "-hls_segment_filename",
        ])
        .arg(root.join("hls/segment%03d.ts"))
        .arg(root.join("hls/vod.m3u8"));
    assert!(hls.status().unwrap().success());
    let mut audio = Command::new("ffmpeg");
    audio
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=48000",
            "-t",
            "2",
            "-af",
            "volume=0.05",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(root.join("tone.wav"));
    assert!(audio.status().unwrap().success());
}
