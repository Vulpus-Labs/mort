# ADR 0008: Mix plans: the intended shape of the piece

- **Status:** Accepted (implemented in E002)
- **Date:** 2026-11-18

## Context

Findings on their own judge each moment against fixed norms. Large-scale
decisions ("the bridge drops to half density and opens up in width, the
last chorus is the loudest point") need a statement of intent that the map
can be checked against. That intent should outlive one agent session and
be readable and editable by the person.

## Decision

1. **A plan is a KDL file**, `<project>.mort/plan.kdl`, beside the project.
   KDL matches videbimus's config and is pleasant to edit by hand.
2. **The plan is organised by section** (region name), with optional
   targets:
   - loudness, short-term LUFS, as a value or a range;
   - density, as transient density or number of active elements;
   - width;
   - spectral tilt;
   - which tracks or groups are active, featured or absent;
   - the kind of transition into the section: cut, build or fade.
3. **Findings gain `off_plan`:** a section's measured value outside the
   plan's target, with the size and direction of the miss.
   `loudness_jump` is suppressed at a boundary the plan marks as a cut.
4. **The plan is never written behind the person's back.** `plan.set`
   writes the file and logs to the bridge log. The file is the source of
   truth, and edits made by hand are picked up when the file changes.

## Consequences

- An agent can state the arc first and then work toward it, with the map
  saying how far off each section is.
- The plan stays a readable record of intent alongside the project.
