---
id: "0013"
title: "Findings and queries; calibration on Fenland material"
priority: high
created: 2026-11-18
epic: E001
depends: ["0012", "0008"]
---

## Summary

Write the queries and finding kinds of ADR 0007 §2–4, then calibrate
thresholds on Fenland, where we know what needed fixing.

## Acceptance criteria

- [ ] `summary`, `track` and `frames` queries, with CLI output short
      enough for an agent (a section summary of 20 tracks fits in about 60
      lines).
- [ ] Finding kinds `masking`, `mud`, `harsh`, `headroom`, `loudness_jump`,
      `low_end_width` and `register_clash`, each with evidence and a
      suggestion.
- [ ] Calibration: render Fenland's stems (from the song render in the
      silkscreen scratch pipeline or the videbimus fenland crate), measure
      them, and check findings against known problems and fixes. Put the
      thresholds in one table with reasons. Record the results in the
      close-out.
- [ ] Each finding kind has a fake-session test that plants the problem
      and expects the finding, and a clean case that does not produce
      it.
