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

- [x] clack CLAP plugin. Stereo pass-through, bit-exact. No UI.
      Read-only `slot` and `attached` parameters.
- [x] Attach through `cockos.reaper_extension` → `GetFunc("mort_probe_attach")`.
      Confirm in REAPER that `GetFunc` finds an `API_` function registered by
      another extension. If it does not, record the finding and choose a
      fallback (named shared memory) in ADR 0005.
- [x] SPSC ring, no allocation, locks or system calls on the audio thread
      (checked with an allocation-counting test harness). Dropped frames
      are counted.
- [x] Frames stamped from CLAP transport plus sample offset, with the
      current run id. Tested in REAPER: frames line up within one block in
      live play and in render, across a tempo change.
- [x] Bridge: `probes.ensure(tracks?)` inserts probes last in each chain and
      on the master, as one undo point. `probes.list` maps slots to tracks
      and reports detached instances.

## Close-out (2026-11-19)

- `mort-link` (new crate): the probe/bridge contract.
  - `ATTACH_API` (`mort_probe_attach`), `LINK_VERSION`, and `Slot`, which
    holds the slot id, attached/active flags, sample rate, push and drop
    counts, a reference to the bridge's run atomic, and a 2048-record SPSC
    ring on the heap (about 450 KB, too big for REAPER's stack).
  - Each `Record` is a run id, a start in project seconds, and a
    `mort_features::Frame`.
  - Tests: FIFO order, full ring, and 200,000 records across two threads
    with none lost.
- `mort-probe`: clack, pinned to silkscreen's rev.
  - Stereo pass-through (in place, or copied when separate). Read-only
    `slot` and `attached` parameters, shown as text.
  - Attaches in `new_shared` through the `cockos.reaper_extension` host
    extension (REAPER's `reaper_plugin_info_t`) and `GetFunc`. Clears
    `attached` when dropped.
  - Audio-thread work is in `Meter`:
    - it analyses only while the transport plays with a seconds timeline;
    - it resets on a jump of more than 1 ms;
    - it stamps `start = song_pos + (end_offset − len) / rate` and the run
      id.
  - Tests: stamps exact to 1e-9 over a second of 512-sample blocks, stop
    and restart, and zero allocations on the audio path (a per-thread
    counting allocator).
- Bridge:
  - `probes.rs`: `attach` (registered with `RegistrationObject::api`),
    the slot registry, the 5 ms drain thread, `take`, `peek` and `counts`.
  - `probes.ensure`: one probe last on every track and the master, extras
    removed, one undo point.
  - `probes.list`: slot from the `slot` parameter's display, joined with
    the registry.
  - `probes.peek`: a run's frames, for checking.
- CLI: `mort probes`, `mort probes ensure [TRACKS] [--no-master]`.
  `mort install` builds the `.clap` bundle (Info.plist, PkgInfo) from
  `libmort_probe.dylib`.
- REAPER 7.67 (release build), smoke, 56 checks, all `ok`:
  - every probe attached, active at 48 kHz, last, with no frames dropped;
  - a ReaSynth note dry-rendered over 0–4 s gives exactly 80 frames from
    0 s, 50 ms apart (to 1e-6), stamped with the render's run, at −8.6
    LUFS momentary;
  - live play over 0–2 s gives 40 frames on the same grid;
  - a render across the tempo change at bar 9 gives 80 frames from
    14.0 s.
  - REAPER processes about 3 frames past a range's end. Captures clip
    them (ADR 0006 §5).
- Deviations: the drain runs on a bridge thread, not the main-thread
  timer (ADR 0005 §4 updated). The probe is tested through `Meter`, not
  through a CLAP host harness. Pass-through is checked by reading
  (copy/in-place) and the smoke run, not by a sample comparison.
- The smoke client's socket timeout is now 30 s. One `fx.add` of a missing
  plugin took more than 5 s right after a new CLAP was installed, probably
  REAPER searching its plugin list.
