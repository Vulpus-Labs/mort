---
id: "0025"
title: "Voicing vocabularies as data, and candidate generation"
priority: high
created: 2026-11-20
epic: E004
depends: ["0024"]
---

## Summary

Define voicing vocabularies in KDL, and generate each harmony's
candidate voicings within a part's range and voice count (ADR 0013 §5).

## Acceptance criteria

- [ ] A KDL format for vocabularies: built from chord tones by role,
      from stacked intervals on scale degrees, or as fixed shapes, with
      filters (only available notes, no avoid notes, required roles).
- [ ] Built-in vocabularies: close, drop-2, drop-3, shells, spread,
      quartal, "So What", upper-structure triads, clusters.
- [ ] User (`~/Library/Application Support/mort/voicings/`) and project
      (`<project>.mort/voicings/`) vocabularies are loaded too, and
      override built-ins by name. Errors name the file and the line.
- [ ] Candidates for a harmony, a range and a voice count, every note
      labelled with its role.
- [ ] Tests: the quartal candidates of D Dorian, and of Gm9 under the
      default scale; drop-2 of Cmaj7 in every inversion; a user
      vocabulary overriding a built-in; refusing a vocabulary that can
      build nothing for a harmony.
