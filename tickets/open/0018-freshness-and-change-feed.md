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

- [ ] The map carries a version, bumped on capture added, staleness
      change (state count moved), and threshold change.
- [ ] `map.view {start, end, step}`: rows in folder order; per column
      coverage, newest source and time, stale reason, unmeasured reason,
      and findings over threshold with severity.
- [ ] `map.watch {since, timeout}` answered on the socket thread from
      shared state the timer updates. It never queues on the dispatcher.
      It returns the version, the changed tracks and spans, and the
      transport position. ADR 0002 amended.
- [ ] Findings cached per section and recomputed only where data or
      thresholds changed. A test shows that an audition over one section
      recomputes only that section.
- [ ] Fake tests: watch wakes on an audition, on an edit (stale), and on
      a threshold change, and times out otherwise.
