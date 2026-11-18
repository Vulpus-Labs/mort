---
id: "0007"
title: "Automation: read and write envelopes"
priority: high
created: 2026-11-18
epic: E001
depends: ["0006"]
---

## Summary

Read and write track and FX-parameter envelopes, in musical positions,
as one undo point per write.

## Acceptance criteria

- [ ] `envelope.get` for volume, pan, sends and any FX parameter, creating
      the envelope on write if it is missing.
- [ ] Write points with shape (linear, square, slow, bezier with tension).
      Write a ramp or a hold over a range. Clear a range.
- [ ] CLI `mort automate <track> <target> 9.1=0.2 13.1=0.8 --shape slow`
      and `--clear 9.1..17.1`.
- [ ] Round-trip tests over the fake. Smoke in REAPER shows the envelope as
      expected.
