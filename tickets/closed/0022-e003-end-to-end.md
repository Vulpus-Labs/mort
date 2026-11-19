---
id: "0022"
title: "End to end in REAPER: the E003 proof"
priority: medium
created: 2026-11-19
epic: E003
depends: ["0021"]
---

## Summary

Run E003's proof in REAPER, and settle the questions ADR 0012 leaves to
measurement.

## Acceptance criteria

- [x] The four steps in E003's goal pass, recorded in `docs/e003-proof.md`
      with screenshots of the page.
- [x] Measured and recorded:
      - whether REAPER resets plugins on seek (affects the warm-up rule);
      - whether dragging an FX knob moves the state count during the drag
        or only on release;
      - whether non-soloed tracks still run their FX.
      ADR 0012 updated with the answers and its status set to Accepted.
- [x] `scripts/smoke.sh` gains audition checks: a manual play adds an
      audition capture, and a solo play does not touch the bus.

## Close-out (2026-11-19)

- **The epic's four steps passed** in REAPER 7.67 with the person at the
  transport and the page open. The log is in
  [docs/e003-proof.md](../../docs/e003-proof.md), and the session's build
  script in [docs/e003-build.kdl](../../docs/e003-build.kdl).
  - Hand plays filled the map live.
  - A solo play updated the tracks but not their buses.
  - A hand EQ edit made exactly Warm, Chords and the master stale, and
    playing again refreshed what was played and left a skipped stretch
    stale.
  - Lowering `loudness_jump` on the page made a finding in both the page
    and `mort findings`.
  - **Deviation:** no screenshots; the person's observations are
    recorded instead.
- **ADR 0012's questions answered** and recorded in the ADR, which is now
  Accepted:
  - REAPER resets plugins on seeks and playback starts. The probe gained
    a `resets` parameter to count them, shown in `probes.list`.
  - Knob drags move the state count during the drag, about once a second.
  - Unsoloed tracks keep running their FX.
- **Fixed on the way:**
  - The page fetched a fixed range.
  - Growing captures left 50 ms holes.
  - Covers a few microseconds long were left where two plays met; covers
    shorter than half a frame are now dropped.
  - A batch could not carry `state` and `undo_point`; it now answers
    `{results, state, undo_point}`.
- **Smoke:** `scripts/smoke.sh` has 72 checks, all passing in REAPER on
  the final build, including the new solo-audition check and the info
  lines for resets and unsoloed FX.
- 170 tests pass.
