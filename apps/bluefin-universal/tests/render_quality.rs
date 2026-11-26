use bluefin_universal::mpv::Decoder;
use std::time::{Duration, Instant};

#[test]
fn render_dimensions_preserve_aspect_and_bound_memory() {
    use bluefin_universal::mpv::render_size;
    assert_eq!(render_size(1920.0, 1080.0), Some([1920, 1080]));
    assert_eq!(render_size(8000.0, 2000.0), Some([4096, 1024]));
    assert_eq!(render_size(1080.0, 3840.0), Some([608, 2160]));
    for (w, h) in [
        (f64::NAN, 10.0),
        (1.0, f64::INFINITY),
        (0.0, 0.0),
        (-1.0, 2.0),
    ] {
        assert_eq!(render_size(w, h), None);
    }
    let s = render_size(1e20, 1e20).unwrap();
    assert!(s[0] * s[1] <= 4096 * 2160);
}

#[test]
#[ignore = "requires original fixture and libmpv; exercises absent GL owner"]
fn unavailable_gl_owner_falls_back_and_retires() {
    bluefin_universal::mpv::enable_gl(true);
    let decoder = Decoder::start(
        std::env::var("BLUEFIN_QUALITY_FIXTURE").unwrap(),
        0.0,
        true,
        false,
    )
    .unwrap();
    bluefin_universal::mpv::enable_gl(false);
    let deadline = Instant::now() + Duration::from_secs(8);
    while decoder.frame().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(decoder.snapshot().render_path, "Software RGBA8");
    decoder.stop();
    while !decoder.snapshot().stopped {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires original task-owned 1080p fixture and libmpv"]
fn source_resolution_is_not_replaced_with_540p() {
    let decoder = Decoder::start(
        std::env::var("BLUEFIN_QUALITY_FIXTURE").unwrap(),
        0.0,
        true,
        false,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(frame) = decoder.frame() {
            decoder.stop();
            assert_eq!(
                frame.size,
                [1920, 1080],
                "1080p source must retain its spatial detail"
            );
            break;
        }
        assert!(Instant::now() < deadline, "no decoded video");
        std::thread::sleep(Duration::from_millis(20));
    }
    while !decoder.snapshot().stopped {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}
