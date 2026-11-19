---
id: "0019"
title: "Threshold overrides per project"
priority: medium
created: 2026-11-19
epic: E003
depends: ["0013"]
---

## Summary

Let a project override the measured finding thresholds and the minimum
severity shown (ADR 0012 §7). The defaults stay those of 0013.

## Acceptance criteria

- [ ] `song.mort/thresholds.toml` read on load and when the project path
      changes. Unknown keys and out-of-range values are errors that name
      the key and its allowed range.
- [ ] `thresholds.get` returns each threshold's default, override and
      reason. `thresholds.set` writes the file and bumps the map version.
      Unsaved projects keep overrides in memory.
- [ ] Every finding lists any overridden thresholds it used.
- [ ] `mort thresholds` shows them, and `mort thresholds set KEY VALUE` /
      `reset KEY` change them. ADRs 0007 §3 and 0009 amended.
- [ ] Fake test: raising the mud threshold clears a planted mud finding,
      and resetting brings it back.
