---
id: "0018"
title: "Freshness and the change feed: map.view, map.watch, findings cache"
priority: high
created: 2026-11-19
epic: E003
depends: ["0017", "0019"]
---

## Summary

Give clients the whole map as a grid, and tell them when it changes
(ADR 0012 §6 and §8).

## Acceptance criteria

- [x] The map carries a version, bumped on capture added, staleness
      change (state count moved), and threshold change.
- [x] `map.view {start, end, step}`: rows in folder order; per column
      coverage, newest source and time, stale reason, unmeasured reason,
      and findings over threshold with severity.
- [x] `map.watch {since, timeout}` answered on the socket thread from
      shared state the timer updates. It never queues on the dispatcher.
      It returns the version, the changed tracks and spans, and the
      transport position. ADR 0002 amended.
- [x] Findings cached per section and recomputed only where data or
      thresholds changed. A test shows that an audition over one section
      recomputes only that section.
- [x] Fake tests: watch wakes on an audition, on an edit (stale), and on
      a threshold change, and times out otherwise.

## Close-out (2026-11-19)

- **The feed** (`feed.rs`) has a version, a log of changes (capture,
  stale, thresholds or project, each with tracks and spans) and the
  transport position, behind a mutex and condvar.
  `Server::start_with_feed` answers `map.watch {since, timeout_ms}` on
  the connection's reader thread from it. The test answers a timed-out
  watch without ticking the dispatcher at all. REAPER and the CLI test
  bridge use it.
- **Changes announced:**
  - Captures (measure and audition) as they are added.
  - Thresholds and project-path changes at once.
  - Staleness when the hash cache is ready for a new state, listing the
    tracks whose stale spans changed.
  - The auditioner's tick records the transport for the feed.
- **`map.view {start, end, step}`:** rows (master, then tracks in order,
  with folder depth and an unmeasured reason) and compact cells: coverage,
  source, time, stale reason, finding kinds and severity. It also returns
  the findings, sections, version and the sections it recomputed.
- **Findings cache:** `findings::section_findings`, `jump_findings` and
  `assemble` split the computation. The bridge caches each section's
  findings under a signature of the state, the overrides and the parts of
  covers inside it.
- **Live filling:** stretches commit every 2 s into one growing capture
  (`MixMap::extend`, which updates the tempo fingerprint and joins
  neighbouring covers). Before this, the map filled only when playback
  stopped.
- **Fixed on the way:**
  - The shared hash cache lost the old state's snapshot before a stretch
    under it finished, so it now keeps four states.
  - Clearing the staleness baseline after a capture lost an edit that
    came before the next tick; the baseline now keeps its state and is
    refreshed quietly.
  - Section signatures used whole cover spans, so splitting a cover
    elsewhere recomputed untouched sections; they now use only the part
    inside the section.
- **Tests:**
  - Feed unit test.
  - `tests/view.rs`: the grid (order, depth, coverage, source, time,
    muted, masking markers, stale reasons after an edit); findings
    recomputed only for the section an audition touched, and for every
    section after a threshold change; `map.watch` over a real socket
    waking on an audition, an edit and a threshold change, and timing
    out otherwise.
  - A long-playback audition test.
  - 167 tests in all.
- **REAPER:** three smoke checks added for `map.view` and `map.watch`.
  They run with ticket 0022's REAPER pass.
