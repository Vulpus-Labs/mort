---
id: "0012"
title: "Captures: assembly, staleness, sidecar persistence, measure --stale"
priority: high
created: 2026-11-18
epic: E001
depends: ["0011"]
---

## Summary

Assemble frames from a run into a capture, keep the mix map, mark
staleness, persist (ADR 0006). Make `FakeSession` render through
`mort-features` so all of this runs in `cargo test` (ADR 0011 §2).

## Acceptance criteria

- [x] `measure(range, tracks?, mode)` ensures probes, starts a render or
      play run, collects frames, and stores a capture with chunk hashes.
- [x] Staleness per ADR 0006 §3, including routing and tempo changes.
      Tested over the fake: editing one track makes it and its buses
      stale, and nothing else.
- [x] `--stale` re-measures only stale tracks and ranges, together with
      the buses they reach.
- [x] Sidecar save and reload with the project. Unsaved projects keep
      captures in memory only.
- [x] `mort map` prints what the map covers and what is stale.

## Close-out (2026-11-19)

- **The fake renders.** `FakeSession` tracks take test sources (`Source`:
  sine, noise, band-limited noise, and a voice that plays the track's MIDI
  notes). The fake mixes them through folders, sends, faders, pan and
  ReaEQ's band, and feeds each probe's input to the real `mort-features`
  analyzer. `Session` gained `begin_run`, `end_run`, `take_frames` and
  `audio_chunk`. In REAPER these use the run atomic, the probe rings and
  the chunk with view lines and probes cut out (`chunk::audio_chunk`).
- **`mort-map`** holds `Capture`, `Segment`, `MixMap`, `Routing` and
  `Now`. A capture replaces older frames over its range on the tracks it
  measured, and captures left empty are dropped. Frames are clipped to
  the range with a 1 µs allowance for rounding in the stamps. Staleness
  reports why: `changed`, `routing` or `tempo`. `store` writes and reads
  the sidecar.
- **Bridge.** `measure {start, end, tracks?, mode, stale?}` is an edit
  that adds probes and queues a job, and `map.status` reports coverage.
  The dispatcher now lets a single edit queue a job; a batch still may
  not. The map follows the project path (ADR 0006 §6).
- **CLI.** `mort measure RANGE [TRACKS…] [--play] [--stale] [--no-wait]`
  and `mort map`.
- **Tests.**
  - mort-map: 5 tests covering clipping, replacement, staleness by edit,
    routing and tempo, and the sidecar round trip;
  - fake: 2 tests covering renders through the feature code, folders,
    EQ, mute and stopped playback;
  - bridge: 3 tests covering measure, edit, `--stale`, play mode, held,
    errors, and save and reload;
  - CLI: `measure_and_map`.
- **REAPER 7.67.** `scripts/smoke.sh` passes all 63 checks, 7 of them
  new.
  - The audio-chunk hash holds steady through renders and renames.
  - A volume change on Kick makes exactly Kick, Drums, Verb (Kick sends
    to it) and MASTER stale, and `--stale` re-measures just those four.
- **Found in REAPER.** It does not run a muted track's FX, so the muted
  Bass sent no frames. Measure now names that reason, the fake skips
  muted tracks too, and ADR 0006 §4 and ADR 0011 §2 record it.
- **Not checked in REAPER:** the sidecar. The smoke project is never
  saved under its own name, so it has no path. Save and reload are
  covered over the fake.
- **Deviation:** `--stale` re-measures the stale tracks over one span,
  the union of their stale stretches, rather than one run per stretch.
  It measures more than strictly needed, and stays correct.
