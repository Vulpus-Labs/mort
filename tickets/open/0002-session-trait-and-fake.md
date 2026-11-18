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

- [ ] The trait covers: project and track reads, FX and parameter
      read/write, envelopes, MIDI, regions, the tempo map, state count,
      track chunk text, undo blocks with labels, transport and render
      start/stop.
- [ ] `FakeSession` moves the state count on every edit, gives stable chunk
      text per track that changes when the track changes, and keeps an undo
      stack with labels.
- [ ] Unit tests for each of those behaviours.
