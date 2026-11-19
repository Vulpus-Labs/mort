# ADR 0002: The bridge's RPC: socket, dispatch and jobs

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

REAPER's API may be called only from the main thread. Some operations take
seconds or minutes (playing a range, rendering), and the client must not
hold REAPER's UI up while they run. Several clients may connect at once, for
example an agent and a person's shell.

## Decision

1. **Transport:** JSON-RPC 2.0, one message per line, over a Unix domain
   socket at `$TMPDIR/mort/<reaper-pid>.sock`. The bridge also writes
   `$TMPDIR/mort/current` with the path of the most recently started
   instance. The client uses `current` unless `--socket` is given. The
   socket is created mode 0600, so only the local user can connect.
2. **Dispatch:** a socket thread parses requests and puts them on a queue.
   A timer callback registered with REAPER (`plugin_register("timer")`, run
   at UI rate) drains the queue on the main thread, with a time budget of
   about 8 ms per tick so the UI stays responsive. Responses go back
   through the socket thread. One method is the exception: `map.watch`
   (ADR 0012 §8) is answered by the socket thread itself, from a change
   feed the main thread updates, so a client waiting for changes never
   holds a place in the queue.
3. **Jobs:** any request that waits on audio (`play`, `render`, `measure`)
   returns a job id at once. `job.wait(id, timeout)` blocks the client, not
   the main thread. `job.status` and `job.cancel` do what they say. A job
   moves through `queued`, `running`, then `done`, `failed` or `cancelled`.
   Its result is kept until it is read or 10 minutes have passed.
4. **Notifications:** a client may subscribe to `project.changed` (state
   count moved, ADR 0004) and `job.finished`.
5. **Versioning:** `bridge.hello` returns the protocol version and the
   bridge and probe build ids. The client refuses to talk to a different
   major version and names both versions in the error.
6. **Errors** use JSON-RPC error codes plus a `data` object holding:
   - `kind`: `not_found`, `stale`, `invalid`, `busy` or `reaper`
   - `subject`: the id of the element the error is about
   - `fix`: one sentence saying what to do
7. **Batch references.** In a `batch`, any string parameter of the form
   `$N.field.field…` is replaced by that part of call N's result, so a
   call can use what an earlier call in the batch made, such as `$0.track`
   for a track that call 0 added. `mort run` relies on this (ADR 0009 §4).
   A reference to a call that has not run, or to a field its result lacks,
   fails the batch.

## Consequences

- REAPER stays usable while mort works. A long render is a job, not a
  frozen UI.
- At UI rate each request costs at least one timer tick, roughly 30 ms.
  `batch` (an array of calls run in one tick, all or nothing for edits)
  keeps chatty sequences fast.
- Anyone with the user's own account on the machine can drive REAPER.
  That is the same trust as the user's own shell.
