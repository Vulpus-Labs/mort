---
id: "0006"
title: "Edits: tracks, FX, params, mix controls; undo points, expect_state, the wheel, undo, batch"
priority: high
created: 2026-11-18
epic: E001
depends: ["0005"]
---

## Summary

Write the edit RPCs and the rules of ADR 0004.

## Acceptance criteria

- [ ] Edits:
  - tracks: add (name, position, parent folder), rm, rename;
  - volume, pan, mute, solo;
  - sends: add and set;
  - FX: add by plugin name or id at a position in the chain, rm, move,
    bypass;
  - presets: load by name, and get/set state chunk;
  - params: set normalised, or set from display text.
- [ ] Each edit is one undo point labelled `mort: <summary>`, and `batch` is
      one point.
- [ ] `expect_state` mismatch gives `stale`. `project.changed` says whose
      change it was.
- [ ] The *mort: hold edits* action toggles the wheel. Edits give `busy`
      while it is held, and reads still work.
- [ ] `undo` refuses unless the top of the undo stack is a `mort:` point.
- [ ] CLI: `track add|rm|set`, `fx add|rm|bypass|preset`, `set`,
      `undo`, `redo`.
