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

- [ ] `play(range)` plays and stops at the end. The job finishes when
      playback stops. Cancel stops playback.
- [ ] `render(range, tracks?)` renders faster than real time. Find out
      whether REAPER's API allows a dry run. If it does not, render to
      `$TMPDIR/mort/renders` and delete afterwards. Record which in the
      close-out and in ADR 0006 §4.
- [ ] Run ids are unique, and the bridge publishes the current run id where
      probes can read it without locking.
- [ ] CLI `mort play 9.1..17.1`, `mort render 9.1..17.1`.
