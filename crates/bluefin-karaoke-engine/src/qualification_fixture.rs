//! Original additive-synthesis qualification music. No recorded/personal audio.
use crate::audio::RATE;
pub fn synth(seconds: f64) -> [Vec<f32>; 3] {
    let n = (seconds * RATE as f64) as usize;
    let mut mix = Vec::with_capacity(n * 2);
    let mut vocal = Vec::with_capacity(n * 2);
    let mut backing = Vec::with_capacity(n * 2);
    let mut phase = 0f64;
    let mut noise = 1u32;
    let mut low = 0f64;
    let notes = [
        196., 220., 246.9417, 261.6256, 293.6648, 261.6256, 220., 196.,
    ];
    for i in 0..n {
        let t = i as f64 / RATE as f64;
        let syllable = t % 0.7;
        let envelope =
            ((syllable / 0.09).min(1.) * ((0.62 - syllable) / 0.12).clamp(0., 1.)).max(0.);
        let fundamental = notes[((t / 0.7) as usize) % notes.len()]
            * (1. + 0.005 * (std::f64::consts::TAU * 5.4 * t).sin());
        phase += std::f64::consts::TAU * fundamental / RATE as f64;
        let mut v = 0.;
        for harmonic in 1..=30 {
            let f = fundamental * harmonic as f64;
            let weight = 0.15 * (-((f - 750.) / 220.).powi(2)).exp()
                + 0.08 * (-((f - 1200.) / 260.).powi(2)).exp()
                + 0.04 * (-((f - 2600.) / 400.).powi(2)).exp()
                + 0.025 / harmonic as f64;
            v += weight * (phase * harmonic as f64).sin();
        }
        v *= envelope;
        let beat = t % 0.5;
        let pluck = (-beat * 8.).exp();
        noise = noise.wrapping_mul(1664525).wrapping_add(1013904223);
        low = low * 0.85 + (f64::from(noise >> 8) / (1u32 << 24) as f64 * 2. - 1.) * 0.15;
        let drum = low * (-beat * 70.).exp() * 0.16;
        for ch in 0..2 {
            let mut a = 0.055 * (std::f64::consts::TAU * 98. * t).sin() + drum;
            for f in [130.8128, 164.8138, 195.9977] {
                for h in 1..=6 {
                    a += 0.045 * pluck / h as f64
                        * (std::f64::consts::TAU * f * h as f64 * t + ch as f64 * 0.18 * h as f64)
                            .sin();
                }
            }
            let fade = (t / 0.15).min(1.) * ((seconds - t) / 0.15).clamp(0., 1.);
            vocal.push((v * fade) as f32);
            backing.push((a * fade) as f32);
            mix.push(((v + a) * fade) as f32);
        }
    }
    [mix, vocal, backing]
}
pub fn snr(reference: &[f32], actual: &[f32], skip: usize) -> f64 {
    let mut signal = 0.;
    let mut error = 0.;
    for (r, a) in reference[skip..].iter().zip(&actual[skip..]) {
        signal += f64::from(*r).powi(2);
        error += f64::from(r - a).powi(2);
    }
    10. * (signal / error.max(1e-20)).log10()
}
pub fn si_sdr(reference: &[f32], actual: &[f32]) -> f64 {
    let dot: f64 = reference
        .iter()
        .zip(actual)
        .map(|(r, a)| f64::from(*r) * f64::from(*a))
        .sum();
    let power: f64 = reference.iter().map(|r| f64::from(*r).powi(2)).sum();
    let scale = dot / power.max(1e-20);
    let target = scale * scale * power;
    let error: f64 = reference
        .iter()
        .zip(actual)
        .map(|(r, a)| (f64::from(*a) - f64::from(*r) * scale).powi(2))
        .sum();
    10. * (target.max(1e-20) / error.max(1e-20)).log10()
}
