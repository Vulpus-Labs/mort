//! Feature extraction for mort's probe (ADR 0007 §1).
//!
//! [`Analyzer`] takes stereo blocks and emits one [`Frame`] per 50 ms of
//! audio. It allocates only in [`Analyzer::new`]; [`Analyzer::process`]
//! does no allocation, locking or system calls, so the probe can call it
//! from the audio thread (ADR 0005 §7).
//!
//! ```
//! use mort_features::Analyzer;
//!
//! let mut analyzer = Analyzer::new(48_000.0, 512).unwrap();
//! let left = [0.0f32; 512];
//! let right = [0.0f32; 512];
//! let mut frames = 0;
//! for _ in 0..10 {
//!     analyzer.process(&left, &right, |_frame| frames += 1);
//! }
//! assert_eq!(frames, 2); // 5120 samples hold two 2400-sample frames
//! ```

mod filters;
mod frame;
mod spectrum;
mod true_peak;

use std::fmt;

use realfft::RealFftPlanner;

pub use frame::{
    FRAME_BYTES, Frame, OCTAVE_CENTRES_HZ, OCTAVES, SILENCE_DB, THIRD_OCTAVE_CENTRES_HZ,
    THIRD_OCTAVES,
};

use filters::{Biquad, butterworth4_lowpass, k_weighting};
use spectrum::{
    BandPlan, CROSSOVER_HZ, Decimator, Spectrum, octave_edges, sizes, third_octave_edges,
};
use true_peak::TruePeak;

/// Frames per second.
pub const FRAMES_PER_SECOND: u32 = 20;
/// Lowest sample rate accepted, in Hz.
pub const MIN_SAMPLE_RATE: f64 = 8_000.0;
/// Highest sample rate accepted, in Hz.
pub const MAX_SAMPLE_RATE: f64 = 768_000.0;

/// Frames in the momentary-loudness window (400 ms).
const MOMENTARY_FRAMES: usize = 8;
/// Frames in the short-term-loudness window (3 s).
const SHORT_TERM_FRAMES: usize = 60;
/// Cutoff for the low-frequency correlation.
const LOW_CORRELATION_HZ: f64 = 150.0;
/// The spectral-shape range.
const SHAPE_LO_HZ: f64 = 20.0;
const SHAPE_HI_HZ: f64 = 20_000.0;
/// Energies below this count as silence (-150 dB).
const SILENT: f64 = 1e-15;

/// Why an [`Analyzer`] could not be built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfigError {
    /// The sample rate is outside the supported range.
    SampleRate(f64),
    /// The maximum block size is zero.
    MaxBlock,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleRate(rate) => write!(
                f,
                "the sample rate {rate} Hz is not supported: mort-features needs a rate \
                 from {MIN_SAMPLE_RATE} to {MAX_SAMPLE_RATE} Hz"
            ),
            Self::MaxBlock => write!(
                f,
                "the maximum block size is zero: pass the largest block the host will send, \
                 at least 1 sample"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// `n` as `f64`. Every count here is far below 2^52.
#[expect(clippy::cast_precision_loss, reason = "counts are far below 2^52")]
pub(crate) fn as_f64(n: usize) -> f64 {
    n as f64
}

/// `x` as `f32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "features are stored as f32"
)]
fn as_f32(x: f64) -> f32 {
    x as f32
}

/// Power to dB, floored at [`SILENCE_DB`].
fn power_db(p: f64) -> f32 {
    if p > SILENT {
        as_f32(10.0 * p.log10()).max(SILENCE_DB)
    } else {
        SILENCE_DB
    }
}

/// Correlation from sums, 1 for two silent channels and 0 for one.
fn correlation(ll: f64, rr: f64, lr: f64) -> f32 {
    match (ll > SILENT, rr > SILENT) {
        (true, true) => as_f32((lr / (ll * rr).sqrt()).clamp(-1.0, 1.0)),
        (false, false) => 1.0,
        _ => 0.0,
    }
}

/// Running sums over the frame being built.
#[derive(Clone, Copy, Default)]
struct Sums {
    ll: f64,
    rr: f64,
    lr: f64,
    low_ll: f64,
    low_rr: f64,
    low_lr: f64,
    k: f64,
    sample_peak: f32,
    true_peak: f32,
}

/// Stereo feature extraction, one [`Frame`] per 50 ms.
pub struct Analyzer {
    rate: f64,
    max_block: usize,
    /// Samples since construction or reset.
    position: u64,
    /// Sample at which the current frame ends.
    frame_end: u64,
    /// Sample at which the current frame started.
    frame_start: u64,
    /// Frames completed since construction or reset.
    index: u64,
    sums: Sums,
    k_weight: [[Biquad; 2]; 2],
    low_pass: [[Biquad; 2]; 2],
    true_peak: [TruePeak; 2],
    mid: Vec<f32>,
    side: Vec<f32>,
    mid_high: Spectrum,
    side_high: Spectrum,
    mid_low: Spectrum,
    side_low: Spectrum,
    mid_decimator: Decimator,
    side_decimator: Decimator,
    third_high: BandPlan,
    third_low: BandPlan,
    octave_high: BandPlan,
    octave_low: BandPlan,
    /// Which third octaves and octaves come from the low spectrum.
    third_is_low: [bool; THIRD_OCTAVES],
    octave_is_low: [bool; OCTAVES],
    /// Bins of the high spectrum used for shape and onset.
    shape_bins: (usize, usize),
    previous_power: Vec<f32>,
    /// K-weighted energy and length of recent frames, newest at `ring_pos`.
    loudness: [(f64, u32); SHORT_TERM_FRAMES],
    ring_pos: usize,
}

impl Analyzer {
    /// An analyzer for `sample_rate`, taking blocks of up to `max_block`
    /// samples per call without splitting them (larger blocks are split
    /// internally). This is the only call that allocates.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] if the rate is outside [`MIN_SAMPLE_RATE`] to
    /// [`MAX_SAMPLE_RATE`], or `max_block` is zero.
    pub fn new(sample_rate: f64, max_block: usize) -> Result<Self, ConfigError> {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
            return Err(ConfigError::SampleRate(sample_rate));
        }
        if max_block == 0 {
            return Err(ConfigError::MaxBlock);
        }
        let (factor, low_n, high_n) = sizes(sample_rate);
        let low_rate = sample_rate / as_f64(factor);
        let mut planner = RealFftPlanner::new();
        let thirds: Vec<_> = (-17..=13).map(third_octave_edges).collect();
        let octaves: Vec<_> = (-5..=4).map(octave_edges).collect();
        let is_low = |&(_, hi): &(f64, f64)| hi <= CROSSOVER_HZ * (1.0 + 1e-9);
        let mut third_is_low = [false; THIRD_OCTAVES];
        for (flag, edges) in third_is_low.iter_mut().zip(&thirds) {
            *flag = is_low(edges);
        }
        let mut octave_is_low = [false; OCTAVES];
        for (flag, edges) in octave_is_low.iter_mut().zip(&octaves) {
            *flag = is_low(edges);
        }
        let mid_high = Spectrum::new(&mut planner, high_n, sample_rate);
        let bin = mid_high.bin_hz();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bin indices are small and positive"
        )]
        let shape_bins = (
            ((SHAPE_LO_HZ / bin).ceil() as usize).max(1),
            ((SHAPE_HI_HZ.min(sample_rate / 2.0) / bin).floor() as usize).min(high_n / 2),
        );
        let k = k_weighting(sample_rate);
        let lp = butterworth4_lowpass(sample_rate, LOW_CORRELATION_HZ);
        let nominal = Self::boundary(sample_rate, 1);
        let nominal = u32::try_from(nominal).unwrap_or(u32::MAX);
        Ok(Self {
            rate: sample_rate,
            max_block,
            position: 0,
            frame_end: Self::boundary(sample_rate, 1),
            frame_start: 0,
            index: 0,
            sums: Sums::default(),
            k_weight: [k.map(Biquad::new), k.map(Biquad::new)],
            low_pass: [lp.map(Biquad::new), lp.map(Biquad::new)],
            true_peak: [TruePeak::new(max_block), TruePeak::new(max_block)],
            mid: vec![0.0; max_block],
            side: vec![0.0; max_block],
            side_high: Spectrum::new(&mut planner, high_n, sample_rate),
            mid_low: Spectrum::new(&mut planner, low_n, low_rate),
            side_low: Spectrum::new(&mut planner, low_n, low_rate),
            mid_decimator: Decimator::new(factor, sample_rate, max_block),
            side_decimator: Decimator::new(factor, sample_rate, max_block),
            third_high: BandPlan::new(&thirds, high_n, sample_rate),
            third_low: BandPlan::new(&thirds, low_n, low_rate),
            octave_high: BandPlan::new(&octaves, high_n, sample_rate),
            octave_low: BandPlan::new(&octaves, low_n, low_rate),
            third_is_low,
            octave_is_low,
            shape_bins,
            previous_power: vec![0.0; high_n / 2 + 1],
            mid_high,
            loudness: [(0.0, nominal); SHORT_TERM_FRAMES],
            ring_pos: 0,
        })
    }

    /// The sample at which frame `k` starts: `floor(k · rate / 20)`.
    fn boundary(rate: f64, k: u64) -> u64 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "frame counts stay far below 2^52 and the result is positive"
        )]
        let b = ((k as f64) * rate / f64::from(FRAMES_PER_SECOND)).floor() as u64;
        b
    }

    /// The sample rate given at construction.
    #[must_use]
    pub fn sample_rate(&self) -> f64 {
        self.rate
    }

    /// Clears all state, as if newly constructed. Does not allocate.
    pub fn reset(&mut self) {
        self.position = 0;
        self.frame_start = 0;
        self.frame_end = Self::boundary(self.rate, 1);
        self.index = 0;
        self.sums = Sums::default();
        for b in self.k_weight.iter_mut().chain(&mut self.low_pass).flatten() {
            b.reset();
        }
        for t in &mut self.true_peak {
            t.reset();
        }
        for s in [
            &mut self.mid_high,
            &mut self.side_high,
            &mut self.mid_low,
            &mut self.side_low,
        ] {
            s.reset();
        }
        self.mid_decimator.reset();
        self.side_decimator.reset();
        self.previous_power.fill(0.0);
        let nominal = u32::try_from(Self::boundary(self.rate, 1)).unwrap_or(u32::MAX);
        self.loudness = [(0.0, nominal); SHORT_TERM_FRAMES];
        self.ring_pos = 0;
    }

    /// Analyses one stereo block and calls `emit` with each frame that
    /// ends inside it, in order. If the channels differ in length, the
    /// shorter length is used. Does not allocate, lock or make system
    /// calls.
    pub fn process(&mut self, left: &[f32], right: &[f32], mut emit: impl FnMut(&Frame)) {
        let n = left.len().min(right.len());
        let mut pos = 0;
        while pos < n {
            let to_boundary = self.frame_end - self.position;
            let take = (n - pos)
                .min(self.max_block)
                .min(usize::try_from(to_boundary).unwrap_or(usize::MAX));
            self.segment(&left[pos..pos + take], &right[pos..pos + take]);
            pos += take;
            self.position += as_u64(take);
            if self.position == self.frame_end {
                let frame = self.finish_frame(pos);
                emit(&frame);
            }
        }
    }

    /// Runs the per-sample stages over part of one frame, no longer than
    /// `max_block`.
    fn segment(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len();
        let s = &mut self.sums;
        let [kl, kr] = &mut self.k_weight;
        let [ll, lr] = &mut self.low_pass;
        for (i, (&l, &r)) in left.iter().zip(right).enumerate() {
            s.sample_peak = s.sample_peak.max(l.abs()).max(r.abs());
            let (l64, r64) = (f64::from(l), f64::from(r));
            s.ll += l64 * l64;
            s.rr += r64 * r64;
            s.lr += l64 * r64;
            let kl = cascade(kl, l64);
            let kr = cascade(kr, r64);
            s.k += kl * kl + kr * kr;
            let a = cascade(ll, l64);
            let b = cascade(lr, r64);
            s.low_ll += a * a;
            s.low_rr += b * b;
            s.low_lr += a * b;
            self.mid[i] = 0.5 * (l + r);
            self.side[i] = 0.5 * (l - r);
        }
        let tp = self.true_peak[0]
            .process(left)
            .max(self.true_peak[1].process(right));
        s.true_peak = s.true_peak.max(tp);
        self.mid_high.push_slice(&self.mid[..n]);
        self.side_high.push_slice(&self.side[..n]);
        self.mid_decimator
            .process(&self.mid[..n], &mut self.mid_low);
        self.side_decimator
            .process(&self.side[..n], &mut self.side_low);
    }

    /// Builds the frame that ends at `end_offset` in the current block and
    /// starts the next.
    fn finish_frame(&mut self, end_offset: usize) -> Frame {
        let len = self.frame_end - self.frame_start;
        let len_u32 = u32::try_from(len).unwrap_or(u32::MAX);
        let s = self.sums;
        let n = as_f64(usize::try_from(len).unwrap_or(usize::MAX));

        self.loudness[self.ring_pos] = (s.k, len_u32);
        let window = |frames: usize, ring: &[(f64, u32)], pos: usize| {
            let (mut e, mut l) = (0.0, 0.0);
            for i in 0..frames {
                let (fe, fl) = ring[(pos + SHORT_TERM_FRAMES - i) % SHORT_TERM_FRAMES];
                e += fe;
                l += f64::from(fl);
            }
            power_db(e / l) - 0.691
        };
        let momentary_lufs = window(MOMENTARY_FRAMES, &self.loudness, self.ring_pos);
        let short_term_lufs = window(SHORT_TERM_FRAMES, &self.loudness, self.ring_pos);
        self.ring_pos = (self.ring_pos + 1) % SHORT_TERM_FRAMES;

        let sample_peak_dbfs = power_db(f64::from(s.sample_peak).powi(2));
        let true_peak_dbtp = power_db(f64::from(s.true_peak.max(s.sample_peak)).powi(2));
        let rms_dbfs = power_db((s.ll + s.rr) / (2.0 * n));
        let crest_db = if rms_dbfs > SILENCE_DB {
            sample_peak_dbfs - rms_dbfs
        } else {
            0.0
        };

        self.mid_high.compute();
        self.side_high.compute();
        self.mid_low.compute();
        self.side_low.compute();

        let mut mid_bands = [0.0; THIRD_OCTAVES];
        for (i, band) in mid_bands.iter_mut().enumerate() {
            *band = if self.third_is_low[i] {
                self.third_low.energy(i, &self.mid_low.power)
            } else {
                self.third_high.energy(i, &self.mid_high.power)
            };
        }
        let mut side_octaves = [0.0; OCTAVES];
        for (i, band) in side_octaves.iter_mut().enumerate() {
            *band = if self.octave_is_low[i] {
                self.octave_low.energy(i, &self.side_low.power)
            } else {
                self.octave_high.energy(i, &self.side_high.power)
            };
        }

        let (lo, hi) = self.shape_bins;
        let bin = self.mid_high.bin_hz();
        let (mut total, mut weighted, mut log_sum, mut onset) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for k in lo..=hi {
            let p = f64::from(self.mid_high.power[k]);
            total += p;
            weighted += p * as_f64(k) * bin;
            log_sum += (p + 1e-30).ln();
            onset += (p - f64::from(self.previous_power[k])).max(0.0);
        }
        self.previous_power.copy_from_slice(&self.mid_high.power);
        let count = as_f64(hi + 1 - lo);
        let (centroid_hz, flatness) = if total > SILENT {
            (
                as_f32(weighted / total),
                as_f32(((log_sum / count).exp() / (total / count)).clamp(0.0, 1.0)),
            )
        } else {
            (0.0, 0.0)
        };

        #[expect(
            clippy::cast_possible_truncation,
            reason = "the frame index wraps after 6.8 years"
        )]
        let index = self.index as u32;
        let frame = Frame {
            index,
            len: len_u32,
            end_offset: u32::try_from(end_offset).unwrap_or(u32::MAX),
            momentary_lufs: momentary_lufs.max(SILENCE_DB),
            short_term_lufs: short_term_lufs.max(SILENCE_DB),
            sample_peak_dbfs,
            true_peak_dbtp,
            rms_dbfs,
            crest_db,
            correlation: correlation(s.ll, s.rr, s.lr),
            correlation_low: correlation(s.low_ll, s.low_rr, s.low_lr),
            centroid_hz,
            flatness,
            onset: as_f32(onset),
            mid_bands,
            side_octaves,
        };

        self.index += 1;
        self.sums = Sums::default();
        self.frame_start = self.frame_end;
        self.frame_end = Self::boundary(self.rate, self.index + 1);
        frame
    }
}

/// Runs `x` through two biquads in series.
#[inline]
fn cascade(b: &mut [Biquad; 2], x: f64) -> f64 {
    let y = b[0].tick(x);
    b[1].tick(y)
}

/// `n` as `u64`.
fn as_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}
