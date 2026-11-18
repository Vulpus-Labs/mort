---
id: "0002"
title: "mort-session: the Session trait and FakeSession"
priority: high
created: 2026-11-18
epic: E001
depends: ["0001"]
---

## Summary

Define the `Session` trait of ADR 0011 §1, covering everything the
bridge will need from REAPER in E001. Implement `FakeSession` over an
in-memory project. Its renders come in 0012, once the capture path exists.

## Acceptance criteria

- [x] The trait covers: project and track reads, FX and parameter
      read/write, envelopes, MIDI, regions, the tempo map, state count,
      track chunk text, undo blocks with labels, transport and render
      start/stop.
- [x] `FakeSession` moves the state count on every edit, gives stable chunk
      text per track that changes when the track changes, and keeps an undo
      stack with labels.
- [x] Unit tests for each of those behaviours.

## Close-out (2026-11-18)

- `mort-proto::model` holds the records that cross the wire:
  `ProjectInfo`, `Region`, `TrackInfo`, `SendInfo`, `FxInfo`, `ParamInfo`,
  `ItemInfo`, `EnvelopeTarget`, `EnvelopeInfo`, `EnvelopePoint` and
  `Shape`, `Note`, `ControlChange`. Times are seconds, gains linear and
  parameters normalised.
- `mort-session`:
  - `Session` covers the project, tracks and sends, chunks, FX and
    parameters (including `format_param` for setting from display text),
    FX state and presets, envelopes, MIDI items, undo blocks, and
    transport. Errors are `ErrorObject`s built with `not_found` and
    `invalid` helpers.
  - `FakeSession` keeps the whole project in memory. Undo restores
    snapshots, as REAPER does. Chunks are the track serialised. Plugins
    come from a catalogue: a one-band ReaEQ, ReaComp and the probe.
- Deviation: there is no render method yet. Its shape depends on whether
  REAPER's API allows a dry run, so ticket 0009 adds it to the trait.
  Ticket 0012 gives the fake audio.
- 12 integration tests.
