---
id: "0028"
title: "mort compose show and write: items, provenance, staleness, hand edits"
priority: high
created: 2026-11-20
epic: E004
depends: ["0027"]
---

## Summary

Bring the layers to REAPER through the CLI (ADR 0013 §9, §10).

## Acceptance criteria

- [ ] `mort compose show <section> [--blocks | --notes] [--part P]`,
      read-only, compact enough for an agent.
- [ ] `mort compose write [--part P] [--section S] [--force]` writes one
      MIDI item per part per section, covering the section, through
      `midi.write`, in one undo point.
- [ ] Sections map to regions by name. A section with no region, or a
      region whose length disagrees with the sheet, is an error that
      names both.
- [ ] Provenance in `<project>.mort/compose/`: a hash of each layer's
      inputs, the seed, and a hash of the notes written. Only stale
      layers regenerate.
- [ ] An item whose notes differ from the recorded hash is reported as
      edited by hand and left alone unless `--force` is given.
- [ ] An unsaved project is refused, with the fix.
- [ ] Tests over the fake: write, change a chord (only the dependent
      items change), change an articulation (only that part), edit a
      note by hand (protected), `--force`.
