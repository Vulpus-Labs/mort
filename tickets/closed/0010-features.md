---
id: "0010"
title: "mort-features: features, reference tests, filter-bank choice, cost"
priority: high
created: 2026-11-18
epic: E001
depends: ["0001"]
---

## Summary

Implement the features of ADR 0007 §1 as a block-processing library that
allocates only at construction and emits 50 ms frames.

## Acceptance criteria

- [x] Loudness within 0.1 LU of EBU Tech 3341 test cases (momentary and
      short-term). True peak within 0.2 dB of BS.1770 references.
- [x] Third-octave bands: write a scratch harness and measure an IIR bank
      against a long FFT for accuracy (sines at band edges, pink noise)
      and cost per stereo instance. Record the numbers, choose, and
      update ADR 0007 §1.
- [x] Correlation (full and below 150 Hz), side octaves, centroid,
      flatness and onset strength, each tested on signals with known
      answers.
- [x] Frame layout fixed and documented, with its size in bytes.
- [x] Cost per stereo instance at 48 kHz measured and recorded. Target:
      40 instances under 5 % of one M3 core.

## Close-out (2026-11-18)

- `mort-features` has `Analyzer`: built with a sample rate (8 to
  768 kHz) and a maximum block size, it takes stereo blocks and calls back
  with one `Frame` per 50 ms. Frame `k` covers samples `floor(k·rate/20)`
  to `floor((k+1)·rate/20)`, so boundaries are exact; frames carry their
  index, length and `end_offset` in the block, and the probe stamps the
  transport position. Splitting audio into different block sizes gives
  bit-identical frames.
- `Frame` is `#[repr(C)]`, three `u32` and 52 `f32`, 220 bytes (checked at
  compile time). Levels are in dB floored at -150, energies are linear mean
  squares, nothing is NaN. ADR 0006's frame size and capture size are
  updated (220 bytes, about 53 MB for five minutes of 40 tracks).
- K-weighting is designed for any rate from the analog prototype and
  matches BS.1770's 48 kHz coefficients to 1e-8. True peak uses a 48-tap
  (4 × 12) Kaiser-windowed sinc interpolator of our own design rather than
  Annex 2's published coefficients.
- Bands: `examples/band_harness.rs` compares, at 48 kHz on an M3:

  | | sine at centre | sine at edge (vs -3.01 dB) | pink, worst / rms | cost per instance |
  |---|---|---|---|---|
  | IIR bank, sixth-order Butterworth, f64 | 0.011 dB | 0.016 dB | 0.33 / 0.21 dB | 0.47 % (bands only) |
  | 32768-point FFT | 0.063 dB | 0.27 dB | 0.12 / 0.043 dB | 0.31 % (bands only) |
  | two-resolution FFT (chosen) | 0.063 dB | 0.27 dB | 0.12 / 0.045 dB | 0.107 % (whole analyzer) |

  The two-resolution FFT was chosen; ADR 0007 §1 says why.
- Cost: the whole analyzer takes 0.107 % of real time per stereo instance
  at 48 kHz in release (1.07 ms of CPU per second of audio), so 40
  instances use 4.3 % of one M3 core, inside the 5 % target. Getting there
  needed the true-peak interpolator and the decimation filter rewritten so
  their inner loops vectorise (0.175 % before).
- Tests (28 plus a doctest): EBU Tech 3341 cases 1 and 2 (steady -23 and
  -33 dBFS tones, at 48 and 44.1 kHz), case 9 (short-term, 1.34 s at -20 /
  1.66 s at -30 dBFS) and case 12 (momentary, 0.18 s at -20 / 0.22 s at
  -30 dBFS), all within 0.1 LU; true peak of a quarter-rate sine at 45°
  within 0.2 dB at 0 and -6 dBTP; bands for sines at every centre and edge
  and for a pink multisine; correlation, low correlation, side octaves,
  centroid, flatness (white noise reads e^-γ) and onset on known signals;
  exact framing at four rates; and a counting global allocator showing
  `process` and `reset` make no allocations.
- Deviations:
  - EBU Tech 3341 cases 3 to 8 test integrated loudness, gating or
    multichannel, or use programme files, and are out of scope. Cases 10,
    11 and 13 were not reproduced, because their exact definitions were
    not to hand. The true-peak cases 15 to 23 of Tech 3341 were replaced
    by analytic intersample-peak signals.
  - Bands below 708 Hz come from a 683 ms window, so they lag the frame;
    this is recorded in ADR 0007 and in `Frame`'s docs.
  - Dependency added: `realfft` (pure Rust, with `rustfft`).
