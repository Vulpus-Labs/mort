---
id: "0012"
title: "Captures: assembly, staleness, sidecar persistence, measure --stale"
priority: high
created: 2026-11-18
epic: E001
depends: ["0011"]
---

## Summary

Assemble frames from a run into a capture, keep the mix map, mark
staleness, persist (ADR 0006). Make `FakeSession` render through
`mort-features` so all of this runs in `cargo test` (ADR 0011 §2).

## Acceptance criteria

- [ ] `measure(range, tracks?, mode)` ensures probes, starts a render or
      play run, collects frames, and stores a capture with chunk hashes.
- [ ] Staleness per ADR 0006 §3, including routing and tempo changes.
      Tested over the fake: editing one track makes it and its buses
      stale, and nothing else.
- [ ] `--stale` re-measures only stale tracks and ranges, together with
      the buses they reach.
- [ ] Sidecar save and reload with the project. Unsaved projects keep
      captures in memory only.
- [ ] `mort map` prints what the map covers and what is stale.
