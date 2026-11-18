---
id: E002
title: "Mix plans and instrument manifests"
status: deferred
created: 2026-11-18
---

## Goal

Give mort the intended shape of a piece (ADR 0008) and the meaning of
instrument parameters (ADR 0010). With these, an agent can state an arc
for a piece, set instruments in real units, and have findings report how
far each section is from the plan.

## Outline

- `plan.kdl`: parse, validate against the project's regions, watch for
  edits made by hand; `plan.get` and `plan.set`.
- `off_plan` findings, and plan-aware `loudness_jump`.
- Manifest loading from the bundle and user directories; manifest data in
  `params`; `set` in real units.
- In silkscreen: generate `mort-manifest.json` from typed parameter
  definitions (that ticket lives in silkscreen).

Tickets are written when E001 closes.
