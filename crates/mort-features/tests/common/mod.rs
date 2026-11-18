//! Test signals shared by the integration tests and the band harness.

#![allow(dead_code, reason = "each test binary uses a different subset")]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test signal generation"
)]

use std::f64::consts::PI;

use mort_features::{Analyzer, Frame};
use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// A sine of `amplitude` at `freq`, `secs` long, starting at `phase`.
pub fn sine(rate: f64, freq: f64, amplitude: f64, phase: f64, secs: f64) -> Vec<f32> {
    let n = (rate * secs).round() as usize;
    (0..n)
        .map(|i| (amplitude * (2.0 * PI * freq * i as f64 / rate + phase).sin()) as f32)
        .collect()
}

/// dBFS (sine peak) to amplitude.
pub fn amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// A small deterministic generator (xorshift64*).
pub struct Rng(pub u64);

impl Rng {
    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [-1, 1).
    pub fn bipolar(&mut self) -> f64 {
        2.0 * self.next_f64() - 1.0
    }
}

/// White noise of the given RMS.
pub fn white(rng: &mut Rng, rms: f64, n: usize) -> Vec<f32> {
    // Uniform on [-1, 1) has RMS 1/sqrt(3).
    (0..n)
        .map(|_| (rng.bipolar() * rms * 3f64.sqrt()) as f32)
        .collect()
}

/// A periodic pink multisine of `n` samples (a power of two): one
/// component per FFT bin from `lo` to `hi` Hz with power proportional to
/// 1/f and random phase, scaled to `rms`. Returns the signal and its
/// components as `(frequency, mean square)`.
pub fn pink(
    rng: &mut Rng,
    rate: f64,
    n: usize,
    lo: f64,
    hi: f64,
    rms: f64,
) -> (Vec<f32>, Vec<(f64, f64)>) {
    let df = rate / n as f64;
    let mut spec = vec![Complex::new(0.0f64, 0.0); n / 2 + 1];
    let mut comps = Vec::new();
    for (k, c) in spec.iter_mut().enumerate().take(n / 2).skip(1) {
        let f = k as f64 * df;
        if f >= lo && f <= hi {
            let ph = 2.0 * PI * rng.next_f64();
            let a = 1.0 / f.sqrt();
            *c = Complex::from_polar(a, ph);
            comps.push((f, a));
        }
    }
    let ifft = RealFftPlanner::<f64>::new().plan_fft_inverse(n);
    let mut out = ifft.make_output_vec();
    ifft.process(&mut spec, &mut out).unwrap();
    // A component of magnitude a in bin k becomes a cosine of amplitude 2a.
    let ms: f64 = comps.iter().map(|(_, a)| 2.0 * a * a).sum();
    let scale = rms / ms.sqrt();
    let comps = comps
        .into_iter()
        .map(|(f, a)| (f, 2.0 * (a * scale).powi(2)))
        .collect();
    (out.iter().map(|x| (x * scale) as f32).collect(), comps)
}

/// Runs `left` and `right` through a fresh analyzer in blocks of `block`.
pub fn run(rate: f64, left: &[f32], right: &[f32], block: usize) -> Vec<Frame> {
    let mut a = Analyzer::new(rate, block).unwrap();
    let mut frames = Vec::new();
    for (l, r) in left.chunks(block).zip(right.chunks(block)) {
        a.process(l, r, |f| frames.push(*f));
    }
    frames
}

/// Exact base-10 third-octave edges for band index `i` of the frame.
pub fn third_edges(i: usize) -> (f64, f64) {
    let c = 1_000.0 * 10f64.powf((i as f64 - 17.0) / 10.0);
    (c * 10f64.powf(-0.05), c * 10f64.powf(0.05))
}

/// Exact centre of band `i`.
pub fn third_centre(i: usize) -> f64 {
    1_000.0 * 10f64.powf((i as f64 - 17.0) / 10.0)
}

/// dB of a power ratio.
pub fn db(x: f64) -> f64 {
    10.0 * x.log10()
}
