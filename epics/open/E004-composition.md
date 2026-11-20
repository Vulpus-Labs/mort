---
id: E004
title: "Composition: parts from a song sheet"
status: open
created: 2026-11-20
---

## Goal

Write guide parts, comping and drums from a song sheet, following ADR
0013. The sheet gives harmony, voicing and articulation as separate
layers, and changing one layer regenerates only what depends on it.

The proof (ticket 0029) is a session in REAPER built from a sheet:

- two sections, one modal (a scale held for 8 bars), one with chord
  symbols;
- a pad with quartal voicings, keys playing drop-2 voicings in a bossa
  nova pattern with anticipations, a bass walking the keys' bass voice,
  and drums from a kit map;
- `mort compose show --blocks` agrees with what is heard;
- changing one chord regenerates only the items that depend on it;
- changing the keys' articulation regenerates only the keys;
- a note edited by hand in REAPER protects that item until `--force`.

Decisions: ADR 0013, with ADR 0008's sidecar and section naming.

## Tickets

| Ticket | What |
|---|---|
| 0024 | `mort-compose`: pitches, scales, chord-symbol grammar, harmony; sheet form and harmony |
| 0025 | Voicing vocabularies as data, and candidate generation |
| 0026 | Voice-leading: Viterbi over candidates, constraints, costs; labelled blocks; non-chord tones |
| 0027 | Articulation and performance: patterns, anticipation, playability, drums and kit maps, swing and feel |
| 0028 | `mort compose show` and `write`: items per section, provenance, staleness per layer, hand edits |
| 0029 | End to end in REAPER: the proof above |

## Dependency order

0024, then 0025, then 0026, then 0027, then 0028, then 0029.

## Follow-ups found by the proof

| Ticket | What |
|---|---|
| 0030 | `project.save` with a path saves a copy, not *Save as* |
| 0031 | Parts are voiced alone; they need to avoid semitones against each other |
| 0032 | `register_clash` counts drum MIDI as pitches |
