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

- [ ] Loudness within 0.1 LU of EBU Tech 3341 test cases (momentary and
      short-term). True peak within 0.2 dB of BS.1770 references.
- [ ] Third-octave bands: write a scratch harness and measure an IIR bank
      against a long FFT for accuracy (sines at band edges, pink noise)
      and cost per stereo instance. Record the numbers, choose, and
      update ADR 0007 §1.
- [ ] Correlation (full and below 150 Hz), side octaves, centroid,
      flatness and onset strength, each tested on signals with known
      answers.
- [ ] Frame layout fixed and documented, with its size in bytes.
- [ ] Cost per stereo instance at 48 kHz measured and recorded. Target:
      40 instances under 5 % of one M3 core.
