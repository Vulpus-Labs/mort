---
id: "0027"
title: "Articulation and performance: patterns, anticipation, playability, drums, feel"
priority: high
created: 2026-11-20
epic: E004
depends: ["0026"]
---

## Summary

Turn blocks into notes with patterns, and apply feel (ADR 0013 §8).

## Acceptance criteria

- [ ] A KDL format for patterns: hits at bar positions, each selecting
      notes by voice and role, with length, accent, octave doubling or
      shift, hand or instrument, and `anticipate`. Each pattern says how
      it treats non-chord tones.
- [ ] Patterns are anchored to the bar and run across chord changes.
      Anticipating hits take the next block's notes.
- [ ] Playability: hand span (a tenth by default) and instrument range.
      Hits that cannot be played are reported with bar and beat, and not
      moved.
- [ ] Drums: hits on kit pieces, and kit maps from pieces to note
      numbers, built in (General MIDI) and user-defined.
- [ ] Performance: swing, timing deviation and velocity curves, seeded
      per part and section.
- [ ] Built-in patterns: sustained, block on each beat, arpeggio up and
      down, bossa piano, walking (if 0026 makes it an articulation), and
      basic rock and bossa drum grooves.
- [ ] Tests: the bossa pattern over a two-chord bar anticipates the
      second chord; bass in octaves doubles only the bass voice; a span
      too wide for a hand is reported; the same seed gives the same feel.
