---
id: "0035"
title: "Mix levels ignore volume automation"
priority: high
created: 2026-11-20
epic: E004
depends: []
---

## Summary

`summary`'s per-track `level` and `share` come from the pre-fader probe
plus the fader's gain read once. With a volume envelope, REAPER's fader
follows the envelope, so mort applies whatever the envelope says at the
play cursor to the whole range. In the ambient piece, measured with the
cursor at the end, where envelopes sit at −60 dB, the drone, pulse and
hiss read 10 to 20 dB lower than they sound. The master's own levels are
right. They come from its probe.

## Acceptance criteria

- [ ] Track contributions use the volume envelope's value at each
      frame's time (and pan and send envelopes likewise), falling back
      to the fader where there is no envelope.
- [ ] ADR 0005's consequence on pre-fader placement and ADR 0007's
      `track` query say so.
- [ ] Fake test: a track with a fade-out envelope reads its level from
      the envelope, not from the fader at the end.
- [ ] Smoke: the ambient case, where track shares sum to about the
      master's energy.
