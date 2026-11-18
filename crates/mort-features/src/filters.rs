//! IIR filters: biquads, BS.1770 K-weighting designed for any sample rate,
//! and the Butterworth low-pass used for low-frequency correlation.

use std::f64::consts::PI;

/// Coefficients of one biquad, normalised so that `a0` is 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoeffs {
    /// Feed-forward coefficients `b0`, `b1`, `b2`.
    pub b: [f64; 3],
    /// Feedback coefficients `a1`, `a2`.
    pub a: [f64; 2],
}

/// A biquad in transposed direct form II, with `f64` state.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    c: BiquadCoeffs,
    s1: f64,
    s2: f64,
}

impl Biquad {
    /// A biquad with the given coefficients and zero state.
    pub fn new(c: BiquadCoeffs) -> Self {
        Self {
            c,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// Filters one sample.
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = self.c.b[0] * x + self.s1;
        self.s1 = self.c.b[1] * x - self.c.a[0] * y + self.s2;
        self.s2 = self.c.b[2] * x - self.c.a[1] * y;
        y
    }

    /// Clears the state.
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// The two stages of BS.1770's K-weighting (the head-related high shelf,
/// then the RLB high-pass) for `sample_rate`.
///
/// BS.1770 gives coefficients for 48 kHz only. These come from the analog
/// prototype those coefficients were derived from (centre frequencies, gain
/// and Q as recovered by the libebur128 and pyloudnorm authors), through the
/// bilinear transform with the centre frequency prewarped. At 48 kHz they
/// match the published coefficients to better than 1e-8.
pub fn k_weighting(sample_rate: f64) -> [BiquadCoeffs; 2] {
    // Stage 1: high shelf.
    let f0 = 1_681.974_450_955_533;
    let gain_db = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;
    let k = (PI * f0 / sample_rate).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = BiquadCoeffs {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    // Stage 2: high-pass. BS.1770 leaves the numerator unnormalised as
    // 1, -2, 1; we do the same so the 48 kHz coefficients match.
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (PI * f0 / sample_rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let highpass = BiquadCoeffs {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    [shelf, highpass]
}

/// A second-order low-pass section with cutoff `f0` and quality `q`,
/// by the bilinear transform with the cutoff prewarped.
pub fn lowpass(sample_rate: f64, f0: f64, q: f64) -> BiquadCoeffs {
    let k = (PI * f0 / sample_rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let b0 = k * k / a0;
    BiquadCoeffs {
        b: [b0, 2.0 * b0, b0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    }
}

/// The two sections of a fourth-order Butterworth low-pass at `f0`.
pub fn butterworth4_lowpass(sample_rate: f64, f0: f64) -> [BiquadCoeffs; 2] {
    // Section Qs of a fourth-order Butterworth: 1 / (2 cos(pi/8)) and
    // 1 / (2 cos(3 pi/8)).
    let q1 = 1.0 / (2.0 * (PI / 8.0).cos());
    let q2 = 1.0 / (2.0 * (3.0 * PI / 8.0).cos());
    [lowpass(sample_rate, f0, q1), lowpass(sample_rate, f0, q2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_weighting_matches_bs1770_at_48k() {
        let [s1, s2] = k_weighting(48_000.0);
        let want1 = BiquadCoeffs {
            b: [
                1.535_124_859_586_97,
                -2.691_696_189_406_38,
                1.198_392_810_852_85,
            ],
            a: [-1.690_659_293_182_41, 0.732_480_774_215_85],
        };
        let want2 = BiquadCoeffs {
            b: [1.0, -2.0, 1.0],
            a: [-1.990_047_454_833_98, 0.990_072_250_366_21],
        };
        for (got, want) in [(s1, want1), (s2, want2)] {
            for i in 0..3 {
                assert!((got.b[i] - want.b[i]).abs() < 1e-8, "{got:?} vs {want:?}");
            }
            for i in 0..2 {
                assert!((got.a[i] - want.a[i]).abs() < 1e-8, "{got:?} vs {want:?}");
            }
        }
    }

    fn gain_at(c: &[BiquadCoeffs], fs: f64, f: f64) -> f64 {
        let w = 2.0 * PI * f / fs;
        let mut g = 1.0;
        for s in c {
            let (cr, ci) = (w.cos(), -w.sin());
            let (c2r, c2i) = ((2.0 * w).cos(), -(2.0 * w).sin());
            let nr = s.b[0] + s.b[1] * cr + s.b[2] * c2r;
            let ni = s.b[1] * ci + s.b[2] * c2i;
            let dr = 1.0 + s.a[0] * cr + s.a[1] * c2r;
            let di = s.a[0] * ci + s.a[1] * c2i;
            g *= ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt();
        }
        g
    }

    #[test]
    fn k_weighting_response_is_the_same_at_other_rates() {
        let reference = k_weighting(48_000.0);
        for fs in [44_100.0, 88_200.0, 96_000.0, 192_000.0] {
            let c = k_weighting(fs);
            for f in [20.0, 100.0, 1_000.0, 4_000.0, 10_000.0] {
                let a = 20.0 * gain_at(&reference, 48_000.0, f).log10();
                let b = 20.0 * gain_at(&c, fs, f).log10();
                assert!((a - b).abs() < 0.05, "{fs} Hz rate, {f} Hz: {a} vs {b}");
            }
        }
    }

    #[test]
    fn butterworth_is_3db_down_at_cutoff() {
        let c = butterworth4_lowpass(48_000.0, 150.0);
        let g = 20.0 * gain_at(&c, 48_000.0, 150.0).log10();
        assert!((g + 3.01).abs() < 0.05, "{g}");
        assert!(gain_at(&c, 48_000.0, 10.0) > 0.999);
    }
}
