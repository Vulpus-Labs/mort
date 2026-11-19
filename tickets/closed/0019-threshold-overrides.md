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

- [x] `song.mort/thresholds.toml` read on load and when the project path
      changes. Unknown keys and out-of-range values are errors that name
      the key and its allowed range.
- [x] `thresholds.get` returns each threshold's default, override and
      reason. `thresholds.set` writes the file and bumps the map version.
      Unsaved projects keep overrides in memory.
- [x] Every finding lists any overridden thresholds it used.
- [x] `mort thresholds` shows them, and `mort thresholds set KEY VALUE` /
      `reset KEY` change them. ADRs 0007 §3 and 0009 amended.
- [x] Fake test: raising the mud threshold clears a planted mud finding,
      and resetting brings it back.

## Close-out (2026-11-19)

- **`mort_map::thresholds`:**
  - `Limits` now holds every finding threshold (16 of them), with
    `get`/`set` by name checked against a min and max in `TABLE`.
  - `Overrides` holds a project's values and `min_severity`, and reads
    and writes the flat TOML file. Unknown keys, non-numbers and values
    out of range are errors naming the key and its range.
  - `used_by(kind)` says which thresholds each finding uses.
  - The onset constant stays in the table but cannot be overridden.
- **Findings** use only `Limits` (no constants left in `findings.rs`) and
  carry `overridden` (omitted when empty).
- **Bridge:**
  - The store reads `<project>.mort/thresholds.toml` when the project
    path changes, writes overrides held in memory when a project is
    first saved, and reports a broken file and uses the defaults.
  - It has a `version`, bumped on threshold changes and path changes;
    0018 adds map changes.
  - `thresholds.get` and `thresholds.set {name, value | reset}` are new;
    `map.thresholds` is gone.
  - `map.findings`, `map.summary` and `map.compare` use the overrides and
    hide findings under `min_severity` (`--all` ignores both).
- **CLI:** `mort thresholds [set KEY VALUE | reset KEY]` marks overridden
  values with `*` and says where the file is. `mort findings
  --thresholds` prints the same table.
- **Tests:**
  - Parsing and its errors.
  - Bridge: mud cleared by `mud_excess = 4`, kept and marked overridden
    at 2.5, back on reset; `min_severity` hides it in findings and
    summary; errors; the file is written on save and read by a new
    bridge; a broken file is reported.
  - The CLI set and reset round trip.
  - 162 tests in all.
- **REAPER:** no REAPER calls changed.
