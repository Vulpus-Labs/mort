//! The fake session's sound (ADR 0011 §2): test sources on tracks, and the
//! filters its FX apply.
//!
//! Sources are mono; a track's audio is the same in both channels until
//! its pan. Levels are in dBFS: a sine's `db` is its peak (0 dB is full
//! scale), a noise's is its RMS.

use serde::Serialize;

use mort_proto::model::Note;

/// A test signal on a fake track.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A sine at `hz`, peak `db`.
    Sine {
        /// Frequency.
        hz: f64,
        /// Peak level.
        db: f64,
    },
    /// White noise at RMS `db`.
    Noise {
        /// RMS level.
        db: f64,
    },
    /// Noise band-limited to `lo`–`hi` Hz by fourth-order high- and
    /// low-pass filters, at about RMS `db`.
    Band {
        /// Lower edge.
        lo: f64,
        /// Upper edge.
        hi: f64,
        /// Approximate RMS level.
        db: f64,
    },
    /// Pink noise (equal energy per octave) band-limited to `lo`–`hi` Hz
    /// by fourth-order filters, at RMS `db` exactly. Use 20 and 20 000
    /// for the full band.
    Pink {
        /// Lower edge.
        lo: f64,
        /// Upper edge.
        hi: f64,
        /// RMS level.
        db: f64,
    },
    /// Like [`Source::Pink`], but with independent noise in each channel:
    /// a wide source, its L/R correlation near 0.
    Wide {
        /// Lower edge.
        lo: f64,
        /// Upper edge.
        hi: f64,
        /// RMS level of each channel.
        db: f64,
    },
    /// A simple synth voice playing the track's MIDI notes: eight
    /// harmonics falling as `1/k`, velocity-scaled, 5 ms attack and 30 ms
    /// release. `db` is the fundamental's peak at velocity 127.
    Voice {
        /// Fundamental level at full velocity.
        db: f64,
    },
}

fn gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// A small, fast, deterministic noise generator (xorshift64*).
#[derive(Debug, Clone)]
pub struct Noise(u64);

impl Noise {
    /// A generator seeded with `seed` (any value).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// Uniform in [-1, 1).
    pub fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
        #[expect(clippy::cast_precision_loss, reason = "53-bit mantissa by design")]
        let u = v as f64 / (1u64 << 53) as f64;
        u * 2.0 - 1.0
    }
}

/// A biquad filter, RBJ cookbook forms, direct form I.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [a1 / a0, a2 / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn w(hz: f64, rate: f64, q: f64) -> (f64, f64) {
        let w = std::f64::consts::TAU * (hz / rate).clamp(1e-6, 0.499);
        (w.cos(), w.sin() / (2.0 * q))
    }

    /// A second-order low-pass.
    #[must_use]
    pub fn low_pass(hz: f64, rate: f64, q: f64) -> Self {
        let (c, al) = Self::w(hz, rate, q);
        Self::new(
            (1.0 - c) / 2.0,
            1.0 - c,
            (1.0 - c) / 2.0,
            1.0 + al,
            -2.0 * c,
            1.0 - al,
        )
    }

    /// A second-order high-pass.
    #[must_use]
    pub fn high_pass(hz: f64, rate: f64, q: f64) -> Self {
        let (c, al) = Self::w(hz, rate, q);
        Self::new(
            f64::midpoint(1.0, c),
            -(1.0 + c),
            f64::midpoint(1.0, c),
            1.0 + al,
            -2.0 * c,
            1.0 - al,
        )
    }

    /// A peaking EQ band of `db` at `hz`.
    #[must_use]
    pub fn peak(hz: f64, rate: f64, q: f64, db: f64) -> Self {
        let (c, al) = Self::w(hz, rate, q);
        let a = 10f64.powf(db / 40.0);
        Self::new(
            1.0 + al * a,
            -2.0 * c,
            1.0 - al * a,
            1.0 + al / a,
            -2.0 * c,
            1.0 - al / a,
        )
    }

    /// Filters one sample.
    pub fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// Butterworth Q values for a fourth-order filter made of two biquads.
const BUTTER4: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_6];

/// Pink noise by Paul Kellet's refined filter, band-limited, scaled to RMS
/// `db` over the whole buffer.
fn pink(len: usize, rate: f64, (lo, hi): (f64, f64), db: f64, seed: u64) -> Vec<f64> {
    let mut white = Noise::new(seed);
    let mut state = [0.0f64; 7];
    let mut fs: Vec<Biquad> = Vec::new();
    if lo > 20.0 {
        fs.extend(BUTTER4.iter().map(|q| Biquad::high_pass(lo, rate, *q)));
    }
    if hi < 20_000.0 {
        fs.extend(BUTTER4.iter().map(|q| Biquad::low_pass(hi, rate, *q)));
    }
    let mut out: Vec<f64> = (0..len)
        .map(|_| {
            let x = white.next();
            state[0] = 0.998_86 * state[0] + x * 0.055_517_9;
            state[1] = 0.993_32 * state[1] + x * 0.075_075_9;
            state[2] = 0.969_00 * state[2] + x * 0.153_852_0;
            state[3] = 0.866_50 * state[3] + x * 0.310_485_6;
            state[4] = 0.550_00 * state[4] + x * 0.532_952_2;
            state[5] = -0.761_6 * state[5] - x * 0.016_898_0;
            let p = state[0]
                + state[1]
                + state[2]
                + state[3]
                + state[4]
                + state[5]
                + state[6]
                + x * 0.5362;
            state[6] = x * 0.115_926;
            fs.iter_mut().fold(p, |v, f| f.run(v))
        })
        .collect();
    #[expect(clippy::cast_precision_loss, reason = "buffer lengths are small")]
    let rms = (out.iter().map(|v| v * v).sum::<f64>() / len.max(1) as f64).sqrt();
    if rms > 0.0 {
        let k = gain(db) / rms;
        for v in &mut out {
            *v *= k;
        }
    }
    out
}

/// Adds `source` to `left` and `right`, as [`add`] does for one channel.
/// Wide sources put independent noise in each.
#[expect(clippy::cast_precision_loss, reason = "sample indices are small")]
pub fn add_stereo(
    [left, right]: [&mut [f64]; 2],
    start: f64,
    rate: f64,
    source: &Source,
    span: (f64, f64),
    seed: u64,
    notes: &[Note],
) {
    let on = |i: usize| {
        let x = start + i as f64 / rate;
        x >= span.0 && x < span.1
    };
    if let Source::Wide { lo, hi, db } = *source {
        for (k, ch) in [left, right].into_iter().enumerate() {
            let sig = pink(
                ch.len(),
                rate,
                (lo, hi),
                db,
                seed ^ ((k as u64 + 1) * 0x5851_f42d),
            );
            for (i, (o, v)) in ch.iter_mut().zip(sig).enumerate() {
                if on(i) {
                    *o += v;
                }
            }
        }
        return;
    }
    let mut mono = vec![0.0; left.len()];
    add(&mut mono, start, rate, source, span, seed, notes);
    for ((l, r), m) in left.iter_mut().zip(right.iter_mut()).zip(mono) {
        *l += m;
        *r += m;
    }
}

/// Adds `source` to `out`, which starts at project time `start`, sampled
/// at `rate`, over `[from, to)` only. `seed` makes noise differ between
/// tracks; `notes` are the track's MIDI notes, for [`Source::Voice`].
#[expect(clippy::cast_precision_loss, reason = "sample indices are small")]
pub fn add(
    out: &mut [f64],
    start: f64,
    rate: f64,
    source: &Source,
    (from, to): (f64, f64),
    seed: u64,
    notes: &[Note],
) {
    let t = |i: usize| start + i as f64 / rate;
    let on = |i: usize| {
        let x = t(i);
        x >= from && x < to
    };
    match *source {
        Source::Sine { hz, db } => {
            let a = gain(db);
            for (i, o) in out.iter_mut().enumerate() {
                if on(i) {
                    *o += a * (std::f64::consts::TAU * hz * t(i)).sin();
                }
            }
        }
        Source::Noise { db } => {
            let a = gain(db) * 3f64.sqrt();
            let mut n = Noise::new(seed);
            for (i, o) in out.iter_mut().enumerate() {
                let v = n.next();
                if on(i) {
                    *o += a * v;
                }
            }
        }
        Source::Band { lo, hi, db } => {
            let fraction = ((hi.min(rate / 2.0) - lo) / (rate / 2.0)).clamp(1e-4, 1.0);
            let a = gain(db) * 3f64.sqrt() / fraction.sqrt();
            let mut n = Noise::new(seed);
            let mut fs: Vec<Biquad> = BUTTER4
                .iter()
                .map(|q| Biquad::high_pass(lo, rate, *q))
                .chain(BUTTER4.iter().map(|q| Biquad::low_pass(hi, rate, *q)))
                .collect();
            for (i, o) in out.iter_mut().enumerate() {
                let v = fs.iter_mut().fold(a * n.next(), |x, f| f.run(x));
                if on(i) {
                    *o += v;
                }
            }
        }
        Source::Pink { lo, hi, db } | Source::Wide { lo, hi, db } => {
            let sig = pink(out.len(), rate, (lo, hi), db, seed);
            for (i, (o, v)) in out.iter_mut().zip(sig).enumerate() {
                if on(i) {
                    *o += v;
                }
            }
        }
        Source::Voice { db } => {
            const ATTACK: f64 = 0.005;
            const RELEASE: f64 = 0.03;
            for note in notes {
                let hz = 440.0 * 2f64.powf((f64::from(note.pitch) - 69.0) / 12.0);
                let a = gain(db) * f64::from(note.velocity) / 127.0;
                let first = ((note.start.max(from) - start) * rate).ceil().max(0.0);
                let last = ((note.end + RELEASE).min(to) - start) * rate;
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped to the buffer"
                )]
                let (first, last) = (first as usize, (last.max(0.0) as usize).min(out.len()));
                for (i, o) in out.iter_mut().enumerate().take(last).skip(first) {
                    let x = t(i);
                    let env = ((x - note.start) / ATTACK).min(1.0)
                        * if x > note.end {
                            1.0 - (x - note.end) / RELEASE
                        } else {
                            1.0
                        };
                    let phase = std::f64::consts::TAU * hz * (x - note.start);
                    let mut v = 0.0;
                    for k in 1..=8 {
                        let kf = f64::from(k);
                        if hz * kf < rate * 0.45 {
                            v += (phase * kf).sin() / kf;
                        }
                    }
                    *o += a * env.max(0.0) * v;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f64]) -> f64 {
        #[expect(clippy::cast_precision_loss, reason = "test lengths")]
        let n = x.len() as f64;
        (x.iter().map(|v| v * v).sum::<f64>() / n).sqrt()
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn levels() {
        let rate = 48_000.0;
        let mut s = vec![0.0; 48_000];
        add(
            &mut s,
            0.0,
            rate,
            &Source::Sine {
                hz: 1000.0,
                db: -6.0,
            },
            (0.0, 1.0),
            1,
            &[],
        );
        assert!((db(rms(&s)) - (-6.0 - 3.01)).abs() < 0.05);

        let mut n = vec![0.0; 48_000];
        add(
            &mut n,
            0.0,
            rate,
            &Source::Noise { db: -20.0 },
            (0.0, 1.0),
            1,
            &[],
        );
        assert!((db(rms(&n)) + 20.0).abs() < 0.2, "{}", db(rms(&n)));

        let mut b = vec![0.0; 48_000];
        let band = Source::Band {
            lo: 200.0,
            hi: 400.0,
            db: -20.0,
        };
        add(&mut b, 0.0, rate, &band, (0.0, 1.0), 1, &[]);
        assert!((db(rms(&b)) + 20.0).abs() < 1.5, "{}", db(rms(&b)));
    }

    #[test]
    #[expect(clippy::many_single_char_names, reason = "signal names")]
    fn pink_and_wide() {
        let rate = 48_000.0;
        let mut p = vec![0.0; 96_000];
        let pk = Source::Pink {
            lo: 20.0,
            hi: 20_000.0,
            db: -20.0,
        };
        add(&mut p, 0.0, rate, &pk, (0.0, 2.0), 1, &[]);
        assert!((db(rms(&p)) + 20.0).abs() < 0.01);
        // Equal energy per octave: 100–200 Hz holds as much as 1–2 kHz.
        let band = |lo: f64, hi: f64| {
            let mut fs: Vec<Biquad> = BUTTER4
                .iter()
                .map(|q| Biquad::high_pass(lo, rate, *q))
                .chain(BUTTER4.iter().map(|q| Biquad::low_pass(hi, rate, *q)))
                .collect();
            let y: Vec<f64> = p
                .iter()
                .map(|x| fs.iter_mut().fold(*x, |v, f| f.run(v)))
                .collect();
            db(rms(&y[24_000..]))
        };
        let (a, b) = (band(100.0, 200.0), band(1000.0, 2000.0));
        assert!((a - b).abs() < 1.0, "{a} vs {b}");

        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        let wide = Source::Wide {
            lo: 40.0,
            hi: 150.0,
            db: -20.0,
        };
        add_stereo([&mut l, &mut r], 0.0, rate, &wide, (0.0, 1.0), 9, &[]);
        let c: f64 = l.iter().zip(&r).map(|(a, b)| a * b).sum::<f64>()
            / (l.iter().map(|a| a * a).sum::<f64>() * r.iter().map(|b| b * b).sum::<f64>()).sqrt();
        assert!(c.abs() < 0.3, "correlation {c}");
    }

    #[test]
    fn sources_keep_to_their_span() {
        let mut s = vec![0.0; 4800];
        add(
            &mut s,
            0.0,
            48_000.0,
            &Source::Noise { db: 0.0 },
            (0.05, 0.075),
            3,
            &[],
        );
        assert!(s[..2400].iter().all(|v| *v == 0.0));
        assert!(s[2400..3600].iter().any(|v| *v != 0.0));
        assert!(s[3600..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn voice_plays_notes() {
        let notes = [Note {
            pitch: 69,
            start: 0.1,
            end: 0.2,
            velocity: 127,
            channel: 0,
        }];
        let mut s = vec![0.0; 14_400];
        add(
            &mut s,
            0.0,
            48_000.0,
            &Source::Voice { db: 0.0 },
            (0.0, 1.0),
            1,
            &notes,
        );
        assert!(s[..4800].iter().all(|v| *v == 0.0));
        assert!(rms(&s[5000..9000]) > 0.5);
        assert!(s[11_100..].iter().all(|v| *v == 0.0), "released by 0.23 s");
    }

    #[test]
    fn peak_filter_boosts_its_band() {
        let rate = 48_000.0;
        let mut f = Biquad::peak(1000.0, rate, 1.0, 12.0);
        let tone: Vec<f64> = (0..48_000)
            .map(|i| (std::f64::consts::TAU * 1000.0 * f64::from(i) / rate).sin())
            .collect();
        let out: Vec<f64> = tone.iter().map(|x| f.run(*x)).collect();
        assert!((db(rms(&out[24_000..]) / rms(&tone[24_000..])) - 12.0).abs() < 0.1);
    }
}
