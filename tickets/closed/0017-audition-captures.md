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

- [x] `Capture` gains `source` (`render`, `play`, `audition`). The sidecar
      format version is bumped and old sidecars load as `render`/`play`.
- [x] Stretches split on stop, stamp jumps (seek, loop wrap, gap over 1.5
      frames), state-count changes, and changes to solo set, play rate or
      record monitoring.
- [x] Validity per ADR 0012 §2: play rate 1.0; record-monitored tracks
      and their downstream dropped; under solo, only tracks and buses with
      no silenced input kept; warm-up drops the first 683 ms and marks
      short-term loudness unknown before 3 s.
- [x] Chunk hashes cached per state count and refreshed across timer
      ticks within the budget. A stretch waits for the refresh.
- [x] Solo dropped from the audio chunk (`MUTESOLO` keeps mute). ADR 0006
      §1 amended in this change.
- [x] Stretches from one playback written as one `.frames` file at stop.
- [x] `Session` gains play state, position, play rate, solo set and
      record-monitoring reads. The fake can play without a run, with
      seeks and loops driven by the test.
- [x] Fake tests: plain audition fills the map; seek splits; edit mid-play
      splits and attributes frames to the right state; solo keeps the
      soloed leaf and drops its bus; play rate 1.5 keeps nothing; short
      stretch keeps nothing.
- [x] Check how `measure` treats the first frames of a run, and record
      whether the warm-up rule should apply there too.

## Close-out (2026-11-19)

- **Map** (`mort-map`):
  - `Capture` gains `source` (render, play or audition; sidecars before
    format 3 call it `mode`, and it still reads) and `file`, the frames
    file it lives in.
  - Sidecar format 3 groups a playback's stretches in one file; format 2
    files still load.
  - Covers shorter than the stamp slack are dropped, so the slivers
    rounding left between meeting spans go.
  - Unknown values are NaN, and queries and loudness jumps skip them.
- **Bridge** (`audition.rs`):
  - The `Auditioner` runs on a new dispatcher tick hook. It takes run-0
    frames, which were never taken before and piled up in the buffers.
  - It splits stretches on stop, stamp jumps (seek, loop wrap, gaps),
    and changes to state, solo, play rate or monitoring; frames on the
    tick of a change are dropped.
  - It applies ADR 0012 §2's validity rules and warm-up, and adds what
    survives as `audition` captures.
  - The `HashCache` hashes a few tracks per tick (2 ms budget) after
    each change. Finished stretches wait for it, and are dropped if the
    state moves on first.
  - The map is saved when playback stops and every stretch is in.
- **Session:** `play_rate` and `monitored` are new; the solo set comes
  from `tracks()`. In REAPER they read `Master_GetPlayRate` and
  `I_RECARM` with `I_RECMON` = 1, and the `MUTESOLO` line keeps only its
  mute.
- **The fake** now plays incrementally: probe meters persist across
  advances and reset on a seek, as the real probe does. It also has
  `seek`, `set_loop`, `set_play_rate` and `set_monitored`, solo that
  silences as REAPER does, and solo left out of the audio chunk.
- **mort's `play`** starts no run, so it auditions.
- **Measure warm-up** (the last criterion): a measure's first frames were
  wrong. The 63 Hz band read -59 dB against a steady -28 and took about
  0.6 s to recover, momentary loudness took 0.4 s, and short-term
  loudness 3 s. Measures now pre-roll (render 3 s, play 0.7 s, in whole
  frames) and clip the pre-roll (ADR 0006 §4).
- **Tests:**
  - 6 audition tests: plain playback after the warm-up; seek and loop
    splits; an edit mid-play splitting with each part keeping its state;
    solo keeping leaves and dropping buses; solo not making the map
    stale; rate 1.5, short stretches and monitored tracks keeping
    nothing; one playback written as one frames file.
  - A hash cache unit test, a pre-roll test, and a format 2 sidecar
    test.
  - 159 tests in all.
- **REAPER 7.67:** `scripts/smoke.sh` passes all 68 checks. "live play"
  now checks for an audition capture over 0.7–2.25 s: REAPER played
  about 0.25 s past the job's end before the job stopped it, and the map
  keeps what was heard.
- **Left for 0018:** an unbroken playback adds nothing until it ends, so
  a live view needs stretches committed every few seconds (ADR 0012,
  notes).
