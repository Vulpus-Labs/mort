---
id: "0006"
title: "Edits: tracks, FX, params, mix controls; undo points, expect_state, the wheel, undo, batch"
priority: high
created: 2026-11-18
epic: E001
depends: ["0005"]
---

## Summary

Write the edit RPCs and the rules of ADR 0004.

## Acceptance criteria

- [x] Edits:
  - tracks: add (name, position, parent folder), rm, rename;
  - volume, pan, mute, solo;
  - sends: add and set;
  - FX: add by plugin name or id at a position in the chain, rm, move,
    bypass;
  - presets: load by name, and get/set state chunk;
  - params: set normalised, or set from display text.
- [x] Each edit is one undo point labelled `mort: <summary>`, and `batch` is
      one point.
- [x] `expect_state` mismatch gives `stale`. `project.changed` says whose
      change it was.
- [x] The *mort: hold edits* action toggles the wheel. Edits give `busy`
      while it is held, and reads still work.
- [x] `undo` refuses unless the top of the undo stack is a `mort:` point.
- [x] CLI: `track add|rm|set`, `fx add|rm|bypass|preset`, `set`,
      `undo`, `redo`.

## Close-out (2026-11-19)

- Session and bridge:
  - `Live` implements every edit in the ticket over reaper-low.
  - Track insertion handles folders. `track.add` with a parent appends
    to the folder's end and moves the closing depth onto the new track.
  - `DeleteTrack` leaves folder depths as they were, so `remove_track`
    first moves the deleted track's depth change onto its neighbours.
    Children of a deleted folder move up a level, as in the fake.
  - FX state is the FX's block cut out of the track chunk (`chunk.rs`,
    unit-tested), because REAPER has no call for one FX's state. On the
    wire it is `chunk`, so it can't collide with the project `state` that
    every read adds.
  - New on the trait: `redo_label` and `save_project`.
- RPCs:
  - `track.add|rm|set`, `send.add|set`, `fx.add|rm|move|enable|preset`,
    `fx.state`, `fx.set_state`, `param.set` and `project.save`.
  - `param.set` takes `value` (normalised) or `text`. Text that matches a
    display exactly (switches, modes) gives the middle of that run of
    values. Otherwise the first number in the text, with `k` meaning
    thousands, is bracketed by 200 samples, and the bisection finds both
    edges of the band of values that display it, then takes the middle.
  - `undo` and `redo` are dispatcher built-ins. They refuse unless the
    point is `mort:`, and refuse while edits are held. Edit results
    include the new `state`.
- *mort: hold edits*: a command id, an action-list entry, a hook command
  and a toggle state. Checked by hand in REAPER 7.67: holding makes
  `mort status` report `held yes` and edits fail with `busy` while reads
  work. Releasing restores edits. The action list shows the on/off state.
- CLI:
  - `mort track add|rm|set`, `mort send add|set`,
    `mort fx add|rm|move|bypass|enable|preset` (`mort fx <track>` still
    lists), `mort set <track> <fx> <param> <value> [--normalised]`,
    `mort undo` and `mort redo`.
  - Volumes in dB, pans as `C`, `L20` or `R35`.
  - Each edit sends the state it read as `expect_state`, plus a readable
    `summary`.
- Found by the smoke run and fixed:
  - Rollback compared the state count across the whole call, and REAPER
    moves that count when an undo block opens. A failed no-op edit
    therefore undid the edit before it. Rollback now looks only inside its
    own block, and only through its own point (ADR 0004 §7). The fake now
    moves the count on `begin_undo` too, and a dispatcher test covers it.
  - `Main_SaveProjectEx` writes a copy and leaves the project dirty. The
    smoke project is quit with SIGTERM, which doesn't prompt.
- Smoke: 32 checks, all `ok` in REAPER 7.67. The new ones cover: adding
  at an index and into a folder, track set, sends, FX add at a position,
  move, bypass, a missing plugin, a missing preset, parameters by text
  (`-3` → `-3.0 dB`, `2400` → `2400.0 Hz`) and by value, FX state restore,
  refusing another kind of state, stale, undo and redo, removal keeping
  folders, and project.save.
- Tests: CLI 22, dispatcher 8.
