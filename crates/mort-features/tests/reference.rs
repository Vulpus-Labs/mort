//! Features against signals with known answers (ADR 0011 §3).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test signal arithmetic"
)]
#![allow(
    clippy::float_cmp,
    reason = "silence and above-Nyquist bands are specified as exact values"
)]
#![allow(clippy::many_single_char_names, reason = "l, r for channels")]

mod common;

use common::{Rng, amp, db, pink, run, sine, third_centre, third_edges, white};
use mort_features::{Frame, SILENCE_DB, THIRD_OCTAVES};

const RATE: f64 = 48_000.0;

/// Frames whose end is at or after `secs`.
fn after(frames: &[Frame], rate: f64, secs: f64) -> impl Iterator<Item = &Frame> {
    let mut end = 0u64;
    frames.iter().filter(move |f| {
        end += u64::from(f.len);
        end as f64 >= secs * rate
    })
}

/// `x` with each sample's gain chosen by `gain(seconds)`.
fn gated(x: &[f32], rate: f64, gain: impl Fn(f64) -> f64) -> Vec<f32> {
    x.iter()
        .enumerate()
        .map(|(i, &v)| (f64::from(v) * gain(i as f64 / rate)) as f32)
        .collect()
}

fn assert_near(got: f32, want: f64, tol: f64, what: &str) {
    assert!(
        (f64::from(got) - want).abs() <= tol,
        "{what}: got {got}, want {want} ± {tol}"
    );
}

// --- Loudness: EBU Tech 3341 ----------------------------------------------

/// Cases 1 and 2: a stereo 1 kHz sine at -23 and -33 dBFS reads -23 and
/// -33 LUFS, momentary and short-term, at 48 and 44.1 kHz.
#[test]
fn ebu_3341_cases_1_and_2_steady_tones() {
    for rate in [48_000.0, 44_100.0] {
        for level in [-23.0, -33.0] {
            let x = sine(rate, 1_000.0, amp(level), 0.0, 6.0);
            let frames = run(rate, &x, &x, 512);
            for f in after(&frames, rate, 3.0) {
                assert_near(f.momentary_lufs, level, 0.1, "momentary");
                assert_near(f.short_term_lufs, level, 0.1, "short-term");
            }
        }
    }
}

/// Case 9: 1 kHz alternating 1.34 s at -20 dBFS and 1.66 s at -30 dBFS,
/// five periods. Short-term loudness reads -23 LUFS once the first 3 s
/// have passed.
#[test]
fn ebu_3341_case_9_short_term_alternating_tone() {
    let x = sine(RATE, 1_000.0, 1.0, 0.0, 15.0);
    let x = gated(&x, RATE, |t| {
        if t % 3.0 < 1.34 {
            amp(-20.0)
        } else {
            amp(-30.0)
        }
    });
    let frames = run(RATE, &x, &x, 480);
    for f in after(&frames, RATE, 3.0) {
        assert_near(f.short_term_lufs, -23.0, 0.1, "short-term");
    }
}

/// Case 12: 1 kHz alternating 0.18 s at -20 dBFS and 0.22 s at -30 dBFS,
/// 25 periods. Momentary loudness reads -23 LUFS once the first 400 ms
/// have passed.
#[test]
fn ebu_3341_case_12_momentary_alternating_tone() {
    let x = sine(RATE, 1_000.0, 1.0, 0.0, 10.0);
    let x = gated(&x, RATE, |t| {
        if t % 0.4 < 0.18 {
            amp(-20.0)
        } else {
            amp(-30.0)
        }
    });
    let frames = run(RATE, &x, &x, 333);
    for f in after(&frames, RATE, 0.45) {
        assert_near(f.momentary_lufs, -23.0, 0.1, "momentary");
    }
}

#[test]
fn silence_reads_the_floor_and_nothing_is_nan() {
    let x = vec![0.0f32; 48_000];
    for f in run(RATE, &x, &x, 512) {
        assert_eq!(f.momentary_lufs, SILENCE_DB);
        assert_eq!(f.short_term_lufs, SILENCE_DB);
        assert_eq!(f.sample_peak_dbfs, SILENCE_DB);
        assert_eq!(f.true_peak_dbtp, SILENCE_DB);
        assert_eq!(f.rms_dbfs, SILENCE_DB);
        assert_eq!(f.crest_db, 0.0);
        assert_eq!(f.correlation, 1.0);
        assert_eq!(f.centroid_hz, 0.0);
        assert_eq!(f.flatness, 0.0);
        assert_eq!(f.onset, 0.0);
        assert!(f.mid_bands.iter().chain(&f.side_octaves).all(|b| *b == 0.0));
    }
}

// --- Peaks, RMS and crest ---------------------------------------------------

/// A sine at a quarter of the rate with a 45° phase offset has every
/// sample at ±0.707 of its amplitude, and peaks halfway between samples,
/// where 4× oversampling lands exactly.
#[test]
fn true_peak_of_quarter_rate_sine_at_45_degrees() {
    for rate in [48_000.0, 44_100.0] {
        for level in [0.0, -6.0] {
            let x = sine(
                rate,
                rate / 4.0,
                amp(level),
                std::f64::consts::FRAC_PI_4,
                1.0,
            );
            let frames = run(rate, &x, &x, 256);
            for f in after(&frames, rate, 0.1) {
                assert_near(f.true_peak_dbtp, level, 0.2, "true peak");
                assert_near(f.sample_peak_dbfs, level - 3.0103, 0.01, "sample peak");
            }
        }
    }
}

/// Other tones well below Nyquist read their amplitude within 0.2 dB.
#[test]
fn true_peak_of_tones_reads_their_amplitude() {
    for freq in [997.0, 5_100.0, 9_970.0] {
        let x = sine(RATE, freq, amp(-1.0), 0.4, 1.0);
        let frames = run(RATE, &x, &x, 512);
        for f in after(&frames, RATE, 0.1) {
            assert_near(f.true_peak_dbtp, -1.0, 0.2, "true peak");
            assert!(f.true_peak_dbtp >= f.sample_peak_dbfs);
        }
    }
}

#[test]
fn rms_and_crest_of_a_sine() {
    let x = sine(RATE, 1_000.0, 0.5, 0.0, 1.0);
    for f in run(RATE, &x, &x, 512) {
        assert_near(f.rms_dbfs, db(0.125), 0.01, "rms");
        assert_near(f.crest_db, 3.0103, 0.01, "crest");
    }
}

// --- Bands ------------------------------------------------------------------

/// Mean of the band energies over frames after `secs`.
fn mean_bands(frames: &[Frame], secs: f64) -> Vec<f64> {
    let used: Vec<_> = after(frames, RATE, secs).collect();
    (0..THIRD_OCTAVES)
        .map(|b| used.iter().map(|f| f64::from(f.mid_bands[b])).sum::<f64>() / used.len() as f64)
        .collect()
}

/// A sine at a band's centre puts its energy, a²/2, in that band. The
/// harness measured a worst error of 0.063 dB (the 20 Hz band, where the
/// Hann main lobe reaches the band edges); the tolerance is 0.1 dB.
#[test]
fn sine_at_each_band_centre_lands_in_its_band() {
    for i in 0..THIRD_OCTAVES {
        let x = sine(RATE, third_centre(i), 0.5, 0.3, 2.5);
        let b = mean_bands(&run(RATE, &x, &x, 512), 1.0);
        let err = db(b[i] / 0.125);
        assert!(err.abs() < 0.1, "band {i}: {err} dB");
        let far: f64 = b
            .iter()
            .enumerate()
            .filter(|(j, _)| j.abs_diff(i) > 1)
            .map(|(_, v)| v)
            .sum();
        assert!(
            db(far / 0.125) < -40.0,
            "band {i} leaks {} dB",
            db(far / 0.125)
        );
    }
}

/// A sine on the edge between two bands splits its energy equally, as a
/// filter bank's -3 dB crossings would. The harness measured a worst
/// deviation of 0.27 dB from -3.01 dB (frequency resolution at the lowest
/// edges); the tolerance is 0.35 dB, and the pair must sum to the whole
/// within 0.1 dB.
#[test]
fn sine_at_each_band_edge_splits_between_neighbours() {
    for i in 0..THIRD_OCTAVES - 1 {
        let x = sine(RATE, third_edges(i).1, 0.5, 0.3, 2.5);
        let b = mean_bands(&run(RATE, &x, &x, 512), 1.0);
        for v in [b[i], b[i + 1]] {
            let err = db(v / 0.125) + 3.0103;
            assert!(err.abs() < 0.35, "edge above band {i}: {err} dB");
        }
        let sum = db((b[i] + b[i + 1]) / 0.125);
        assert!(sum.abs() < 0.1, "edge above band {i}: sum {sum} dB");
    }
}

/// Pink noise has equal energy per third octave. The test signal is a
/// periodic multisine with 1/f power and random phases, so each band's
/// energy is known exactly from its components; what remains is the
/// analysis's own error. The harness measured 0.12 dB worst and 0.045 dB
/// rms over 21.8 s; the tolerance is 0.2 dB per band, and the spread of the
/// bands is checked against the 1/f answer (equal energy) within 0.5 dB,
/// which allows for the discrete component grid at 20 Hz.
#[test]
fn pink_multisine_bands_match_their_components() {
    let mut rng = Rng(0x5eed);
    let (x, comps) = pink(&mut rng, RATE, 1 << 20, 10.0, 22_000.0, 0.1);
    let b = mean_bands(&run(RATE, &x, &x, 1024), 2.0);
    let mut energies = Vec::new();
    for (i, v) in b.iter().enumerate() {
        let (lo, hi) = third_edges(i);
        let want: f64 = comps
            .iter()
            .filter(|(f, _)| *f >= lo && *f < hi)
            .map(|(_, p)| p)
            .sum();
        let err = db(v / want);
        assert!(err.abs() < 0.2, "band {i}: {err} dB");
        energies.push(db(*v));
    }
    let mean = energies.iter().sum::<f64>() / energies.len() as f64;
    for (i, e) in energies.iter().enumerate() {
        assert!(
            (e - mean).abs() < 0.5,
            "band {i} is {} dB off flat",
            e - mean
        );
    }
}

/// At 32 kHz the 20 kHz band is above Nyquist and reads exactly zero.
#[test]
fn bands_above_nyquist_read_zero() {
    let mut rng = Rng(3);
    let x = white(&mut rng, 0.1, 32_000);
    for f in run(32_000.0, &x, &x, 512) {
        assert_eq!(f.mid_bands[30], 0.0);
        assert!(
            f.mid_bands
                .iter()
                .chain(&f.side_octaves)
                .all(|v| v.is_finite())
        );
    }
}

// --- Stereo -----------------------------------------------------------------

#[test]
fn correlation_of_known_pairs() {
    let s = sine(RATE, 1_000.0, 0.5, 0.0, 1.0);
    let neg: Vec<f32> = s.iter().map(|v| -v).collect();
    // 1 kHz in quadrature: 50 whole cycles per frame, so exactly 0.
    let q = sine(RATE, 1_000.0, 0.5, std::f64::consts::FRAC_PI_2, 1.0);
    let silent = vec![0.0f32; s.len()];
    let mut rng = Rng(11);
    let a = white(&mut rng, 0.1, 48_000);
    let b = white(&mut rng, 0.1, 48_000);
    let cases: [(&[f32], &[f32], f64, f64); 5] = [
        (&s, &s, 1.0, 1e-6),
        (&s, &neg, -1.0, 1e-6),
        (&s, &q, 0.0, 1e-3),
        (&s, &silent, 0.0, 0.0),
        // Independent noise over 2400 samples: standard error 1/sqrt(2400).
        (&a, &b, 0.0, 0.1),
    ];
    for (l, r, want, tol) in cases {
        for f in run(RATE, l, r, 512) {
            assert_near(f.correlation, want, tol, "correlation");
        }
    }
}

/// A shared 50 Hz tone under independent noise above 2 kHz: the full-band
/// correlation is diluted by the noise, the band below 150 Hz is not. A
/// 60 Hz tone in quadrature (three whole cycles per frame) reads 0 below
/// 150 Hz.
#[test]
fn low_correlation_sees_only_the_low_end() {
    let mut rng = Rng(5);
    let low = sine(RATE, 50.0, 0.3, 0.0, 3.0);
    let mut hp = |x: Vec<f32>| -> Vec<f32> {
        // First difference twice: a crude high-pass, ~-60 dB at 50 Hz.
        let d: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let _ = &mut rng;
        d.windows(2).map(|w| w[1] - w[0]).collect()
    };
    let na = hp(white(&mut Rng(21), 0.5, low.len() + 2));
    let nb = hp(white(&mut Rng(22), 0.5, low.len() + 2));
    let l: Vec<f32> = low.iter().zip(&na).map(|(a, b)| a + b).collect();
    let r: Vec<f32> = low.iter().zip(&nb).map(|(a, b)| a + b).collect();
    for f in after(&run(RATE, &l, &r, 512), RATE, 1.0) {
        assert!(f.correlation < 0.5, "full band {}", f.correlation);
        assert_near(f.correlation_low, 1.0, 0.01, "below 150 Hz");
    }
    let a = sine(RATE, 60.0, 0.5, 0.0, 3.0);
    let b = sine(RATE, 60.0, 0.5, std::f64::consts::FRAC_PI_2, 3.0);
    for f in after(&run(RATE, &a, &b, 512), RATE, 1.0) {
        assert_near(f.correlation_low, 0.0, 0.02, "quadrature below 150 Hz");
    }
}

/// Side is (L - R) / 2: zero for mono, and all of an inverted pair.
#[test]
fn side_octaves_of_known_pairs() {
    let s = sine(RATE, 1_000.0, 0.5, 0.0, 2.0);
    for f in run(RATE, &s, &s, 512) {
        assert!(f.side_octaves.iter().all(|v| *v == 0.0));
    }
    // 1 kHz is octave 5 (high spectrum); 63 Hz is octave 1 (low spectrum).
    for (freq, octave) in [(1_000.0, 5), (63.0957, 1)] {
        let l = sine(RATE, freq, 0.5, 0.0, 2.5);
        let r: Vec<f32> = l.iter().map(|v| -v).collect();
        let frames = run(RATE, &l, &r, 512);
        for f in after(&frames, RATE, 1.0) {
            let err = db(f64::from(f.side_octaves[octave]) / 0.125);
            assert!(err.abs() < 0.1, "{freq} Hz: {err} dB");
            assert!(f.mid_bands.iter().all(|v| *v < 1e-12));
        }
    }
}

// --- Spectral shape and onsets ---------------------------------------------

#[test]
fn centroid_of_tones_and_noise() {
    let one = sine(RATE, 1_000.0, 0.5, 0.0, 1.0);
    for f in after(&run(RATE, &one, &one, 512), RATE, 0.1) {
        assert_near(f.centroid_hz, 1_000.0, 1.0, "1 kHz sine");
    }
    // Equal power at 500 Hz and 2 kHz: the centroid is their mean.
    let a = sine(RATE, 500.0, 0.3, 0.0, 1.0);
    let b = sine(RATE, 2_000.0, 0.3, 0.0, 1.0);
    let two: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x + y).collect();
    for f in after(&run(RATE, &two, &two, 512), RATE, 0.1) {
        assert_near(f.centroid_hz, 1_250.0, 2.0, "two sines");
    }
    // White noise: flat from 20 Hz to 20 kHz, so the centroid is 10.01 kHz.
    let n = white(&mut Rng(9), 0.1, 96_000);
    let frames = run(RATE, &n, &n, 512);
    let used: Vec<_> = after(&frames, RATE, 0.1).collect();
    let mean = used.iter().map(|f| f64::from(f.centroid_hz)).sum::<f64>() / used.len() as f64;
    assert!(
        (mean - 10_010.0).abs() < 100.0,
        "white noise centroid {mean}"
    );
}

/// A pure tone is nearly all in a few bins (flatness near 0). White noise
/// has an exponentially distributed periodogram, whose geometric mean is
/// e^-γ ≈ 0.561 of its arithmetic mean.
#[test]
fn flatness_of_a_tone_and_of_white_noise() {
    let s = sine(RATE, 1_000.0, 0.5, 0.0, 1.0);
    for f in after(&run(RATE, &s, &s, 512), RATE, 0.1) {
        assert!(f.flatness < 0.01, "sine flatness {}", f.flatness);
    }
    let n = white(&mut Rng(13), 0.1, 240_000);
    let frames = run(RATE, &n, &n, 512);
    let used: Vec<_> = after(&frames, RATE, 0.1).collect();
    let mean = used.iter().map(|f| f64::from(f.flatness)).sum::<f64>() / used.len() as f64;
    let want = (-0.577_215_664_9f64).exp();
    assert!(
        (mean - want).abs() < 0.02,
        "white noise flatness {mean}, want {want}"
    );
}

/// A steady tone has no onset strength once it has filled the window. A
/// tone starting from silence gives at least its energy, a²/2, in the
/// frames its start passes through, and the switch-on transient's broad
/// spectrum adds a little more.
#[test]
fn onset_of_a_steady_tone_and_a_tone_starting() {
    let s = sine(RATE, 1_000.0, 0.5, 0.0, 2.0);
    for f in after(&run(RATE, &s, &s, 512), RATE, 0.2) {
        assert!(f.onset < 1e-5, "steady onset {}", f.onset);
    }
    // Silence for 1.025 s, then the tone (mid-frame start).
    let x = gated(&sine(RATE, 1_000.0, 0.5, 0.0, 2.0), RATE, |t| {
        if t < 1.025 { 0.0 } else { 1.0 }
    });
    let frames = run(RATE, &x, &x, 512);
    let total: f64 = frames.iter().map(|f| f64::from(f.onset)).sum();
    assert!((0.125..0.125 * 1.5).contains(&total), "onset total {total}");
    let peak = frames
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.onset.total_cmp(&b.1.onset))
        .unwrap()
        .0;
    // The frame holding the start (frame 20) or the next.
    assert!((20..=21).contains(&peak), "onset peak in frame {peak}");
}

// --- Framing ----------------------------------------------------------------

/// Frame k covers floor(k·rate/20) to floor((k+1)·rate/20), whatever the
/// block size, and `end_offset` places it in its block.
#[test]
fn frame_boundaries_are_exact_in_samples() {
    for (rate, block) in [
        (48_000.0, 512),
        (44_100.0, 333),
        (22_050.0, 4_096),
        (8_000.0, 1),
    ] {
        let x = vec![0.1f32; (rate * 2.0) as usize];
        let mut a = mort_features::Analyzer::new(rate, block).unwrap();
        let mut ends = Vec::new();
        let mut block_start = 0u64;
        for chunk in x.chunks(block) {
            a.process(chunk, chunk, |f| {
                ends.push((f.index, f.len, block_start + u64::from(f.end_offset)));
            });
            block_start += chunk.len() as u64;
        }
        assert_eq!(ends.len(), 40, "{rate} Hz");
        let mut start = 0u64;
        for (k, (index, len, end)) in ends.into_iter().enumerate() {
            let want = ((k as f64 + 1.0) * rate / 20.0).floor() as u64;
            assert_eq!(index as usize, k);
            assert_eq!(end, want, "{rate} Hz frame {k}");
            assert_eq!(u64::from(len), want - start);
            start = want;
        }
    }
}

/// Splitting the audio into different blocks gives identical frames.
#[test]
fn block_size_does_not_change_the_frames() {
    let mut rng = Rng(17);
    let l = white(&mut rng, 0.2, 96_000);
    let r = white(&mut rng, 0.2, 96_000);
    let strip = |f: &Frame| Frame {
        end_offset: 0,
        ..*f
    };
    let a: Vec<_> = run(RATE, &l, &r, 64).iter().map(strip).collect();
    let b: Vec<_> = run(RATE, &l, &r, 4_096).iter().map(strip).collect();
    let c: Vec<_> = run(RATE, &l, &r, 2_401).iter().map(strip).collect();
    assert_eq!(a, b);
    assert_eq!(a, c);
}
