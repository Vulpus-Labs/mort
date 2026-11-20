---
id: "0033"
title: "CLAP parameter changes read back stale until audio runs"
priority: medium
created: 2026-11-20
epic: E004
depends: []
---

## Summary

Found while programming Poly-Kritter for the ambient piece. `param.set`
on a CLAP plugin answers with the new value, but a later `fx.params`
shows the old one until REAPER processes audio (one bar of `mort play`
applied it). REAPER queues the change for the plugin's next process or
flush. A script that sets and then reads, or an agent that checks its
work, sees the wrong value.

Undo is affected too. Undoing a later MIDI write on the Bass track set
Poly-Kritter's `Sustain` back to Off, though it had been set On several
edits earlier. REAPER's undo snapshot seems to hold the plugin's state
from before the queued changes were applied.

## Acceptance criteria

- [ ] Find out in REAPER whether the value is only reported stale or not
      applied at all until processing: render without playing, and check
      the sound.
- [ ] `param.set` makes the change visible to the next read: by asking
      the plugin to flush, or by reading back through the same path. If
      neither is possible, `fx.params` says which values are pending.
- [ ] Smoke check: set a CLAP parameter while stopped, then read it.
- [ ] Smoke check: set a CLAP parameter, make a MIDI edit, undo the MIDI
      edit; the parameter keeps its value.
