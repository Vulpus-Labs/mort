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

- [ ] REAPER (macOS, current release) loads the extension. It logs its
      version to the REAPER console and creates the socket and `current`
      file, mode 0600. They are removed on quit.
- [ ] Dispatcher: queue, ~8 ms budget per tick, `batch` within one tick.
      Tested over `FakeSession` with a real socket.
- [ ] Jobs: queued, running, done, failed, cancelled; `job.wait` with
      timeout; results kept for 10 minutes. Tested with a fake long job.
- [ ] `bridge.hello` returns protocol version and build ids. A different
      major version is refused with both versions named.
- [ ] Smoke: `scripts/smoke.sh` exists and checks hello against REAPER.
