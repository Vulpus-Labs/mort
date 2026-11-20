---
id: "0036"
title: "measure under solo: say so, and keep the master's data"
priority: high
created: 2026-11-20
epic: E004
depends: []
---

## Summary

With a track soloed in REAPER (the person listening to the ambient pad),
`mort measure` rendered with only the soloed track processed. The other
probes sent nothing and were reported as "its probe sent no frames".
The master's capture, made of the pad alone, replaced good data, and
`summary` showed section b 4.6 dB quieter than it is. Auditions already
handle solo (ADR 0012 §2). Measured renders do not.

## Acceptance criteria

- [ ] `measure` sees the solo set before rendering. It either refuses,
      naming the soloed tracks and saying to unsolo (or to pass
      `--solo-ok` to measure just those tracks), or measures only the
      soloed tracks and keeps the master's and buses' old data, marked
      as such. Decide which, and record it in ADR 0006 §4.
- [ ] "Its probe sent no frames" says why when the cause is known:
      muted, or silenced by a solo.
- [ ] Fake test, plus a smoke check with a soloed track.
