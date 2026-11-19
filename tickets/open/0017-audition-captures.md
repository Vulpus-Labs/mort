---
id: "0017"
title: "Audition captures: stretches, validity, warm-up, cached hashes"
priority: high
created: 2026-11-19
epic: E003
depends: ["0012"]
---

## Summary

Fold run-0 probe frames into the map as `audition` captures (ADR 0012
§1–5). Today they are buffered and discarded.

## Acceptance criteria

- [ ] `Capture` gains `source` (`render`, `play`, `audition`). The sidecar
      format version is bumped and old sidecars load as `render`/`play`.
- [ ] Stretches split on stop, stamp jumps (seek, loop wrap, gap over 1.5
      frames), state-count changes, and changes to solo set, play rate or
      record monitoring.
- [ ] Validity per ADR 0012 §2: play rate 1.0; record-monitored tracks
      and their downstream dropped; under solo, only tracks and buses with
      no silenced input kept; warm-up drops the first 683 ms and marks
      short-term loudness unknown before 3 s.
- [ ] Chunk hashes cached per state count and refreshed across timer
      ticks within the budget. A stretch waits for the refresh.
- [ ] Solo dropped from the audio chunk (`MUTESOLO` keeps mute). ADR 0006
      §1 amended in this change.
- [ ] Stretches from one playback written as one `.frames` file at stop.
- [ ] `Session` gains play state, position, play rate, solo set and
      record-monitoring reads. The fake can play without a run, with
      seeks and loops driven by the test.
- [ ] Fake tests: plain audition fills the map; seek splits; edit mid-play
      splits and attributes frames to the right state; solo keeps the
      soloed leaf and drops its bus; play rate 1.5 keeps nothing; short
      stretch keeps nothing.
- [ ] Check how `measure` treats the first frames of a run, and record
      whether the warm-up rule should apply there too.
