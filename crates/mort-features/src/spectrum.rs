//! The spectral path: decimation, windowed FFTs, and band energies from
//! fractionally weighted bins.
//!
//! Bands are measured from two spectra per signal (ADR 0007 §1):
//!
//! - the *high* spectrum: a Hann-windowed FFT at the full rate over about
//!   85 ms (4096 points at 44.1 and 48 kHz), for bands from 800 Hz up, and
//!   for the centroid, flatness and onset strength;
//! - the *low* spectrum: the signal decimated to between 2.9 and 5.8 kHz,
//!   then a Hann-windowed FFT with bins no wider than 1.5 Hz (2048 points,
//!   683 ms, at 48 kHz), for bands up to 630 Hz.
//!
//! The crossover is the edge between the 630 Hz and 800 Hz third octaves,
//! 707.9 Hz, which is also the edge between the 500 Hz and 1 kHz octaves.
//! Each FFT window ends at the last sample of the frame.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::as_f64;
use crate::true_peak::{kaiser, sinc};

/// The edge between the low and the high spectrum, in Hz.
pub const CROSSOVER_HZ: f64 = 707.945_784_384_138;

/// The lowest rate the low spectrum is decimated to.
const LOW_RATE_MIN_HZ: f64 = 2_900.0;
/// The widest bin allowed in the low spectrum, in Hz.
const LOW_BIN_MAX_HZ: f64 = 1.5;
/// The shortest high-spectrum window, in seconds.
const HIGH_WINDOW_S: f64 = 0.085;
/// Stopband attenuation of the decimation filter, in dB.
const DECIMATOR_STOP_DB: f64 = 80.0;

/// Exact centre of third octave `n` (base 10, IEC 61260): 1000 · 10^(n/10).
pub fn third_octave_centre(n: i32) -> f64 {
    1_000.0 * 10f64.powf(f64::from(n) / 10.0)
}

/// The edges of third-octave band `n`.
pub fn third_octave_edges(n: i32) -> (f64, f64) {
    let c = third_octave_centre(n);
    (c * 10f64.powf(-0.05), c * 10f64.powf(0.05))
}

/// The edges of the octave band centred on third-octave band `3 n`.
pub fn octave_edges(n: i32) -> (f64, f64) {
    let c = third_octave_centre(3 * n);
    (c * 10f64.powf(-0.15), c * 10f64.powf(0.15))
}

/// Bin weights for one band: bin `first + i` contributes `weights[i]` of
/// its power.
struct BandWeights {
    first: usize,
    weights: Vec<f32>,
}

/// Which bins make up each band, for one FFT size and rate.
pub struct BandPlan {
    bands: Vec<BandWeights>,
}

impl BandPlan {
    /// Weights for `edges` on a spectrum of `n` points at `rate`. Each bin
    /// is taken to cover the frequencies within half a bin of its centre,
    /// and contributes to a band the fraction of that span inside the band.
    /// A sine on an edge between two bands therefore splits its energy
    /// equally between them, as a filter bank's -3 dB crossings would.
    pub fn new(edges: &[(f64, f64)], n: usize, rate: f64) -> Self {
        let df = rate / as_f64(n);
        let last = n / 2;
        let bands = edges
            .iter()
            .map(|&(lo, hi)| {
                let mut first = None;
                let mut weights = Vec::new();
                for k in 1..=last {
                    let b_lo = (as_f64(k) - 0.5) * df;
                    let b_hi = ((as_f64(k) + 0.5) * df).min(rate / 2.0);
                    let overlap = (b_hi.min(hi) - b_lo.max(lo)).max(0.0);
                    if overlap > 0.0 {
                        first.get_or_insert(k);
                        #[expect(clippy::cast_possible_truncation, reason = "weights are f32")]
                        weights.push((overlap / df) as f32);
                    } else if first.is_some() {
                        break;
                    }
                }
                BandWeights {
                    first: first.unwrap_or(0),
                    weights,
                }
            })
            .collect();
        Self { bands }
    }

    /// Band `i`'s energy in `power`.
    pub fn energy(&self, i: usize, power: &[f32]) -> f32 {
        let b = &self.bands[i];
        b.weights
            .iter()
            .zip(&power[b.first..])
            .map(|(w, p)| w * p)
            .sum()
    }
}

/// A sliding window of one signal and its power spectrum.
pub struct Spectrum {
    rate: f64,
    ring: Vec<f32>,
    pos: usize,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// One-sided power per bin, scaled so that the bins of a sine of
    /// amplitude `a` sum to `a² / 2`.
    pub power: Vec<f32>,
    norm: f32,
}

impl Spectrum {
    /// A spectrum of `n` points of a signal at `rate`.
    pub fn new(planner: &mut RealFftPlanner<f32>, n: usize, rate: f64) -> Self {
        let fft = planner.plan_fft_forward(n);
        let window: Vec<f32> = (0..n)
            .map(|i| {
                let v = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * as_f64(i) / as_f64(n)).cos();
                #[expect(clippy::cast_possible_truncation, reason = "window is f32")]
                let v = v as f32;
                v
            })
            .collect();
        let sum_sq: f64 = window.iter().map(|w| f64::from(*w) * f64::from(*w)).sum();
        #[expect(clippy::cast_possible_truncation, reason = "norm is f32")]
        let norm = (2.0 / (as_f64(n) * sum_sq)) as f32;
        Self {
            rate,
            ring: vec![0.0; n],
            pos: 0,
            window,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; n / 2 + 1],
            fft,
            norm,
        }
    }

    /// The FFT size.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// The width of one bin in Hz.
    pub fn bin_hz(&self) -> f64 {
        self.rate / as_f64(self.len())
    }

    /// Clears the window and spectrum.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.power.fill(0.0);
        self.pos = 0;
    }

    /// Appends samples to the window.
    pub fn push_slice(&mut self, mut x: &[f32]) {
        let n = self.ring.len();
        while !x.is_empty() {
            let take = x.len().min(n - self.pos);
            self.ring[self.pos..self.pos + take].copy_from_slice(&x[..take]);
            self.pos = (self.pos + take) % n;
            x = &x[take..];
        }
    }

    /// Appends one sample.
    pub fn push(&mut self, x: f32) {
        self.ring[self.pos] = x;
        self.pos = (self.pos + 1) % self.ring.len();
    }

    /// Recomputes [`Spectrum::power`] from the window.
    pub fn compute(&mut self) {
        let n = self.ring.len();
        let (newer, older) = self.ring.split_at(self.pos);
        let (head, tail) = self.input.split_at_mut(n - self.pos);
        for ((o, x), w) in head.iter_mut().zip(older).zip(&self.window) {
            *o = x * w;
        }
        for ((o, x), w) in tail.iter_mut().zip(newer).zip(&self.window[n - self.pos..]) {
            *o = x * w;
        }
        // Sizes match by construction, so this cannot fail.
        let _ = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch);
        for (p, c) in self.power.iter_mut().zip(&self.output) {
            *p = c.norm_sqr() * self.norm;
        }
        self.power[0] *= 0.5;
        let last = self.power.len() - 1;
        self.power[last] *= 0.5;
    }
}

/// A low-pass FIR decimator.
pub struct Decimator {
    factor: usize,
    taps: Vec<f32>,
    hist: Vec<f32>,
    /// Inputs to skip before the next output.
    phase: usize,
}

impl Decimator {
    /// A decimator by `factor` from `rate` for blocks of up to `max_block`.
    /// The filter is a Kaiser-windowed sinc with its cutoff at the output
    /// Nyquist frequency, passing [`CROSSOVER_HZ`] and stopping by
    /// `DECIMATOR_STOP_DB` everything that would alias below it.
    pub fn new(factor: usize, rate: f64, max_block: usize) -> Self {
        let out_rate = rate / as_f64(factor);
        let transition = (out_rate - 2.0 * CROSSOVER_HZ) / rate;
        let beta = 0.1102 * (DECIMATOR_STOP_DB - 8.7);
        let len = (DECIMATOR_STOP_DB - 7.95) / (2.285 * 2.0 * std::f64::consts::PI * transition);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small positive tap count"
        )]
        let len = (len.ceil() as usize + 1) | 1;
        let half = as_f64(len / 2);
        let cutoff = 0.5 / as_f64(factor);
        let raw: Vec<f64> = (0..len)
            .map(|i| {
                let t = as_f64(i) - half;
                sinc(2.0 * cutoff * t) * kaiser(t / (half + 1.0), beta)
            })
            .collect();
        let sum: f64 = raw.iter().sum();
        #[expect(clippy::cast_possible_truncation, reason = "taps are f32")]
        let taps = raw.iter().map(|t| (t / sum) as f32).collect();
        Self {
            factor,
            taps,
            hist: vec![0.0; len - 1 + max_block],
            phase: 0,
        }
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.hist.fill(0.0);
        self.phase = 0;
    }

    /// Filters `input` (no longer than the construction's `max_block`) and
    /// pushes every `factor`-th output into `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Spectrum) {
        let l = self.taps.len();
        let n = input.len();
        self.hist[l - 1..l - 1 + n].copy_from_slice(input);
        let mut i = self.phase;
        while i < n {
            let w = &self.hist[i..i + l];
            out.push(dot(&self.taps, w));
            i += self.factor;
        }
        self.phase = i - n;
        self.hist.copy_within(n..n + l - 1, 0);
    }
}

/// The dot product of `a` and `b`, with eight partial sums so that it
/// vectorises.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let chunks = a.len() / 8;
    for (ca, cb) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        for k in 0..8 {
            acc[k] += ca[k] * cb[k];
        }
    }
    let mut sum: f32 = acc.iter().sum();
    for (x, y) in a[chunks * 8..].iter().zip(&b[chunks * 8..]) {
        sum += x * y;
    }
    sum
}

/// The decimation factor and FFT sizes for a sample rate:
/// `(factor, low_n, high_n)`.
pub fn sizes(rate: f64) -> (usize, usize, usize) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rates are positive and bounded"
    )]
    let factor = ((rate / LOW_RATE_MIN_HZ).floor() as usize).max(1);
    let low_rate = rate / as_f64(factor);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rates are positive and bounded"
    )]
    let low_n = ((low_rate / LOW_BIN_MAX_HZ).ceil() as usize).next_power_of_two();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rates are positive and bounded"
    )]
    let high_n = ((rate * HIGH_WINDOW_S).ceil() as usize).next_power_of_two();
    (factor, low_n, high_n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_at_common_rates() {
        assert_eq!(sizes(48_000.0), (16, 2048, 4096));
        assert_eq!(sizes(44_100.0), (15, 2048, 4096));
        assert_eq!(sizes(96_000.0), (33, 2048, 8192));
    }

    #[test]
    fn crossover_is_a_band_edge() {
        assert!((third_octave_edges(-2).1 - CROSSOVER_HZ).abs() < 1e-9);
        assert!((octave_edges(-1).1 - CROSSOVER_HZ).abs() < 1e-9);
    }

    #[test]
    fn weights_partition_the_bins() {
        let edges: Vec<_> = (-17..=13).map(third_octave_edges).collect();
        let plan = BandPlan::new(&edges, 4096, 48_000.0);
        let mut power = vec![0.0f32; 2049];
        power[100] = 1.0;
        let total: f32 = (0..31).map(|i| plan.energy(i, &power)).sum();
        assert!((total - 1.0).abs() < 1e-5);
    }
}
