---
id: "0015"
title: "End to end in REAPER: the E001 proof"
priority: high
created: 2026-11-18
epic: E001
depends: ["0014"]
---

## Summary

Run the proof in E001's goal in a real REAPER session, using only `mort`
commands, and record it.

## Acceptance criteria

- [ ] Session built from scratch: four to six tracks, at least two
      silkscreen instruments, two regions, MIDI and automation.
- [ ] Planted bass/pad masking and a planted mud build-up are both
      reported.
- [ ] ReaEQ edits through `mort` clear them, and `compare` shows it.
- [ ] A ReaEQ edit made by hand shows that track and its buses as stale.
- [ ] Undoing a mort edit from REAPER's undo history, then re-measuring
      with `--stale`, brings the finding back.
- [ ] The full command log, with outputs, is kept in `docs/e001-proof.md`.
