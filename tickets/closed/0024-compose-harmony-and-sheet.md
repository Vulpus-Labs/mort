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

- [x] Crate `mort-compose`, with no dependency on REAPER or the bridge.
- [x] Pitch classes and pitches with spelling (`C4` is 60, as in `mort
      midi`), intervals.
- [x] Scales as data: the modes of major, melodic minor and harmonic
      minor, whole tone, both diminished scales, the pentatonics and the
      blues scale, with names and aliases (`dorian`, `lydian dominant`,
      `altered`, …).
- [x] A chord-symbol grammar: root, quality, sevenths, extensions,
      alterations, `sus`, `add`, `omit`, `alt` and slash bass. Each
      symbol has chord tones with roles (`root`, `3rd`, `5th`, `7th`,
      `tension`).
- [x] A default scale for every chord quality, from one table with the
      reason for each entry. The sheet can override it.
- [x] `Harmony`: root, scale, chord tones with roles, available tensions
      and avoid notes. It is built from a chord symbol, a scale, or both.
- [x] The sheet's sections (named by region, length in bars, meter) and
      harmony spans (symbols or scales, each lasting some bars or beats).
      Parse errors give the line, the column and the fix.
- [x] Tests: every grammar case, round trips of symbols to text, the
      default scales, and a sheet with one modal section and one with
      chord symbols.

## Close-out (2026-11-20)

- **Crate.** `mort-compose` depends only on `kdl`, which is now a
  workspace dependency shared with `mort-cli`. Its modules are `pitch`,
  `scale`, `chord`, `harmony` and `sheet`.
- **Pitches.** These are named as in `mort midi` (`C4` is 60), with
  spellings of up to two sharps or flats, not mixed. The CLI keeps its
  own `parse_pitch`. The two can be merged when the CLI starts using
  this crate (0028).
- **Scales.** 27 scales with aliases. Tests check that every mode is a
  rotation of its parent.
- **Chord symbols.**
  - The grammar is as listed in `chord`'s module doc, including
    `ø`, `Δ`, `mMaj7`, `6/9`, alterations in brackets, `alt`,
    `add`/`omit` and slash bass.
  - Each symbol prints in one canonical form, and that form reads back
    as the same symbol (tested on 18 spellings).
  - Roles gained `2nd`, `4th` (sus) and `6th`, and tensions carry their
    degree. ADR 0013 §7 is amended to match.
- **Deviation: default scales.** The ticket asked for one table entry
  per chord quality. Instead each chord family has a list of scales in
  order of preference, and the chord takes the first that holds every
  note its symbol asks for. One table with a reason per entry still
  covers every combination of alterations (`G7b9` → half-whole,
  `G7b9b13` → phrygian dominant, `G7#9#5` → altered) without listing
  them. 27 symbols are tested. ADR 0013 §4 is amended.
- **Harmony.**
  - Avoid notes are scale notes a semitone above a chord tone (F over
    Cmaj7, C over G7, none in Dorian).
  - A bare scale's chord tones are its 1st, 3rd, 5th and 7th notes.
    Scales with other than seven notes take the nearest notes they have:
    whole tone gives C E G# Bb, and whole-half gives a diminished
    seventh.
  - A named scale (`scale="aeolian"`, or a root and scale such as
    `G7sus4` over `D dorian`) replaces the default. Chord tones outside
    it are allowed.
- **Sheet.**
  - `meter` sets the default time signature, and `section` takes a name,
    `bars` and an optional `meter`, with `harmony` children.
  - A `harmony` node takes one or more symbols or scales, each lasting
    `bars` or `beats` (fractions allowed if they come to whole ticks at
    960 per quarter), with an optional `scale`.
  - A section with no harmony is allowed. One with harmony must be
    covered exactly. Duplicate section names and unknown nodes are
    refused.
  - Every error gives the line, the column and what to do.
- **Tests.** 16 unit tests and 3 sheet tests: a modal section, a section
  of chords, meters and overrides, and every error path. All four
  project checks pass.
