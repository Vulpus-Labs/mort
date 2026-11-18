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

- [ ] `mort status` prints the bridge version, project name, state count
      and whether the wheel is held.
- [ ] `mort call <method> <json>` sends any RPC.
- [ ] Text output by default, `--json` raw. Exit codes 0, 1, 2 and 3 as in
      ADR 0009 §6. Tested against the fake bridge.
- [ ] `mort install` copies `reaper_mort.dylib` to `UserPlugins` and the
      probe bundle to `~/Library/Audio/Plug-Ins/CLAP`. If REAPER is running
      it says that a restart is needed.
