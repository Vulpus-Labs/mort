---
id: "0022"
title: "End to end in REAPER: the E003 proof"
priority: medium
created: 2026-11-19
epic: E003
depends: ["0021"]
---

## Summary

Run E003's proof in REAPER, and settle the questions ADR 0012 leaves to
measurement.

## Acceptance criteria

- [ ] The four steps in E003's goal pass, recorded in `docs/e003-proof.md`
      with screenshots of the page.
- [ ] Measured and recorded:
      - whether REAPER resets plugins on seek (affects the warm-up rule);
      - whether dragging an FX knob moves the state count during the drag
        or only on release;
      - whether non-soloed tracks still run their FX.
      ADR 0012 updated with the answers and its status set to Accepted.
- [ ] `scripts/smoke.sh` gains audition checks: a manual play adds an
      audition capture, and a solo play does not touch the bus.
