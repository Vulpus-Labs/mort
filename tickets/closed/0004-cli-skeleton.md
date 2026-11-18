---
id: "0004"
title: "CLI skeleton: finding the socket, status, call, output modes, exit codes, install"
priority: high
created: 2026-11-18
epic: E001
depends: ["0003"]
---

## Summary

Write the `mort` binary of ADR 0009, with the scaffolding every later
command uses: connection, name resolution hooks, text and JSON output,
waiting on jobs, exit codes. Add `mort install`, which copies the
extension and the probe into REAPER's folders and says when a restart is
needed.

## Acceptance criteria

- [x] `mort status` prints the bridge version, project name, state count
      and whether the wheel is held.
- [x] `mort call <method> <json>` sends any RPC.
- [x] Text output by default, `--json` raw. Exit codes 0, 1, 2 and 3 as in
      ADR 0009 §6. Tested against the fake bridge.
- [x] `mort install` copies `reaper_mort.dylib` to `UserPlugins` and the
      probe bundle to `~/Library/Audio/Plug-Ins/CLAP`. If REAPER is running
      it says that a restart is needed.

## Close-out (2026-11-18)

- `mort-cli`, built with clap:
  - `client`: finds the socket through `$TMPDIR/mort/current` or
    `--socket`, and checks the protocol version with `bridge.hello` on
    connect. It skips notifications while waiting for a reply. After 15 s
    with no reply it exits 3 and says that REAPER runs mort only when idle,
    so close any open dialog (see 0003's close-out).
  - `mort status`: bridge build, project, held, playing, jobs, and the
    state on the last line.
  - `mort call <method> [json]`: pretty-printed result, or one line with
    `--json`.
  - Exit codes 0, 1, 2 and 3 as in ADR 0009 §6. With `--json`, a bridge
    error is also printed as `{"error": …}` on stdout.
  - `mort install`: copies the bridge to `UserPlugins/reaper_mort.dylib`
    and `mort-probe.clap` to the CLAP folder, from the target directory
    `mort` was built in (or `--from`). Copies go to a temporary name and
    are renamed into place, so a running REAPER keeps its loaded copy. If
    the probe has not been built it is skipped and the output says so
    (ticket 0011 builds it). If REAPER is running, it says to restart.
- 7 tests run the real binary against a fake bridge on a thread, plus
  install into temporary directories.
- A real `mort install` on this machine installed the bridge. `mort status`
  with REAPER closed exits 3 and says why.
