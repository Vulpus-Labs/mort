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

- [ ] Decide how parts see each other: voice them in an order (say,
      keys before pad), each later part paying a cost for semitones
      against the earlier ones, or voice them jointly. Record it in ADR
      0013 §6.
- [ ] The weight is a part setting, with a default set by listening.
- [ ] Fake test: the E004 chorus voices without a semitone between the pad
      and keys below C4, unless forced by `fix` or `top`.
