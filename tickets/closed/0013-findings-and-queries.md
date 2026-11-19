---
id: "0013"
title: "Findings and queries; calibration (synthetic; Fenland in 0017)"
priority: high
created: 2026-11-18
epic: E001
depends: ["0012", "0008"]
---

## Summary

Write the queries and finding kinds of ADR 0007 §2–4, then calibrate
thresholds on Fenland, where we know what needed fixing.

## Acceptance criteria

- [x] `summary`, `track` and `frames` queries, with CLI output short
      enough for an agent (a section summary of 20 tracks fits in about 60
      lines).
- [x] Finding kinds `masking`, `mud`, `harsh`, `headroom`, `loudness_jump`,
      `low_end_width` and `register_clash`, each with evidence and a
      suggestion.
- [x] ~~Calibration: render Fenland's stems (from the song render in the
      silkscreen scratch pipeline or the videbimus fenland crate), measure
      them, and check findings against known problems and fixes. Put the
      thresholds in one table with reasons. Record the results in the
      close-out.~~ Calibrated on synthetic mixes with planted problems
      instead, at the user's choice; Fenland moves to 0017.
- [x] Each finding kind has a fake-session test that plants the problem
      and expects the finding, and a clean case that does not produce
      it.

## Close-out (2026-11-19)

- **Analysis in `mort-map`.**
  - `analysis` splits a range into sections by regions, computes each
    track's statistics per section (loudness, peaks, crest, correlation,
    centroid, flatness, onset density, tilt, side level, mean bands), and
    holds the `View` over map and project.
  - `findings` covers all seven kinds, each with evidence, a one-line
    suggestion and a stale flag.
  - `query` provides `summary`, `track` and `frames`.
  - `thresholds` holds the constants, the `Limits` findings run with
    (`DEFAULT`, and `ALL` for `--all`), and the table of reasons.
- **Bridge:** `map.summary`, `map.track`, `map.frames`, `map.findings
  {kinds?, all?}` and `map.thresholds`. Track gains to their bus come from
  fader, pan (as a balance) and mute; MIDI notes come from each track's
  MIDI items.
- **CLI:** `mort summary`, `mort findings [--kind] [--all] [--limit]
  [--thresholds]`, `mort report <track>` and `mort frames <track> <range>
  -f …`. The test's section of 20 tracks prints in 25 lines.
- **The fake** gained `Pink` and `Wide` (decorrelated) sources, and mixes
  in stereo.
- **Tests:**
  - `mort-bridge/tests/findings.rs` has a clean mix with no findings, and
    each of the seven kinds planted and found. Mud, harsh and loudness
    jump also check that a weaker plant is not flagged, and
    register_clash checks a clean voicing.
  - The CLI test covers 20 tracks with one planted double.
- **Calibration, on synthetic material** (the user's choice). The clean
  mix is five pink parts in neighbouring ranges at -26 dB; problems are
  planted one at a time. Measured with `cargo test -p mort-bridge --test
  findings -- --ignored --nocapture`:

  | Measure | Clean | Planted | Threshold |
  |---|---|---|---|
  | masking: third octaves over 30 % for a pair | ≤ 2 (crossovers) | 5–10 | 3 (new) |
  | mud excess, dB | -0.7 to 0.1 | 2.9 (at the others' level), 0.8 (6 dB quieter) | 2.0 (was 3) |
  | harsh excess, dB | 0.3 to 1.2 | 3.3 (at the others' level), 1.7 (6 dB quieter) | 2.0 (was 3) |
  | loudness jump, LU | 0.0 | 1.1 / 5.7 / 8.9 (pink added at -24 / -14 / -10) | 6 (kept) |
  | low-end correlation | 1.0 | 0.01 (track), 0.11 (master) | 0.5 (kept) |
  | master true peak, dBTP | -6.2 to -4.2 | above -1 when planted | -1 (kept) |
  | onset density, per s | 0.0 (steady noise) | 2.3 for 2.0 played | refractory of 2 frames added |

  Every value and its reason is in `mort_map::thresholds::TABLE`.
- **Found while calibrating:**
  - Masking below 708 Hz is generous. The low bands' 683 ms window
    spreads energy across neighbouring bands, so same-level parts whose
    ranges overlap by a few percent can compete in three bands there.
    This is noted for 0017.
  - The fake's first CLI fixture overlapped every part by two octaves and
    produced 50 masking findings. Each was real, but there were too many
    to read, so `mort findings` now shows the 20 worst and says how many
    it left out.
- **Deviations:**
  - Calibration was on synthetic material, not Fenland; Fenland is
    deferred to ticket 0017.
  - Track contributions ignore automation and REAPER's pan law (ADR 0007
    §2).
- **REAPER:** this ticket adds no REAPER calls. The bridge's queries read
  tracks, items and notes, which `scripts/smoke.sh` already checks. Three
  smoke checks for the queries were added and will first run with ticket
  0015's REAPER pass.
