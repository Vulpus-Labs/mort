//! Scratch harness for ticket 0010: third-octave bands from an IIR filter
//! bank, from one long FFT, and from `mort-features`' two-resolution FFT,
//! compared for accuracy and cost at 48 kHz.
//!
//! Run with `cargo run --release -p mort-features --example band_harness`.
//!
//! - IIR: 31 third-octave and 10 octave sixth-order Butterworth band-passes
//!   (third-order prototype, as in IEC 61260 class 1 designs), three `f64`
//!   biquads each, at the full rate; energy is the mean square of each
//!   output over the frame.
//! - Long FFT: a 32768-point Hann FFT (1.46 Hz bins, 683 ms) of mid and of
//!   side every frame, with bins fractionally weighted into bands.
//! - Library: [`mort_features::Analyzer`], whose cost includes every other
//!   feature too.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    missing_docs,
    reason = "scratch harness"
)]

#[path = "../tests/common/mod.rs"]
mod common;

use std::f64::consts::PI;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use common::{Rng, db, pink, sine, third_centre, third_edges, white};
use mort_features::{Analyzer, THIRD_OCTAVES};
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

const RATE: f64 = 48_000.0;
const HOP: usize = 2_400;
const BLOCK: usize = 512;

#[derive(Clone, Copy)]
struct Section {
    b: [f64; 3],
    a: [f64; 2],
    s1: f64,
    s2: f64,
}

impl Section {
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.s1;
        self.s1 = self.b[1] * x - self.a[0] * y + self.s2;
        self.s2 = self.b[2] * x - self.a[1] * y;
        y
    }
}

#[inline]
fn run3(f: &mut [Section; 3], x: f64) -> f64 {
    let a = f[0].tick(x);
    let b = f[1].tick(a);
    f[2].tick(b)
}

/// A sixth-order Butterworth band-pass from `lo` to `hi`, as three biquads,
/// unit gain at the geometric centre.
fn bandpass(lo: f64, hi: f64) -> [Section; 3] {
    let wl = 2.0 * RATE * (PI * lo / RATE).tan();
    let wh = 2.0 * RATE * (PI * hi.min(RATE * 0.499) / RATE).tan();
    let w0 = (wl * wh).sqrt();
    let bw = wh - wl;
    let mut poles = Vec::new();
    for k in 0..3 {
        let p = Complex::from_polar(1.0, PI * f64::from(2 * k + 4) / 6.0);
        let disc = (p * p * bw * bw - 4.0 * w0 * w0).sqrt();
        for s in [(p * bw + disc) / 2.0, (p * bw - disc) / 2.0] {
            if s.im > 0.0 {
                poles.push(s);
            }
        }
    }
    assert_eq!(poles.len(), 3);
    let mut out = [Section {
        b: [1.0, 0.0, -1.0],
        a: [0.0; 2],
        s1: 0.0,
        s2: 0.0,
    }; 3];
    let fc = (lo * hi.min(RATE * 0.499)).sqrt();
    let zc = Complex::from_polar(1.0, -2.0 * PI * fc / RATE);
    for (sec, s) in out.iter_mut().zip(poles) {
        let z = (2.0 * RATE + s) / (2.0 * RATE - s);
        sec.a = [-2.0 * z.re, z.norm_sqr()];
        let num = Complex::new(1.0, 0.0) - zc * zc;
        let den = Complex::new(1.0, 0.0) + zc * sec.a[0] + zc * zc * sec.a[1];
        let g = 1.0 / (num / den).norm();
        sec.b = [g, 0.0, -g];
    }
    out
}

fn octave_edges(i: usize) -> (f64, f64) {
    let c = 1_000.0 * 10f64.powf(0.3 * (i as f64 - 5.0));
    (c * 10f64.powf(-0.15), c * 10f64.powf(0.15))
}

struct IirBank {
    mid: Vec<[Section; 3]>,
    side: Vec<[Section; 3]>,
    mid_acc: Vec<f64>,
    side_acc: Vec<f64>,
}

impl IirBank {
    fn new() -> Self {
        let mid: Vec<_> = (0..THIRD_OCTAVES)
            .filter(|&i| third_edges(i).0 < RATE / 2.0)
            .map(|i| {
                let (l, h) = third_edges(i);
                bandpass(l, h)
            })
            .collect();
        let side: Vec<_> = (0..10)
            .map(|i| {
                let (l, h) = octave_edges(i);
                bandpass(l, h)
            })
            .collect();
        Self {
            mid_acc: vec![0.0; mid.len()],
            side_acc: vec![0.0; side.len()],
            mid,
            side,
        }
    }

    /// Returns per-frame mid band energies.
    fn run(&mut self, left: &[f32], right: &[f32]) -> Vec<Vec<f64>> {
        let mut frames = Vec::new();
        let mut count = 0;
        for (&l, &r) in left.iter().zip(right) {
            let m = 0.5 * f64::from(l + r);
            let s = 0.5 * f64::from(l - r);
            for (f, acc) in self.mid.iter_mut().zip(&mut self.mid_acc) {
                let y = run3(f, m);
                *acc += y * y;
            }
            for (f, acc) in self.side.iter_mut().zip(&mut self.side_acc) {
                let y = run3(f, s);
                *acc += y * y;
            }
            count += 1;
            if count == HOP {
                frames.push(self.mid_acc.iter().map(|a| a / HOP as f64).collect());
                black_box(&self.side_acc);
                self.mid_acc.fill(0.0);
                self.side_acc.fill(0.0);
                count = 0;
            }
        }
        frames
    }
}

struct LongFft {
    n: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    ring_m: Vec<f32>,
    ring_s: Vec<f32>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    norm: f32,
    thirds: Vec<(usize, Vec<f32>)>,
    octaves: Vec<(usize, Vec<f32>)>,
}

fn weights(edges: (f64, f64), n: usize) -> (usize, Vec<f32>) {
    let df = RATE / n as f64;
    let mut first = 0;
    let mut w = Vec::new();
    for k in 1..=n / 2 {
        let lo = (k as f64 - 0.5) * df;
        let hi = (k as f64 + 0.5) * df;
        let o = (hi.min(edges.1) - lo.max(edges.0)).max(0.0);
        if o > 0.0 {
            if w.is_empty() {
                first = k;
            }
            w.push((o / df) as f32);
        }
    }
    (first, w)
}

impl LongFft {
    fn new(n: usize) -> Self {
        let fft = RealFftPlanner::new().plan_fft_forward(n);
        let window: Vec<f32> = (0..n)
            .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()) as f32)
            .collect();
        let sum_sq: f64 = window.iter().map(|w| f64::from(*w).powi(2)).sum();
        Self {
            n,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            ring_m: vec![0.0; n],
            ring_s: vec![0.0; n],
            power: vec![0.0; n / 2 + 1],
            norm: (2.0 / (n as f64 * sum_sq)) as f32,
            thirds: (0..THIRD_OCTAVES)
                .map(|i| weights(third_edges(i), n))
                .collect(),
            octaves: (0..10).map(|i| weights(octave_edges(i), n)).collect(),
        }
    }

    fn spectrum(&mut self, side: bool, pos: usize) {
        let ring = if side { &self.ring_s } else { &self.ring_m };
        for i in 0..self.n {
            self.input[i] = ring[(pos + i) % self.n] * self.window[i];
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .unwrap();
        for (p, c) in self.power.iter_mut().zip(&self.output) {
            *p = c.norm_sqr() * self.norm;
        }
    }

    fn bands(&self, plan: &[(usize, Vec<f32>)]) -> Vec<f64> {
        plan.iter()
            .map(|(first, w)| {
                w.iter()
                    .zip(&self.power[*first..])
                    .map(|(w, p)| f64::from(w * p))
                    .sum()
            })
            .collect()
    }

    fn run(&mut self, left: &[f32], right: &[f32]) -> Vec<Vec<f64>> {
        let mut frames = Vec::new();
        let mut pos = 0;
        let mut count = 0;
        for (&l, &r) in left.iter().zip(right) {
            self.ring_m[pos] = 0.5 * (l + r);
            self.ring_s[pos] = 0.5 * (l - r);
            pos = (pos + 1) % self.n;
            count += 1;
            if count == HOP {
                count = 0;
                self.spectrum(false, pos);
                let mid = self.bands(&self.thirds);
                self.spectrum(true, pos);
                black_box(self.bands(&self.octaves));
                frames.push(mid);
            }
        }
        frames
    }
}

fn library(left: &[f32], right: &[f32]) -> Vec<Vec<f64>> {
    let mut a = Analyzer::new(RATE, BLOCK).unwrap();
    let mut frames = Vec::new();
    for (l, r) in left.chunks(BLOCK).zip(right.chunks(BLOCK)) {
        a.process(l, r, |f| {
            frames.push(f.mid_bands.iter().map(|&x| f64::from(x)).collect());
        });
    }
    frames
}

#[derive(Clone, Copy)]
enum Method {
    Iir,
    LongFft,
    Library,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Self::Iir => "IIR bank",
            Self::LongFft => "32k FFT",
            Self::Library => "two-resolution FFT (library)",
        }
    }

    fn run(self, x: &[f32]) -> Vec<Vec<f64>> {
        match self {
            Self::Iir => IirBank::new().run(x, x),
            Self::LongFft => LongFft::new(32_768).run(x, x),
            Self::Library => library(x, x),
        }
    }
}

/// Mean band energies over frames after `skip` seconds.
fn mean_bands(frames: &[Vec<f64>], skip_s: f64) -> Vec<f64> {
    let skip = (skip_s * 20.0) as usize;
    let used = &frames[skip..];
    let bands = used[0].len();
    (0..bands)
        .map(|b| used.iter().map(|f| f[b]).sum::<f64>() / used.len() as f64)
        .collect()
}

fn accuracy(m: Method) {
    println!("\n== {} ==", m.name());
    let a = 0.5;
    let e = a * a / 2.0;
    let (mut worst_centre, mut worst_centre_at) = (0.0f64, 0.0);
    let mut worst_leak = f64::NEG_INFINITY;
    for i in 0..THIRD_OCTAVES {
        let f = third_centre(i);
        let x = sine(RATE, f, a, 0.3, 4.0);
        let b = mean_bands(&m.run(&x), 2.5);
        if i >= b.len() {
            continue;
        }
        let err = db(b[i] / e);
        if err.abs() > worst_centre.abs() {
            worst_centre = err;
            worst_centre_at = f;
        }
        let total: f64 = b.iter().sum();
        let far: f64 = b
            .iter()
            .enumerate()
            .filter(|(j, _)| j.abs_diff(i) > 1)
            .map(|(_, v)| v)
            .sum();
        worst_leak = worst_leak.max(db(far.max(1e-30) / total));
    }
    println!(
        "sine at centre: worst error {worst_centre:+.3} dB (at {worst_centre_at:.1} Hz); \
         worst energy beyond the neighbours {worst_leak:.1} dB"
    );
    let (mut worst_edge, mut worst_edge_at) = (0.0f64, 0.0);
    let mut worst_sum = 0.0f64;
    for i in 0..THIRD_OCTAVES - 1 {
        let f = third_edges(i).1;
        let x = sine(RATE, f, a, 0.3, 4.0);
        let b = mean_bands(&m.run(&x), 2.5);
        if i + 1 >= b.len() {
            continue;
        }
        for v in [b[i], b[i + 1]] {
            let err = db(v / e) + 3.0103;
            if err.abs() > worst_edge.abs() {
                worst_edge = err;
                worst_edge_at = f;
            }
        }
        let s = db((b[i] + b[i + 1]) / e);
        if s.abs() > worst_sum.abs() {
            worst_sum = s;
        }
    }
    println!(
        "sine at edge: worst deviation from -3.01 dB {worst_edge:+.3} dB (at {worst_edge_at:.1} Hz); \
         worst error of the two bands' sum {worst_sum:+.3} dB"
    );
    let mut rng = Rng(0x5eed);
    let (x, comps) = pink(&mut rng, RATE, 1 << 20, 10.0, 22_000.0, 0.1);
    let b = mean_bands(&m.run(&x), 2.0);
    let mut errs = Vec::new();
    for (i, v) in b.iter().enumerate() {
        let (lo, hi) = third_edges(i);
        let want: f64 = comps
            .iter()
            .filter(|(f, _)| *f >= lo && *f < hi)
            .map(|(_, p)| p)
            .sum();
        errs.push(db(v / want));
    }
    let worst = errs
        .iter()
        .fold(0.0f64, |w, e| if e.abs() > w.abs() { *e } else { w });
    let rms = (errs.iter().map(|e| e * e).sum::<f64>() / errs.len() as f64).sqrt();
    let low: Vec<String> = errs[..6].iter().map(|e| format!("{e:+.2}")).collect();
    println!(
        "pink multisine: worst band error {worst:+.3} dB, rms {rms:.3} dB; 20-63 Hz: {}",
        low.join(" ")
    );
}

fn cost(m: Method, secs: f64) -> f64 {
    let mut rng = Rng(7);
    let n = (RATE * secs) as usize;
    let l = white(&mut rng, 0.1, n);
    let r = white(&mut rng, 0.1, n);
    let start = Instant::now();
    match m {
        Method::Iir => {
            black_box(IirBank::new().run(&l, &r));
        }
        Method::LongFft => {
            black_box(LongFft::new(32_768).run(&l, &r));
        }
        Method::Library => {
            let mut a = Analyzer::new(RATE, BLOCK).unwrap();
            let mut sink = 0.0f32;
            for (l, r) in l.chunks(BLOCK).zip(r.chunks(BLOCK)) {
                a.process(l, r, |f| sink += f.mid_bands[10]);
            }
            black_box(sink);
        }
    }
    start.elapsed().as_secs_f64() / secs
}

fn main() {
    for m in [Method::Iir, Method::LongFft, Method::Library] {
        accuracy(m);
    }
    println!("\n== cost per stereo instance at 48 kHz (60 s of white noise, best of 3) ==");
    for m in [Method::Iir, Method::LongFft, Method::Library] {
        let best = (0..3).map(|_| cost(m, 60.0)).fold(f64::INFINITY, f64::min);
        println!(
            "{:<30} {:7.3} % of real time, {:5.0} instances in 5 % of a core",
            m.name(),
            best * 100.0,
            0.05 / best
        );
    }
}
