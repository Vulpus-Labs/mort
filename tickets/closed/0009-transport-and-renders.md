---
id: "0009"
title: "Transport and renders as jobs; run ids"
priority: high
created: 2026-11-18
epic: E001
depends: ["0005"]
---

## Summary

Playing and rendering a range, as jobs (ADR 0002 §3), each with a run id
that is handed to probes before it starts (ADR 0005 §3).

## Acceptance criteria

- [x] `play(range)` plays and stops at the end. The job finishes when
      playback stops. Cancel stops playback.
- [x] `render(range, tracks?)` renders faster than real time. Find out
      whether REAPER's API allows a dry run. If it does not, render to
      `$TMPDIR/mort/renders` and delete afterwards. Record which in the
      close-out and in ADR 0006 §4.
- [x] Run ids are unique, and the bridge publishes the current run id where
      probes can read it without locking.
- [x] CLI `mort play 9.1..17.1`, `mort render 9.1..17.1`.

## Close-out (2026-11-19)

- Dry run: REAPER 7.67 has action 43349, *File: Dry run render project,
  using the most recent render settings*. `actions.find` found it.
  `Live::render`:
  1. saves `RENDER_BOUNDSFLAG`, `RENDER_STARTPOS`, `RENDER_ENDPOS`,
     `RENDER_SETTINGS`, `RENDER_TAILFLAG` and `RENDER_ADDTOPROJ`;
  2. sets custom bounds, master mix, no tail;
  3. runs the action, which renders inside the call;
  4. restores the six settings.

  No file is written. ADR 0006 §4 is updated.
- `Live::play` moves the edit cursor to the start and presses play.
  `position` reads `GetPlayPosition2Ex`, the position being heard.
- Jobs: `play`, which stops at the end and reports `stopped_early` if
  someone else stops it; `render`, which reports seconds taken and speed;
  and `stop`.
- Run ids (`runs.rs`): one atomic per bridge. `begin` before each play or
  render, `end` after, and `current` for probes to read lock-free in
  0011. `bridge.status` shows the current run.
- `actions.find {text}`: searches REAPER's main action list. Read-only.
- CLI: `mort play RANGE`, `mort render RANGE`, `mort stop`, and
  `--no-wait`. Waiting uses `job.wait` with progress on stderr. A render
  holds REAPER's main thread for its length, so replies while waiting on
  a job may take up to 10 minutes; other calls keep the 15 s limit.
- Smoke, 48 checks, all `ok` in REAPER 7.67:
  - an 8 s dry-run render took 3.1 s, and the bridge answered at once
    afterwards, so no dialog was left open;
  - `play 1..2 s` stopped at 2.06 s, one timer tick late;
  - `stop` ended a long play early.
- Open:
  - 3× real time is slow for an empty project. Render speed with real
    instruments is measured in 0012.
  - That smoke run took several minutes longer overall than its checks
    account for, and the cause is not known yet. Watch for it in the
    next runs.
  - `current` is left behind when REAPER is killed (SIGTERM skips the
    destroy hook). The bridge cleans up dead sockets at its next start,
    but until then the CLI reports that no bridge is running and names
    the file.
- Tests: CLI 24, including render, play to the end, stop from outside,
  and a reversed range. `runs` unit test.
