---
id: "0038"
title: "ReaEQ band on/off and type are out of reach"
priority: medium
created: 2026-11-21
epic: E004
depends: []
---

## Summary

A ReaEQ added by mort has a fifth band, "High Pass 5", that is switched
off. Whether a band is on, and its type, are not parameters, so mort
cannot use a high-pass. Measured on the ambient reverb return: with the
band at 2 kHz, 1 kHz did not move. The mix used ReaEQ's low shelf at
−12 dB as a low cut instead.

## Acceptance criteria

- [ ] mort can turn a ReaEQ band on or off and set its type, through
      `fx.set_state` with a checked edit of ReaEQ's state, or through
      `TrackFX_SetNamedConfigParm` if REAPER exposes it (check
      `BANDENABLED`/`BANDTYPE`-style names).
- [ ] `mort fx`/`params` show each band's type and whether it is on.
- [ ] Smoke check: enable the high-pass at 300 Hz and measure the band
      below it fall.
