---
id: "0031"
title: "Voice parts against each other: no semitone rubs between parts"
priority: medium
created: 2026-11-20
epic: E004
depends: []
---

## Summary

Each part is voice-led on its own (ADR 0013 §6). In the E004 proof the
quartal pad held A3 under Gm9 while the drop-2 keys had B♭3, and
`mort findings` reported a register clash. Parts that sound together
need a cost, or a constraint, against minor seconds (and minor ninths
low down) between their notes.

## Acceptance criteria

- [x] Decide how parts see each other: voice them in an order (say,
      keys before pad), each later part paying a cost for semitones
      against the earlier ones, or voice them jointly. Record it in ADR
      0013 §6.
- [x] The weight is a part setting, with a default set by listening.
- [x] Fake test: the E004 chorus voices without a semitone between the pad
      and keys below C4, unless forced by `fix` or `top`.

## Close-out (2026-11-20)

- **Order.** `lead::voice_in_context` voices the parts that voice
  themselves in sheet order. Each part is voiced against the blocks of
  the parts before it (`voice_part_against`). The first part is voiced
  exactly as alone. `play_part` and `mort compose show` use it, and
  parts that play another's voices (`from`) take that part's voicing in
  context. ADR 0013 §6 is amended.
- **Costs.** Two new weights:
  - `clash`, default 30, per semitone between this part's notes and an
    earlier part's notes sounding at the same time, a minor ninth
    counting half;
  - `bottom`, default 1, when the lowest note is a tension.
- **Default set by measurement and listening.**
  - At 6, the proof's quartal pad still held A D G C against the keys'
    B♭ in Gm9: holding still across eight chords saved more than a rub
    cost.
  - At 30 it takes a clash-free voicing wherever one exists.
  - Where none exists (four-voice quartal over C13 and A7alt beside
    drop-2 keys), it cannot help. The sheet has to separate the ranges.
    The E004 proof's second arrangement does, and its register clashes
    are gone.
- **Test.** `parts_are_voiced_against_the_parts_before_them`:
  - the first part is unchanged;
  - the pad voiced alone rubs in Gm9;
  - voiced in context, it rubs only where every quartal candidate would.
