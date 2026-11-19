---
id: "0017"
title: "Calibrate finding thresholds on Fenland's stems"
priority: medium
created: 2026-11-19
epic: E001
depends: ["0013"]
---

## Summary

Ticket 0013 calibrated the finding thresholds on synthetic mixes with
planted problems (the user's choice). Check them against real material
where we know what needed fixing: Fenland's stems, from the song render in
the silkscreen scratch pipeline or the videbimus fenland crate.

## Acceptance criteria

- [ ] Render Fenland's stems into a REAPER project, measure them, and
      list the findings with `mort findings --all`.
- [ ] Compare them with the known problems and fixes. Adjust thresholds
      where real material disagrees with the synthetic margins, and
      update the reasons in `mort_map::thresholds::TABLE`.
- [ ] Check masking below 708 Hz in particular: the low bands' 683 ms
      window spreads energy across neighbouring bands, so parts whose
      ranges barely overlap can compete in three bands there.
