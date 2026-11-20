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

- [x] A KDL format for patterns: hits at bar positions, each selecting
      notes by voice and role, with length, accent, octave doubling or
      shift, hand or instrument, and `anticipate`. Each pattern says how
      it treats non-chord tones.
- [x] Patterns are anchored to the bar and run across chord changes.
      Anticipating hits take the next block's notes.
- [x] Playability: hand span (a tenth by default) and instrument range.
      Hits that cannot be played are reported with bar and beat, and not
      moved.
- [x] Drums: hits on kit pieces, and kit maps from pieces to note
      numbers, built in (General MIDI) and user-defined.
- [x] Performance: swing, timing deviation and velocity curves, seeded
      per part and section.
- [x] Built-in patterns: sustained, block on each beat, arpeggio up and
      down, bossa piano, walking (if 0026 makes it an articulation), and
      basic rock and bossa drum grooves.
- [x] Tests: the bossa pattern over a two-chord bar anticipates the
      second chord; bass in octaves doubles only the bass voice; a span
      too wide for a hand is reported; the same seed gives the same feel.

## Close-out (2026-11-20)

- **Patterns** (module `pattern`, built-ins in `patterns/builtin.kdl`).
  - A pattern holds `hit` lines, one `arp`, or one `walk`, and takes
    `bars`, `drums` and `nct`.
  - A hit takes `at`, `length`, `velocity`, `accent` (+20), `octave`,
    `double`, `hand` and `anticipate`.
  - `hand NAME span=N` sets a hand's reach.
  - Kits are `kit NAME { piece NAME NOTE }`.
  - Patterns load from the built-ins, then the user's and the project's
    `patterns/` directories. Errors give the file, line and column.
- **Built-ins.** Eight patterns:
  - sustain, block, arpeggio up, arpeggio down;
  - bossa piano: the bass in octaves on 1 and 3 in the left hand, a
    two-bar figure in the right, and an accented push on 2:4&;
  - walking;
  - rock drums, bossa drums.

  One kit, `gm`.
- **Parts** gained:
  - `articulation NAME range=… kit=…`, where the range is the
    instrument's, for checking;
  - `feel swing= timing= velocity= seed=`;
  - `from PART voice="…"`.

  Articulation and feel can be set per section. The sheet checks that
  `from` names a part it has.
- **Playing** (module `articulate`).
  - `play_part(sheet, part, vocabularies, patterns)` returns events
    (section, start, length, pitch, velocity) and playability problems.
  - Drum sections play even with no harmony. Pitched sections play
    their blocks.
  - Hits are anchored to the section's bars and run across chord
    changes. `anticipate` takes the adjacent next block's notes.
  - Hand spans and the instrument's range are reported with bar and
    beat. The notes are still written as they are.
- **Feel** (module `perform`). Swing moves off-beat eighths, and timing
  and velocity deviate by a fixed SplitMix64. No new dependency.
- **Bug fixed before closing.** A position past the end of a shorter
  meter's bar (beat 4 in 3/4) would have landed on the next bar's
  downbeat. Such positions are now dropped.
- **Printed output checked by eye.**
  - A Dm9–G13–Cmaj9–A7alt bossa: Cmaj9's upper voices pushed in on
    2:4&.
  - A shell part's walking bass: D E F F♯ | G A G B | C B C B♭ | A …,
    approaching each root by a semitone.
- **Tests.** 8 in `tests/articulate.rs`:
  - the bossa push;
  - an anticipating hit in a two-chord bar;
  - bass in octaves doubling only the bass voice;
  - hand span and instrument range reported, not moved;
  - walking bass from another part's bass voice;
  - drums through the kit map, and an unknown piece;
  - seeded feel, including the swing position;
  - pattern errors.

  There are 2 unit tests (positions and the built-ins). All four project
  checks pass. ADR 0013 §8 is amended.
- **Noted for 0029 (by ear).** Bossa piano over `rootless` voicings
  doubles the 7th or 3rd in the left hand, because that is the voicing's
  lowest voice. Pair it with drop-2 or shell voicings for a root bass,
  or give the keys a separate bass part.
