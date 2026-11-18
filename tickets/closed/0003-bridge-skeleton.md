---
id: "0003"
title: "Bridge skeleton: extension loads, socket, main-thread dispatcher, jobs, bridge.hello"
priority: high
created: 2026-11-18
epic: E001
depends: ["0002"]
---

## Summary

Build `reaper_mort.dylib` on `reaper-medium`. Start the socket thread and
the timer-driven dispatcher with its per-tick budget (ADR 0002). Add the job
table, notifications and `bridge.hello`. The dispatcher is generic over
`Session`, so all of this runs over `FakeSession` in tests.

## Acceptance criteria

- [x] REAPER (macOS, current release) loads the extension. It logs its
      version to the REAPER console and creates the socket and `current`
      file, mode 0600. They are removed on quit.
- [x] Dispatcher: queue, ~8 ms budget per tick, `batch` within one tick.
      Tested over `FakeSession` with a real socket.
- [x] Jobs: queued, running, done, failed, cancelled; `job.wait` with
      timeout; results kept for 10 minutes. Tested with a fake long job.
- [x] `bridge.hello` returns protocol version and build ids. A different
      major version is refused with both versions named.
- [x] Smoke: `scripts/smoke.sh` exists and checks hello against REAPER.

## Close-out (2026-11-18)

- `mort-bridge`:
  - `server`: an accept thread, plus a reader and a writer thread per
    connection. Lines that are not valid messages are answered by the
    reader. On start it removes sockets left by dead instances. The
    directory is mode 0700 and the socket 0600. `current` is written
    atomically. Drop removes both.
  - `dispatch`: `Dispatcher<S: Session>`.
    - Methods are registered with an `edits` flag. Edits are refused while
      held (`busy`), checked against `expect_state` (`stale`), and wrapped
      in an undo point labelled `mort: <summary>`. A failed edit, or a
      failed `batch`, is rolled back with an undo.
    - One job runs at a time, and the rest queue. `job.wait` is answered
      on a later tick rather than by blocking.
    - `project.changed` reports `by_mort`.
    - `bridge.hello` refuses a different major version and names both.
  - `reaper`: entry through `reaper_macros::reaper_extension_plugin`, a
    timer at UI rate with an 8 ms budget, and `catch_unwind` around each
    tick. `Live` implements project info, the state count, the tempo map
    (`TimeMap` calls), undo and stop. Everything else returns a `reaper`
    error naming the ticket that adds it.
- reaper-rs is pinned to git rev `bd7cbb0` (the crates.io 0.1.0 is from
  2020). `.cargo/config.toml` sets `git-fetch-with-cli`, because libgit2
  stalls on the WDL submodule.
- Smoke test in REAPER 7.67 on macOS: hello, version refusal and status
  all `ok`. The bridge loads from `UserPlugins/reaper_mort.dylib`, and on
  quit it removes the socket and `current`.
- Deviations:
  - The bridge logs to `$TMPDIR/mort/bridge.log`, not the REAPER console,
    because opening the console on every start is intrusive. The console
    is used only when startup fails.
  - REAPER does not run timers while a modal dialog is open (the first
    smoke run, during startup, timed out). Clients see this as a slow
    reply. The CLI's error for a timeout should say so (ticket 0004).
- 7 dispatcher tests run through a real socket over `FakeSession`.
