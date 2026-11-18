---
id: "0011"
title: "mort-probe: CLAP plugin, attach, ring, slot identity, stamping"
priority: high
created: 2026-11-18
epic: E001
depends: ["0009", "0010"]
---

## Summary

The probe of ADR 0005 and the bridge's side of it: the registered attach
function, the rings, finding probe instances in chains, and inserting
probes where needed.

## Acceptance criteria

- [ ] clack CLAP plugin. Stereo pass-through, bit-exact. No UI.
      Read-only `slot` and `attached` parameters.
- [ ] Attach through `cockos.reaper_extension` → `GetFunc("mort_probe_attach")`.
      Confirm in REAPER that `GetFunc` finds an `API_` function registered by
      another extension. If it does not, record the finding and choose a
      fallback (named shared memory) in ADR 0005.
- [ ] SPSC ring, no allocation, locks or system calls on the audio thread
      (checked with an allocation-counting test harness). Dropped frames
      are counted.
- [ ] Frames stamped from CLAP transport plus sample offset, with the
      current run id. Tested in REAPER: frames line up within one block in
      live play and in render, across a tempo change.
- [ ] Bridge: `probes.ensure(tracks?)` inserts probes last in each chain and
      on the master, as one undo point. `probes.list` maps slots to tracks
      and reports detached instances.
