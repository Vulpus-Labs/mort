---
id: "0014"
title: "compare, and mort run scripts"
priority: high
created: 2026-11-18
epic: E001
depends: ["0013", "0007"]
---

## Summary

Judge edits by comparing captures. Run sequences of commands as scripts
(ADR 0009 §4).

## Acceptance criteria

- [x] `compare(a, b, range)`: changes in features per track and section,
      and findings that appeared or disappeared. The CLI defaults to the
      latest two captures that cover the range.
- [x] `mort run file.kdl`: edits become one `batch` carrying
      `expect_state`. A failure stops the script and names the step.
- [x] Tested over the fake: plant masking, measure, apply an EQ cut by
      script, re-measure, compare shows the finding gone.

## Close-out (2026-11-19)

- **The map keeps history.**
  - Each capture keeps its own frames. Tracks hold covers that point at
    the capture that is current over each stretch.
  - Wholly replaced captures are kept as superseded, the latest 8 of
    them, and `MixMap::as_of(id, range)` rebuilds the map as it was
    after any capture.
  - Each capture records every track's gain, bus and MIDI at the time.
  - Sidecar format 2 writes each capture's frames once, with covers in
    the index (ADR 0006 §6–7).
- **`compare`** (`mort_map::compare`, RPC `map.compare`, `mort compare
  [range] [--a] [--b] [--min]`):
  - Shows per-section, per-track changes in loudness, RMS, peak,
    centroid, tilt, side and low correlation, with the three third
    octaves that moved most.
  - Shows findings gone, new and kept, with severity before and after.
  - Defaults to the latest capture over the range and the one before it.
- **`mort run file.kdl`:**
  - Each KDL node is a command line, and every step is parsed before any
    runs.
  - Edits are held in the client and sent as one `batch`, with the
    script's undo label and the first edit's `expect_state`.
  - `tracks.list` shows tracks and FX the held edits add, with
    placeholder ids such as `$0.track`.
  - Jobs and `commit` send the batch first, and a job then expects the
    state the batch left.
  - A failure names the step and its line; held edits are not sent.
- **Bridge:**
  - `batch` calls may refer to earlier results with `$N.field…` (ADR 0002
    §7).
  - `param.set` takes a parameter `param` by name: exact, then prefix,
    then substring. "Q-Band" first matched "Freq-Band 1" as a substring.
- **CLI:** `main` now runs every command through one `step` function on a
  shared connection, which scripts use.
- **Tests:**
  - The acceptance test: two pads masking, measure, then a script adds
    ReaEQ to Pad 2 and cuts 300 Hz by 24 dB, re-measures and compares.
    The masking finding is gone, and Pad 2 moved about -23 dB at
    250–400 Hz. One undo removes the four scripted edits.
  - A failed step stops the script and sends nothing; a usage error is
    caught before anything runs; `commit` makes two batches.
  - Map tests cover history and `as_of`; a unit test covers batch
    references.
- **Found:** an FX added after a track's probe pushes the probe off the
  end. The next `measure` moves it back, as an undo point of its own
  (`mort: add probes to measure`).
- **REAPER:** two smoke checks were added (compare after the smoke's
  two captures, and a batch with references and a parameter by name).
  They will run with 0015's REAPER pass. This ticket adds no REAPER calls.
