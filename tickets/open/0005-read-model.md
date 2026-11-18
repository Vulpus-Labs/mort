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

- [ ] `project.get`: name, path, sample rate, length, tempo map, regions
      and markers, `state`.
- [ ] `tracks.list`: id, name, folder depth, volume, pan, mute/solo,
      sends and receives, the parent bus, the FX chain (id, name, plugin
      id, bypass).
- [ ] `fx.params`: index, name, normalised value, display text, and step
      count where the plugin reports one.
- [ ] `items.list` per track: position, length, take, and whether it is
      MIDI.
- [ ] Ambiguous names in the CLI list the matches. Smoke covered in
      REAPER.
