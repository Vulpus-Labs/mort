---
id: "0039"
title: "Render a range to an audio file"
priority: medium
created: 2026-11-21
epic: E004
depends: []
---

## Summary

`mort render` is a dry run by design (ADR 0006 §4), so A/B files for the
ambient mix needed a Lua script in REAPER
(`docs/render-wav.lua.tmpl`). The script sets the `RENDER_*` project
settings, runs action 42230, and puts the settings back. An agent
comparing mixes, or handing the person files to listen to, needs this
in mort.

## Acceptance criteria

- [ ] `mort render RANGE --to FILE [--tail SECONDS] [--format wav24]`
      writes the master mix, as a job, with the project's render
      settings restored afterwards. Stems per track are out of scope.
- [ ] The result says when the file is complete (the file appears
      while it is still being written).
- [ ] Optional `--match CAPTURE`: gain on the render so its loudness
      matches an earlier capture, for fair A/B.
- [ ] ADR 0006 §4 or ADR 0009 says how a file render differs from a
      measuring render.
