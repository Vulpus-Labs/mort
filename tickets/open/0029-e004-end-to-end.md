---
id: "0029"
title: "End to end in REAPER: the E004 proof"
priority: medium
created: 2026-11-20
epic: E004
depends: ["0028"]
---

## Summary

Run E004's proof in REAPER and listen to the result.

## Acceptance criteria

- [ ] The steps in E004's goal pass, recorded in `docs/e004-proof.md`
      with the sheet, the commands and their output.
- [ ] Listened to. The default cost weights and pattern velocities are
      adjusted by ear where they sound wrong, and the reasons recorded
      (ADR 0013, Consequences).
- [ ] ADR 0013 updated with what changed, and its status set to Accepted.

## Notes from earlier tickets

- 0028: with the default weights, a quartal pad over Gm9 C13 Fmaj9
  Bbmaj7#11 holds E A D G through all four chords, because that stack
  fits each of them and holding costs least. Listen: if the pad should
  follow the changes, weigh the bottom voice towards chord tones, or
  lower `common`.
- 0027: bossa piano over `rootless` voicings doubles the 7th or 3rd in
  the left hand. Use drop-2 or shell voicings for a root bass.
