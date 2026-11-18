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
   Renders for measurement go to a scratch directory (ADR 0006).

## Consequences

- The person can review mort's history in REAPER's undo history window.
- A busy person and a busy agent will see `stale` errors. That is the
  intended outcome: the error is cheap, and a wrong edit is not.
