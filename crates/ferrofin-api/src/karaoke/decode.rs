//! Bounded immutable encoded snapshot -> real Symphonia decode -> anti-aliased
//! Rubato resampling. No path/URL or executable is accepted here.
use super::{Reason, WorkResult};
use bluefin_karaoke_engine::Cancellation;
use rubato::{FftFixedIn, Resampler};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::DecoderOptions,
    errors::Error,
    formats::FormatOptions,
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::{Limit, MetadataOptions},
    probe::Hint,
};
const MAX_SOURCE: u64 = 128 * 1024 * 1024;
const MAX_PACKET: usize = 65536;

// Demuxer admission precedes upstream parsing/allocations. Only formats with
// bounded frame structures are enabled; MP4/AAC/ALAC/Vorbis are unavailable.
#[allow(clippy::too_many_lines)]
fn container_bounds(bytes: &[u8]) -> WorkResult<()> {
    let invalid = Reason::UnsupportedSource;
    if bytes.starts_with(b"RIFF") {
        if bytes.len() < 44
            || &bytes[8..12] != b"WAVE"
            || u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| invalid)?) as usize + 8
                != bytes.len()
        {
            return Err(invalid);
        }
        let mut position = 12usize;
        let mut chunks = 0;
        let mut format = false;
        let mut audio = false;
        let mut alignment = 0;
        while position < bytes.len() {
            chunks += 1;
            if chunks > 128 || position + 8 > bytes.len() {
                return Err(invalid);
            }
            let kind = &bytes[position..position + 4];
            let len = u32::from_le_bytes(
                bytes[position + 4..position + 8]
                    .try_into()
                    .map_err(|_| invalid)?,
            ) as usize;
            position += 8;
            let end = position
                .checked_add(len)
                .filter(|end| *end <= bytes.len())
                .ok_or(invalid)?;
            if kind == b"fmt " {
                if format || len != 16 {
                    return Err(invalid);
                }
                let data = &bytes[position..end];
                let sample_kind = u16::from_le_bytes([data[0], data[1]]);
                let channels = u16::from_le_bytes([data[2], data[3]]);
                let rate = u32::from_le_bytes(data[4..8].try_into().map_err(|_| invalid)?);
                let bits = u16::from_le_bytes([data[14], data[15]]);
                alignment = u16::from_le_bytes([data[12], data[13]]) as usize;
                if !(1..=2).contains(&channels)
                    || !(8000..=48000).contains(&rate)
                    || !matches!((sample_kind, bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64))
                    || alignment != usize::from(channels) * usize::from(bits / 8)
                    || u32::from_le_bytes(data[8..12].try_into().map_err(|_| invalid)?)
                        != rate * alignment as u32
                {
                    return Err(invalid);
                }
                format = true;
            } else if kind == b"data" {
                if !format || audio || len == 0 || !len.is_multiple_of(alignment) {
                    return Err(invalid);
                }
                audio = true;
            } else if len > 65536 {
                return Err(invalid);
            }
            position = end
                .checked_add(len % 2)
                .filter(|end| *end <= bytes.len())
                .ok_or(invalid)?;
        }
        if !format || !audio {
            return Err(invalid);
        }
    } else if bytes.starts_with(b"fLaC") {
        let mut position = 4usize;
        let mut metadata = 0usize;
        for block in 0..128 {
            if position + 4 > bytes.len() {
                return Err(invalid);
            }
            let kind = bytes[position];
            let metadata_type = kind & 0x7f;
            let len = (usize::from(bytes[position + 1]) << 16)
                | (usize::from(bytes[position + 2]) << 8)
                | usize::from(bytes[position + 3]);
            position += 4;
            if len > 65536 {
                return Err(invalid);
            }
            metadata += len;
            let end = position
                .checked_add(len)
                .filter(|end| *end <= bytes.len())
                .ok_or(invalid)?;
            if metadata > 1024 * 1024
                || (block == 0 && (metadata_type != 0 || len != 34))
                || (block > 0 && metadata_type == 0)
            {
                return Err(invalid);
            }
            if block == 0 {
                let info = &bytes[position..end];
                let frame =
                    (u32::from(info[7]) << 16) | (u32::from(info[8]) << 8) | u32::from(info[9]);
                if frame == 0 || frame > 4 * 1024 * 1024 {
                    return Err(invalid);
                }
            }
            position = end;
            if kind & 128 != 0 {
                return if position < bytes.len() {
                    Ok(())
                } else {
                    Err(invalid)
                };
            }
        }
        return Err(invalid);
    } else {
        let mut offset = 0usize;
        if bytes.starts_with(b"ID3") {
            if bytes.len() < 10 || bytes[6..10].iter().any(|v| v & 128 != 0) {
                return Err(invalid);
            }
            let size = bytes[6..10]
                .iter()
                .fold(0usize, |size, v| (size << 7) | usize::from(*v));
            if size > 65536 {
                return Err(invalid);
            }
            offset = 10 + size;
        }
        // Layer III, finite MPEG frames only. The demuxer must additionally
        // provide a real positive frame count (e.g. Xing/VBRI), checked below.
        if offset + 4 > bytes.len() || bytes[offset] != 255 || bytes[offset + 1] & 0xe6 != 0xe2 {
            return Err(invalid);
        }
        if bytes
            .windows(8)
            .rev()
            .take(32)
            .any(|tail| tail == b"APETAGEX")
        {
            return Err(invalid);
        }
    }
    Ok(())
}

pub(super) fn snapshot(mut file: File, cancel: &Cancellation) -> WorkResult<Vec<u8>> {
    use std::os::unix::fs::MetadataExt;
    let invalid = |_| Reason::UnsupportedSource;
    let before = file.metadata().map_err(invalid)?;
    if !before.is_file() || before.len() == 0 || before.len() > MAX_SOURCE {
        return Err(Reason::UnsupportedSource);
    }
    file.seek(SeekFrom::Start(0)).map_err(invalid)?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    let mut buf = vec![0; 65536].into_boxed_slice();
    loop {
        cancel.check().map_err(|_| Reason::PreparationFailed)?;
        let size = file.read(&mut buf).map_err(invalid)?;
        if size == 0 {
            break;
        }
        if bytes.len() as u64 + size as u64 > MAX_SOURCE {
            return Err(Reason::UnsupportedSource);
        }
        bytes.extend_from_slice(&buf[..size]);
    }
    let after = file.metadata().map_err(invalid)?;
    if before.len() != bytes.len() as u64
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(Reason::UnsupportedSource);
    }
    Ok(bytes)
}

// Keep the ordered format/frame/sample/resampler checks together for audit.
#[allow(clippy::too_many_lines)]
pub(super) fn canonical(
    encoded: Vec<u8>,
    duration: f64,
    cancel: &Cancellation,
) -> WorkResult<Vec<u8>> {
    if !duration.is_finite() || duration <= 0. {
        return Err(Reason::UnsupportedSource);
    }
    if duration > 300. {
        return Err(Reason::DurationUnsupported);
    }
    container_bounds(&encoded)?;
    let mss = MediaSourceStream::new(
        Box::new(Cursor::new(encoded)),
        MediaSourceStreamOptions::default(),
    );
    let probe = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions {
                limit_metadata_bytes: Limit::Maximum(65536),
                limit_visual_bytes: Limit::Maximum(0),
            },
        )
        .map_err(|_| Reason::UnsupportedSource)?;
    let mut format = probe.format;
    if format.tracks().len() != 1 {
        return Err(Reason::UnsupportedSource);
    }
    let track = format.default_track().ok_or(Reason::UnsupportedSource)?;
    let id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or(Reason::UnsupportedSource)?;
    let channels = track
        .codec_params
        .channels
        .ok_or(Reason::UnsupportedSource)?
        .count();
    let expected = track
        .codec_params
        .n_frames
        .filter(|n| *n > 0)
        .ok_or(Reason::UnsupportedSource)?;
    if !(8000..=48000).contains(&rate)
        || !(1..=2).contains(&channels)
        || track
            .codec_params
            .max_frames_per_packet
            .is_some_and(|n| n > MAX_PACKET as u64)
    {
        return Err(Reason::UnsupportedSource);
    }
    if expected > u64::from(rate) * 300 {
        return Err(Reason::DurationUnsupported);
    }
    if (expected as f64 / f64::from(rate) - duration).abs() > 0.05 {
        return Err(Reason::UnsupportedSource);
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions { verify: true })
        .map_err(|_| Reason::UnsupportedSource)?;
    let mut planar = [Vec::new(), Vec::new()];
    let mut frames = 0usize;
    let mut packets = 0u64;
    loop {
        cancel.check().map_err(|_| Reason::PreparationFailed)?;
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(_) => return Err(Reason::UnsupportedSource),
        };
        packets += 1;
        if packets > 300_000 || packet.track_id() != id || packet.data.len() > 4 * 1024 * 1024 {
            return Err(Reason::UnsupportedSource);
        }
        let audio = decoder
            .decode(&packet)
            .map_err(|_| Reason::UnsupportedSource)?;
        if audio.capacity() > MAX_PACKET
            || audio.spec().rate != rate
            || audio.spec().channels.count() != channels
        {
            return Err(Reason::UnsupportedSource);
        }
        if frames + audio.frames() > rate as usize * 300 {
            return Err(Reason::DurationUnsupported);
        }
        let mut samples = SampleBuffer::<f32>::new(audio.capacity() as u64, *audio.spec());
        samples.copy_interleaved_ref(audio);
        for frame in samples.samples().chunks_exact(channels) {
            if frame.iter().any(|v| !v.is_finite() || v.abs() > 1.0001) {
                return Err(Reason::UnsupportedSource);
            }
            planar[0].push(frame[0]);
            planar[1].push(frame[channels - 1]);
            frames += 1;
        }
    }
    if frames == 0 || frames as u64 != expected || decoder.finalize().verify_ok == Some(false) {
        return Err(Reason::UnsupportedSource);
    }
    let output_frames = ((frames as u64 * 44100 + u64::from(rate) / 2) / u64::from(rate)) as usize;
    let mut pcm = header(output_frames);
    let mut append = |left: &[f32], right: &[f32]| -> WorkResult<()> {
        for (&a, &b) in left.iter().zip(right) {
            for value in [a, b] {
                if !value.is_finite() {
                    return Err(Reason::UnsupportedSource);
                }
                // Resampling may overshoot by a few LSBs; PCM conversion clips
                // to representable amplitude and applies no normalization.
                pcm.extend_from_slice(
                    &((value * 32768.).round().clamp(-32768., 32767.) as i16).to_le_bytes(),
                );
            }
        }
        Ok(())
    };
    if rate == 44100 {
        for (a, b) in planar[0].chunks(4096).zip(planar[1].chunks(4096)) {
            cancel.check().map_err(|_| Reason::PreparationFailed)?;
            append(a, b)?;
        }
    } else {
        let mut resampler = FftFixedIn::<f32>::new(rate as usize, 44100, 1024, 2, 2)
            .map_err(|_| Reason::UnsupportedSource)?;
        let mut delay = resampler.output_delay();
        let mut input_offset = 0;
        let mut written = 0;
        while written < output_frames {
            cancel.check().map_err(|_| Reason::PreparationFailed)?;
            let count = resampler.input_frames_next();
            let mut input = vec![vec![0.; count]; 2];
            let available = count.min(frames.saturating_sub(input_offset));
            for (out, data) in input.iter_mut().zip(&planar) {
                out[..available].copy_from_slice(&data[input_offset..input_offset + available]);
            }
            input_offset += available;
            let output = resampler
                .process(&input, None)
                .map_err(|_| Reason::UnsupportedSource)?;
            let skip = delay.min(output[0].len());
            delay -= skip;
            let count = (output[0].len() - skip).min(output_frames - written);
            append(
                &output[0][skip..skip + count],
                &output[1][skip..skip + count],
            )?;
            written += count;
        }
    }
    if bluefin_karaoke::pcm(&pcm).map_err(|_| Reason::UnsupportedSource)?
        != (44100, 2, output_frames as u64)
    {
        return Err(Reason::UnsupportedSource);
    }
    Ok(pcm)
}
pub(super) fn header(frames: usize) -> Vec<u8> {
    let size = (frames * 4) as u32;
    let mut bytes = Vec::with_capacity(size as usize + 44);
    bytes.extend(b"RIFF");
    bytes.extend((size + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(44100u32.to_le_bytes());
    bytes.extend(176_400_u32.to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mono(rate: u32, frames: u32) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((frames * 2 + 36).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * 2).to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for i in 0..frames {
            bytes.extend(((i % 160) as i16 * 100 - 8000).to_le_bytes());
        }
        bytes
    }
    #[test]
    fn real_decode_and_resample_preserve_duration_and_stereo() {
        let wav = canonical(mono(48000, 48000), 1., &Cancellation::default()).unwrap();
        assert_eq!(bluefin_karaoke::pcm(&wav).unwrap(), (44100, 2, 44100));
        assert!(
            wav[44..]
                .as_chunks::<4>()
                .0
                .iter()
                .all(|frame| frame[..2] == frame[2..])
        );
        assert!(wav[44..].iter().any(|value| *value != 0));
    }
    #[test]
    fn malformed_duration_rate_channels_and_cancel_fail_closed() {
        assert!(
            canonical(
                b"encrypted or remote".to_vec(),
                1.,
                &Cancellation::default()
            )
            .is_err()
        );
        assert!(canonical(mono(48000, 48000), 0.5, &Cancellation::default()).is_err());
        assert!(canonical(mono(96000, 96000), 1., &Cancellation::default()).is_err());
        assert!(matches!(
            canonical(mono(8000, 8000), 301., &Cancellation::default()),
            Err(Reason::DurationUnsupported)
        ));
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(canonical(mono(8000, 8000), 1., &cancel).is_err());
        let mut truncated = mono(8000, 8000);
        truncated.truncate(truncated.len() - 2);
        assert!(canonical(truncated, 1., &Cancellation::default()).is_err());
    }
    #[test]
    fn untrusted_container_lengths_are_rejected_before_demuxer_allocation() {
        let mut wave = mono(8000, 8000);
        wave[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(container_bounds(&wave).is_err());
        assert!(container_bounds(b"fLaC\x80\xff\xff\xff").is_err());
        assert!(container_bounds(b"ID3\x04\x00\x00\x7f\x7f\x7f\x7f").is_err());
        assert!(container_bounds(b"\0\0\0\x10ftypM4A encrypted or unsupported").is_err());
    }
}
