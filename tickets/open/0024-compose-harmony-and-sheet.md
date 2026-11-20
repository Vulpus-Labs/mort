---
id: "0024"
title: "mort-compose: pitches, scales, chord symbols, harmony; the sheet's form and harmony"
priority: high
created: 2026-11-20
epic: E004
depends: []
---

## Summary

Start `mort-compose` (ADR 0013 §1, §3, §4): the harmony layer, and the
parts of `sheet.kdl` that describe form and harmony.

## Acceptance criteria

- [ ] Crate `mort-compose`, with no dependency on REAPER or the bridge.
- [ ] Pitch classes and pitches with spelling (`C4` is 60, as in `mort
      midi`), intervals.
- [ ] Scales as data: the modes of major, melodic minor and harmonic
      minor, whole tone, both diminished scales, the pentatonics and the
      blues scale, with names and aliases (`dorian`, `lydian dominant`,
      `altered`, …).
- [ ] A chord-symbol grammar: root, quality, sevenths, extensions,
      alterations, `sus`, `add`, `omit`, `alt` and slash bass. Each
      symbol has chord tones with roles (`root`, `3rd`, `5th`, `7th`,
      `tension`).
- [ ] A default scale for every chord quality, from one table with the
      reason for each entry. The sheet can override it.
- [ ] `Harmony`: root, scale, chord tones with roles, available tensions
      and avoid notes. It is built from a chord symbol, a scale, or both.
- [ ] The sheet's sections (named by region, length in bars, meter) and
      harmony spans (symbols or scales, each lasting some bars or beats).
      Parse errors give the line, the column and the fix.
- [ ] Tests: every grammar case, round trips of symbols to text, the
      default scales, and a sheet with one modal section and one with
      chord symbols.
