# Original preview media

`BluefinOceanStudy.mp4` is a thirty-second original generated animation, encoded
as H264 at 640x360/30 fps. `BluefinAudioCheck.m4a` is a four-second original
440 Hz test tone, encoded as 16-bit/44.1 kHz mono ALAC. Neither file contains
personal media, library metadata, a recording, or a third-party asset.

The repository license applies. Source generation is in
`tools/GeneratePreview.swift` and `Tests/NativePlaybackFixtures.swift` in the
Apple app directory. Compile those two files together as a native host tool and
supply a new output folder. `NativePlaybackFixtures.makeAudio` regenerates the
tone. Container metadata can vary between encoder versions.
