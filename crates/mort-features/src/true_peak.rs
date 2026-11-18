//! True peak by 4× oversampling, after BS.1770 Annex 2.
//!
//! Each input sample gets three interpolated points between it and the next
//! (at 1/4, 1/2 and 3/4), from a 12-tap-per-phase Kaiser-windowed sinc
//! interpolator: 48 taps in all, as in Annex 2. Phase 0 is the input sample
//! itself, so the true peak never reads below the sample peak. The
//! interpolator is centred, so interpolated points lag the input by six
//! samples; a peak in a frame's last six samples is counted in the next
//! frame.

use std::f64::consts::PI;

use crate::as_f64;

/// Taps per phase.
const TAPS: usize = 12;
/// Kaiser window shape.
const BETA: f64 = 7.0;

/// The zeroth-order modified Bessel function of the first kind.
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..50 {
        let k = f64::from(k);
        term *= q / (k * k);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// The Kaiser window at `t` in `[-1, 1]`.
pub(crate) fn kaiser(t: f64, beta: f64) -> f64 {
    if t.abs() >= 1.0 {
        0.0
    } else {
        bessel_i0(beta * (1.0 - t * t).sqrt()) / bessel_i0(beta)
    }
}

/// `sin(pi x) / (pi x)`.
pub(crate) fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// The interpolator's phases 1/4, 1/2 and 3/4. Tap `j` of a phase applies
/// to the input `TAPS - 1 - j` samples before the newest.
fn phases() -> [[f32; TAPS]; 3] {
    let mut out = [[0.0; TAPS]; 3];
    for (p, phase) in out.iter_mut().enumerate() {
        let d = (as_f64(p) + 1.0) / 4.0;
        let mut taps = [0.0f64; TAPS];
        for (j, tap) in taps.iter_mut().enumerate() {
            // Interpolating at time (newest - 6 + d).
            let tau = as_f64(j) - 5.0 - d;
            *tap = sinc(tau) * kaiser(tau / 6.0, BETA);
        }
        let sum: f64 = taps.iter().sum();
        for (o, t) in phase.iter_mut().zip(taps) {
            #[expect(clippy::cast_possible_truncation, reason = "taps are stored as f32")]
            {
                *o = (t / sum) as f32;
            }
        }
    }
    out
}

/// True-peak state for one channel.
pub struct TruePeak {
    phases: [[f32; TAPS]; 3],
    /// The last `TAPS - 1` inputs, then room for one block.
    hist: Vec<f32>,
    /// One phase's outputs for the current block.
    out: Vec<f32>,
}

impl TruePeak {
    /// State for blocks of up to `max_block` samples.
    pub fn new(max_block: usize) -> Self {
        Self {
            phases: phases(),
            hist: vec![0.0; TAPS - 1 + max_block],
            out: vec![0.0; max_block],
        }
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.hist.fill(0.0);
    }

    /// The largest absolute interpolated value over `x`, which must be no
    /// longer than the `max_block` given at construction.
    pub fn process(&mut self, x: &[f32]) -> f32 {
        let n = x.len();
        self.hist[TAPS - 1..TAPS - 1 + n].copy_from_slice(x);
        // Accumulate across samples rather than across taps, so the inner
        // loop has no reduction and vectorises.
        let out = &mut self.out[..n];
        let mut peak = 0.0f32;
        for phase in &self.phases {
            out.fill(0.0);
            for (j, &h) in phase.iter().enumerate() {
                for (o, &v) in out.iter_mut().zip(&self.hist[j..j + n]) {
                    *o += h * v;
                }
            }
            peak = out.iter().fold(peak, |p, v| p.max(v.abs()));
        }
        self.hist.copy_within(n..n + TAPS - 1, 0);
        peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_have_unit_dc_gain() {
        for p in phases() {
            let s: f32 = p.iter().sum();
            assert!((s - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn sine_at_quarter_rate_with_45_degree_offset_reads_full_scale() {
        // Samples sit at ±0.7071 (-3.01 dBFS); the waveform peaks at 1.0.
        let x: Vec<f32> = (0..4_800)
            .map(|n| {
                let t = f64::from(n) * PI / 2.0 + PI / 4.0;
                #[expect(clippy::cast_possible_truncation, reason = "test signal")]
                let v = t.sin() as f32;
                v
            })
            .collect();
        let mut tp = TruePeak::new(4_800);
        let peak = tp.process(&x);
        let db = 20.0 * f64::from(peak).log10();
        assert!(db.abs() < 0.2, "{db} dBTP");
    }
}
