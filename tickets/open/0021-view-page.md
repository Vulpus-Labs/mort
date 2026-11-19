---
id: "0021"
title: "The map view page"
priority: medium
created: 2026-11-19
epic: E003
depends: ["0020"]
---

## Summary

Draw the whole map in the browser (ADR 0012 §10).

## Acceptance criteria

- [ ] Grid of tracks (nested by folder) by time, with regions and bars
      across the top. It zooms and scrolls in time.
- [ ] Cells shaded by data age, with a legend. Stale cells hatched with
      their reason on hover. Never-measured cells empty, and unmeasured
      cells marked with their reason.
- [ ] Findings markers coloured by severity. Hovering shows kind and
      tracks.
- [ ] Master lane: short-term loudness and true-peak overs.
- [ ] Play cursor follows the transport. Cells update as events arrive,
      without redrawing the whole grid.
- [ ] Clicking a cell shows the `track` query for that section and the
      evidence of its findings. The thresholds panel edits overrides.
- [ ] Readable in light and dark. Colours do not rely on red/green alone.
- [ ] Checked by hand with a 40-track, 5-minute fake session for speed:
      an update draws in under 16 ms.
