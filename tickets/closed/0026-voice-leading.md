---
id: "0026"
title: "Voice-leading: Viterbi over candidates; labelled blocks; non-chord tones"
priority: high
created: 2026-11-20
epic: E004
depends: ["0025"]
---

## Summary

Choose one voicing per harmony change, at the least total cost, and
produce labelled blocks (ADR 0013 §6, §7).

## Acceptance criteria

- [x] Viterbi over the candidates. Hard constraints: ranges, voice
      count, a required top note, and literal voicings given in the
      sheet. Soft costs, weighted per part: movement, leaps, common tones,
      low-interval spacing, parallel fifths and octaves, crossing.
- [x] Deterministic tie-breaking. The same input gives the same blocks,
      tested.
- [x] When the hard constraints cannot all be met, the error names the
      section, bar and constraint, and suggests what to relax.
- [x] Blocks: start, length, and notes labelled with voice (`bass`,
      `inner1…`, `top`) and role.
- [x] Optional non-chord tones between blocks: passing, neighbour,
      suspension, anticipation and pedal, each labelled.
- [x] Sheet syntax for a part's voicing spec, cost weights, literal
      voicings and top-note constraints.
- [x] Decide whether walking bass is a line generator here or an
      articulation (ADR 0013, Consequences), and record the choice in the
      ADR.
- [x] Tests: a ii–V–I keeps common tones; a forced top line is honoured;
      an impossible range fails with a clear error; passing tones fill
      thirds when switched on.

## Close-out (2026-11-20)

- **Parts.** `part` nodes in the sheet (module `part`) take:
  - `voicing NAME voices=N range="LOW..HIGH"`;
  - `weights …` (movement, leaps, common, spacing, parallels, crossing,
    centre);
  - `nct KIND… beats=N`;
  - `pedal PITCH`;
  - `in SECTION…`;
  - per-section overrides in `section NAME { … }`;
  - `top SECTION NOTE…`, where `-` leaves a note free;
  - `fix SECTION bar=N beat=N "PITCHES"`.

  A part may come before the sections it names. Names are checked once
  the whole sheet is read, and duplicate part names are refused.
- **Voice-leading** (module `lead`).
  - `voice_part(sheet, part, vocabularies)` runs Viterbi over every
    harmony the part plays, in form order. The unary costs are spacing
    against a table of low-interval limits, and distance from the
    range's middle. The transition costs are movement, leaps, held
    common tones, parallels and crossing.
  - Ties go to the earlier candidate, and a test checks that the same
    sheet voices the same way.
  - The chain restarts where the voice count changes.
- **Blocks.** Each block has its section, start, length, harmony and
  notes. Each note carries its voice (`bass`, `innerN`, `top`), its role
  over the harmony it sounds against, chord or non-chord tone, and its
  own start and length.
- **Non-chord tones.**
  - Passing, neighbour, suspension and anticipation go in the inner and
    top voices, the first kind in the part's order that fits each voice.
    `pedal` replaces the bass.
  - A suspension must be in the next harmony's scale. Found by printing
    output: without this, E♮ was held over E♭ lydian, and A over the A♭
    of G7b9.
- **Errors.** Each names the part, section, bar and beat, and says what
  to relax. The cases are:
  - no voicing for a section;
  - an unknown vocabulary;
  - an impossible range or voice count;
  - a top note no candidate has;
  - a `fix` with the wrong voice count, with notes outside the harmony,
    or not on a harmony's start.
- **Decided: walking bass is articulation.** ADR 0013 §7 is amended
  with this, the non-chord-tone rules and the cost terms (§6). Its
  "thirds below about E2" became the usual low-interval limits (a minor
  third no lower than C3).
- **Printed output checked by eye.**
  - A rootless ii–V–I in C runs Dm9 C E F A, G13 B E F A, Cmaj9
    B D E G, with every voice moving by step, plus a 4–3 suspension
    into Cmaj9.
  - Quartal refuses G7b9: half-whole diminished has no four-note chain
    of fourths. The error names bar 6.
- **Tests.** 9 in `tests/lead.rs`:
  - a ii–V–I keeps at least two common tones and moves by step;
  - determinism;
  - a top line;
  - unmet constraints;
  - passing tones filling thirds (only when switched on, never in the
    bass);
  - suspension, anticipation, neighbour and pedal;
  - suspensions only within the next scale;
  - form order with overrides;
  - part parse errors.

  All four project checks pass.
- **Not done here:** pitch names print with sharps (`A#3` for B♭). Showing
  pitches spelled from the harmony belongs to `mort compose show` (0028).
