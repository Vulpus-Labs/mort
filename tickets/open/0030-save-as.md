---
id: "0030"
title: "project.save with a path should save as, not save a copy"
priority: medium
created: 2026-11-20
epic: E004
depends: []
---

## Summary

`project.save {path}` calls `Main_SaveProjectEx`, which writes a copy:
the open project stays untitled and dirty, so it has no sidecar, and
`mort compose write` refuses it. Found in the E004 proof (0029). A
project built from scratch by mort cannot be composed into without a
restart.

## Acceptance criteria

- [ ] `project.save {path}` leaves the open project at that path, as
      *Save as* does, or a separate call does that and `project.save`'s
      docs say it saves a copy. Decide which, and record it in ADR 0004 or
      0009.
- [ ] The map follows the new path (ADR 0006 §6), as it does now when the
      person saves.
- [ ] Smoke: save to a new path, then `mort compose show` finds the sheet
      beside it.
