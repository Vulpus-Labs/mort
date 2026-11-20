---
id: "0034"
title: "Scripts: sends added in a held batch are invisible to later steps"
priority: low
created: 2026-11-20
epic: E004
depends: []
---

## Summary

In `mort run`, the reads that later steps make show the tracks and FX
that held edits add (ADR 0009 §4), but not the sends. `send add Drone
Space` followed by `send set Drone Space volume=-12` in the same batch
fails with "Drone has no send to Space". The ambient build had to
commit between them.

## Acceptance criteria

- [ ] The overlay on `tracks.list` adds held sends, with placeholder
      indices that the batch fills in, as it does for tracks and FX.
- [ ] CLI test: `send add` then `send set` in one batch, with one undo
      point.
