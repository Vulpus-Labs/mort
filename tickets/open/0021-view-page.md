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

## Progress (2026-11-19), not yet closed

What's built:

- **The page** (`crates/mort-cli/assets/view.html`) is one file of
  HTML, CSS and JavaScript, embedded in `mort`, with no build step.
  - The grid of tracks is nested by folder, with a time ruler showing
    regions and bars. It zooms in seven steps from 0.05 to 5 s a column
    and scrolls in time, drawing only the visible columns on a canvas
    the size of the window.
  - Cells are shaded by age in one hue, from under a minute to older
    than a day, with a legend; coverage sets their opacity. Stale cells
    are hatched, with the reason on hover. Never-measured cells are
    empty, and rows that cannot be measured say why (`no probe`,
    `muted`).
  - Findings are corner triangles whose size and colour (amber, orange,
    magenta: no red against green) grow with severity. Hovering shows
    the kind, severity and tracks.
  - The master lane shows short-term loudness (-40 to 0 LUFS) and ▲
    where the true peak passes the project's headroom threshold.
  - The play cursor is drawn on its own layer from `cursor` events. A
    `change` event refetches the view and redraws only the cells that
    differ.
  - Clicking a cell shows `map.track` for its section and the evidence
    of its findings. The thresholds tab edits overrides, and can reset
    them.
  - Light and dark come from `prefers-color-scheme`.
  - The status line shows the time of each draw.
- **`map.view` gained** `bars`, `lane` (per column: highest short-term
  loudness and true peak) and `headroom`. Staleness is memoised per
  track and capture: on a 46-track, 5-minute map at 600 columns a fetch
  went from 0.55 s to 0.10 s, with 2 MB of JSON.
- **`examples/fake_bridge.rs`** (`cargo run --release -p mort-bridge
  --example fake_bridge -- 40 5`) serves a big fake session on a socket
  for `mort view`. Measure it in 30 s pieces: a single 5-minute measure
  of 46 tracks did not finish within the CLI's 10-minute wait, because
  the fake renders a whole range in memory at once. Each 30 s piece took
  about 3.3 s.

Still to check by hand, with the person, in 0022:

- The look: light and dark, legend, hatching, markers, master lane.
- The cursor following playback.
- Draw time under 16 ms for an update on the 40-track, 5-minute
  session. The status line shows it. Headless Chrome would not start in
  the agent's sandbox, so it is not measured yet.
