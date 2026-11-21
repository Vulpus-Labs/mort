---
id: "0037"
title: "Setting a dB parameter by text lands twice as far as asked (ReaEQ gain)"
priority: high
created: 2026-11-21
epic: E004
depends: []
---

## Summary

`mort set Pad ReaEQ "Gain-Band 2" -6` set −12.1 dB. −3 gave −9.0 and
−4 gave −10.0. ReaEQ's gain is amplitude: normalised `v` shows as
`20·log10(2v)` dB. The values set match `v = 0.5·10^(dB/10)`, as if the
text were read as power. Frequencies, bandwidths and ReaComp's threshold,
ratio and times set exactly. Found mixing the ambient piece. Its script
sets ReaEQ gains by normalised value instead.

## Acceptance criteria

- [ ] Find where text becomes a value for this parameter
      (`param.parse`, or mort's own search) and why it doubles the dB.
- [ ] `set … ReaEQ "Gain-Band 2" -6` reads back as −6.0. A smoke check
      covers ReaEQ gain, a ReaComp threshold and a CLAP dB parameter.
