---
id: "0005"
title: "Read model: project, tracks, routing, FX, params, regions, tempo map, items"
priority: high
created: 2026-11-18
epic: E001
depends: ["0004"]
---

## Summary

Write the read RPCs and implement them in `ReaperSession`. Every read
returns `state`. Add CLI commands `tracks`, `fx` and `params`, with name
resolution (ADR 0003 §2).

## Acceptance criteria

- [x] `project.get`: name, path, sample rate, length, tempo map, regions
      and markers, `state`.
- [x] `tracks.list`: id, name, folder depth, volume, pan, mute/solo,
      sends and receives, the parent bus, the FX chain (id, name, plugin
      id, bypass).
- [x] `fx.params`: index, name, normalised value, display text, and step
      count where the plugin reports one.
- [x] `items.list` per track: position, length, take, and whether it is
      MIDI.
- [x] Ambiguous names in the CLI list the matches. Smoke covered in
      REAPER.

## Close-out (2026-11-18)

- Bridge:
  - `raw.rs` wraps the reaper-low calls for reading tracks, sends, FX,
    parameters, items, takes, markers and the project path. REAPER's GUID
    struct converts straight to `Guid`.
  - `Live` now reads tracks (folder parent, main send, volume, pan,
    mute, solo, sends, FX chain), FX (`fx_name` config as `plugin`),
    parameters (normalised value, display text, toggles as 2 steps),
    `format_param`, items with their active take, regions and markers,
    and the project path.
  - `methods.rs` registers `project.get`, `tracks.list`, `track.get`,
    `fx.params`, `items.list` and `time.convert`. Every read returns
    `state`. `time.convert` turns position and range text into seconds
    and back through the session's tempo map, so the CLI never needs a
    tempo map of its own.
- CLI:
  - `mort tracks`: numbered, folders indented, dB, pan, flags, FX and
    sends.
  - `mort fx <track>` and `mort params <track> <fx> [filter]`.
  - `resolve.rs` picks tracks and FX by number (from 1, as REAPER shows
    them), GUID, exact name, or a unique piece of a name. Ambiguous or
    missing names list the candidates. The master is named only by
    `master` or its id, so name pieces never match it.
- Smoke: `scripts/smoke-project.lua` builds a known project; REAPER runs it
  when it is given on the command line. `scripts/smoke.sh` checks tracks,
  folder, send, mix controls, FX chain with bypass, master, parameters,
  regions and markers, the MIDI item and time conversion. All `ok` in
  REAPER 7.67.
- What REAPER actually does, found by the smoke run:
  - `TimeMap_QNToMeasures` returns the bar number counted from 1, though
    reaper-medium names it `measure_index`. `TimeMap_GetMeasureInfo`
    takes an index from 0.
  - FX names include the format prefix (`VST: ReaEQ (Cockos)`) until
    renamed. The fake now does the same.
  - `Main_SaveProjectEx` saves a copy; the open project stays untitled.
- 12 CLI tests (5 new) and 4 unit tests for resolving and formatting.
