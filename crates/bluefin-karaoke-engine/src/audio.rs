use crate::{
    Cancellation, Error,
    model::{BINS, FRAMES, Model},
};
use rustfft::{FftPlanner, num_complex::Complex32};
pub const RATE: usize = 44100;
const WINDOW: usize = 4096;
const HOP: usize = 1024;
const FFT_BINS: usize = WINDOW / 2 + 1;

/// Intentionally matches the parent's canonical 44-byte PCM16 format, narrowed
/// to the trained stereo 44.1 kHz configuration. No paths, codecs or resampling.
pub(crate) fn validate(bytes: &[u8], max_seconds: u32) -> Result<usize, Error> {
    if bytes.len() < 48 || bytes.len() > 44 + RATE * 4 * max_seconds as usize {
        return Err(Error::InvalidInput);
    }
    let u16at = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
    let u32at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    if &bytes[..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || u32at(4) + 8 != bytes.len()
        || u32at(16) != 16
        || u16at(20) != 1
        || u16at(22) != 2
        || u32at(24) != RATE
        || u32at(28) != RATE * 4
        || u16at(32) != 4
        || u16at(34) != 16
        || &bytes[36..40] != b"data"
        || u32at(40) + 44 != bytes.len()
        || !(bytes.len() - 44).is_multiple_of(4)
    {
        return Err(Error::InvalidInput);
    }
    Ok((bytes.len() - 44) / 4)
}
pub(crate) fn samples(bytes: &[u8]) -> Vec<f32> {
    bytes[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.)
        .collect()
}
pub(crate) fn encode(samples: &[f32], cancel: &Cancellation) -> Result<Vec<u8>, Error> {
    if samples.is_empty() || !samples.len().is_multiple_of(2) || samples.len() > RATE * 2 * 300 {
        return Err(Error::InvalidOutput);
    }
    let size = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + size).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&size.to_le_bytes());
    for chunk in samples.chunks(32768) {
        cancel.check()?;
        for x in chunk {
            if !x.is_finite() || x.abs() > 1. {
                return Err(Error::InvalidOutput);
            }
            let quantized = (x * 32768.).round().clamp(-32768., 32767.) as i16;
            out.extend_from_slice(&quantized.to_le_bytes());
        }
    }
    Ok(out)
}

pub(crate) fn separate(
    model: &Model,
    samples: &[f32],
    cancel: &Cancellation,
) -> Result<[Vec<f32>; 2], Error> {
    let frames = samples.len() / 2;
    if frames == 0 || samples.len() != frames * 2 {
        return Err(Error::InvalidInput);
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW);
    let inverse = planner.plan_fft_inverse(WINDOW);
    let window: Vec<f32> = (0..WINDOW)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos())
        .collect();
    let n_stft = frames.div_ceil(HOP);
    let mut output = [vec![0f32; samples.len()], vec![0f32; samples.len()]];
    let mut spectrum = vec![Complex32::default(); 2 * FRAMES * FFT_BINS];
    let mut magnitude = vec![0f32; 2 * FRAMES * BINS];
    let mut buffer = vec![Complex32::default(); WINDOW];
    let mut scratch = vec![
        Complex32::default();
        fft.get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len())
    ];
    for patch in (0..n_stft).step_by(FRAMES) {
        cancel.check()?;
        spectrum.fill(Complex32::default());
        magnitude.fill(0.);
        let valid = (n_stft - patch).min(FRAMES);
        for ch in 0..2 {
            for frame in 0..valid {
                cancel.check()?;
                let start = (patch + frame) * HOP;
                for i in 0..WINDOW {
                    buffer[i] = Complex32::new(
                        if start + i < frames {
                            samples[(start + i) * 2 + ch] * window[i]
                        } else {
                            0.
                        },
                        0.,
                    );
                }
                fft.process_with_scratch(&mut buffer, &mut scratch);
                let s = (ch * FRAMES + frame) * FFT_BINS;
                spectrum[s..s + FFT_BINS].copy_from_slice(&buffer[..FFT_BINS]);
                let m = (ch * FRAMES + frame) * BINS;
                for bin in 0..BINS {
                    magnitude[m + bin] = buffer[bin].norm();
                }
            }
        }
        let estimates = model.infer(&magnitude, cancel)?;
        for (stem, out) in output.iter_mut().enumerate() {
            for ch in 0..2 {
                for frame in 0..valid {
                    cancel.check()?;
                    buffer.fill(Complex32::default());
                    let s = (ch * FRAMES + frame) * FFT_BINS;
                    let m = (ch * FRAMES + frame) * BINS;
                    for bin in 0..BINS {
                        let a = estimates[0][m + bin].powi(2);
                        let b = estimates[1][m + bin].powi(2);
                        let mask = (estimates[stem][m + bin].powi(2) + 5e-11) / (a + b + 1e-10);
                        if !mask.is_finite() || !(0.0..=1.000001).contains(&mask) {
                            return Err(Error::InvalidOutput);
                        }
                        buffer[bin] = spectrum[s + bin] * mask;
                        if bin > 0 {
                            buffer[WINDOW - bin] = buffer[bin].conj();
                        }
                    }
                    // Publisher mask_extension=zeros, including Nyquist.
                    inverse.process_with_scratch(&mut buffer, &mut scratch);
                    let start = (patch + frame) * HOP;
                    for i in 0..WINDOW.min(frames - start) {
                        out[(start + i) * 2 + ch] +=
                            buffer[i].re * window[i] * (2. / 3.) / WINDOW as f32;
                    }
                }
            }
        }
    }
    cancel.check()?;
    if output
        .iter()
        .flatten()
        .any(|x| !x.is_finite() || x.abs() > 1.)
    {
        return Err(Error::InvalidOutput);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_fail_closed() {
        let cancel = Cancellation::default();
        let wav = encode(&[0.; 200], &cancel).unwrap();
        assert_eq!(validate(&wav, 1).unwrap(), 100);
        for offset in [0, 4, 8, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
            let mut bad = wav.clone();
            bad[offset] ^= 1;
            assert!(validate(&bad, 1).is_err(), "offset={offset}");
        }
        assert!(validate(&wav[..wav.len() - 1], 1).is_err());
        assert!(encode(&[f32::NAN, 0.], &cancel).is_err());
        assert!(encode(&[1.1, 0.], &cancel).is_err());
        assert!(validate(&wav, 0).is_err());
        assert_eq!(samples(&wav), vec![0.; 200]);
    }
}
