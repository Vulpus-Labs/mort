# ADR 0004: Edits, undo, and sharing the session with the user

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

mort edits a session that a person may be editing at the same moment. The
person must be able to see what mort did and take it back. mort must not
act on a picture of the project the person has since changed.

## Decision

1. **Every edit request is one undo point.** The dispatcher wraps it in
   `Undo_BeginBlock2` / `Undo_EndBlock2`, labelled `mort: <summary>`, for
   example `mort: bass EQ −3 dB at 280 Hz`. The client supplies the summary
   and the bridge prefixes it. A `batch` of edits is one undo point.
2. **Optimistic concurrency.** Every read returns the project's
   `GetProjectStateChangeCount` as `state`. Every edit may carry
   `expect_state`. If the count has moved, the edit fails with `stale`, and
   the client re-reads and decides again. The CLI sends `expect_state` by
   default when it has read the state in the same invocation or the same
   `mort session`.
3. **mort's own edits** move the count too. The bridge records the count
   after each of its own edits. `project.changed` notifications say whether
   the last change was mort's or someone else's.
4. **The wheel.** A REAPER action, *mort: hold edits*, makes the bridge
   refuse every edit with `busy` until the action is toggled off. It shows as
   a toggle in the action list and on a toolbar button. Reads and
   measurements go on working.
5. **Undo from the client.** `undo` and `redo` are RPCs. `undo` refuses if
   the top of REAPER's undo stack is not a `mort:` point, so mort never
   undoes the person's work.
6. **No destructive file operations.** mort never deletes media files and
   never saves the project over its file unless asked with `project.save`.
   Given a path, `project.save` writes a copy there and leaves the
   project's own file and unsaved changes alone, which is what REAPER's
   `Main_SaveProjectEx` does. `project.get` reports `dirty`. Renders for
   measurement go to a scratch directory (ADR 0006).
7. **Rollback goes through the failed edit's own undo point.** REAPER
   7.67 moves the state count by one when an undo block closes, even an
   empty one, and not when it opens or inside it (measured in ticket
   0015). So the count cannot say whether an edit changed anything, and
   whether it made an undo point is judged by the top undo label becoming
   its own. An edit whose label equals the one on top is numbered (`mort:
   add bass (2)`) so the two can be told apart. A failed edit is rolled
   back only if it made its own undo point, so it can never undo the edit
   before it. (Until 0015 this judged by the count inside the block, which
   REAPER never moves, so failed edits were not rolled back in REAPER; the
   fake had modelled REAPER wrongly.)
8. **Edits say whether they left an undo point.** REAPER records only the
   changes it is told about. MIDI calls, for example, need the item marked
   dirty. An edit that REAPER failed to record would make the next `undo`
   revert something older. So every edit result carries `undo_point`, and
   the smoke script fails if any of its edits comes back without one.

## Consequences

- The person can review mort's history in REAPER's undo history window.
- A busy person and a busy agent will see `stale` errors. That is the
  intended outcome: the error is cheap, and a wrong edit is not.
