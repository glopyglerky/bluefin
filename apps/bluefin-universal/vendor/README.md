# HLS fallback

`hls.light.min.js` is the unmodified hls.js 1.7.4 light distribution, downloaded from `https://cdn.jsdelivr.net/npm/hls.js@1.7.4/dist/hls.light.min.js`. Upstream source is [video-dev/hls.js v1.7.4](https://github.com/video-dev/hls.js/tree/v1.7.4). It is Apache-2.0 licensed; the upstream license is retained as `hls.LICENSE`.

SHA-256 bundle: `482cccb3a224554f64d8c2cf0d335dba4a3819bdb726afbfddfe15de4c277d82`.
SHA-256 license: `ca8773cf798c7ed997d4dd7c8e23c348699f8d5b7462636694cc14de6cda12db`.

The light build supplies HLS demuxing/transmuxing and MediaSource playback. Audio/subtitle choices are negotiated by Rust and rendered by the server in the selected stream; this package does not claim the light build's omitted alternate-track or subtitle modules. The bundle is served locally with the application, without runtime CDN access.
