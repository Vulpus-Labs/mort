---
id: "0032"
title: "register_clash ignores drum MIDI"
priority: medium
created: 2026-11-20
epic: E004
depends: []
---

## Summary

`register_clash` (ADR 0007 §2) reads MIDI as pitched notes. On a drum
track the notes are kit pieces: in the E004 proof it reported ReaSynDr's
side stick, note 51, as D♯3 clashing with the pad's E3.

## Acceptance criteria

- [ ] Notes on MIDI channel 10 are not pitches for `register_clash`.
- [ ] A track can be marked as drums for findings, for drum instruments
      that listen on other channels. Decide where that mark lives (track
      name, a sheet part's pattern, or a project setting), and record it
      in ADR 0007.
- [ ] Fake test: a channel-10 note a semitone from a sustained bass note
      raises no finding. The same note on channel 1 does.
