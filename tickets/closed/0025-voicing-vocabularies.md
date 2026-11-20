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

- [x] A KDL format for vocabularies: built from chord tones by role,
      from stacked intervals on scale degrees, or as fixed shapes, with
      filters (only available notes, no avoid notes, required roles).
- [x] Built-in vocabularies: close, drop-2, drop-3, shells, spread,
      quartal, "So What", upper-structure triads, clusters.
- [x] User (`~/Library/Application Support/mort/voicings/`) and project
      (`<project>.mort/voicings/`) vocabularies are loaded too, and
      override built-ins by name. Errors name the file and the line.
- [x] Candidates for a harmony, a range and a voice count, every note
      labelled with its role.
- [x] Tests: the quartal candidates of D Dorian, and of Gm9 under the
      default scale; drop-2 of Cmaj7 in every inversion; a user
      vocabulary overriding a built-in; refusing a vocabulary that can
      build nothing for a harmony.

## Close-out (2026-11-20)

- **Format.** `mort-compose/voicings/builtin.kdl` documents the format
  at its top and holds the built-ins. A `voicing` builds in one of four
  ways:
  - `chord`: roles in order of preference, with `double`, `layout`
    (close, drop2, drop3 or drop24) and `bass ROLE gap=N`;
  - `step`: each step any of the given intervals;
  - `steps`: fixed stacks, as alternatives;
  - `lower`/`upper`: two layers.

  Any of them takes `voices`, `from` and `require ROLE… [count=N]`. ADR
  0013 §5 is amended to summarise this.
- **Built-ins (10).** close, drop-2, drop-3, shell, rootless, spread,
  quartal, so what, upper structure, cluster. `rootless` was not in the
  ticket's list. It covers comping over a separate bass part, the usual
  case for keys over a bass line.
- **Candidates.** `Vocabulary::candidates(harmony, range, voices)` gives
  every voicing, with each note labelled by role, sorted from the
  lowest. It refuses with a reason when the voice count is not allowed,
  or when nothing fits the harmony or the range.
- **Picking in two passes.** A `chord` rule picks first from the notes
  the symbol names, then from the scale's tensions. Without this,
  rootless `Dm9` took the scale's 13th (B) ahead of the symbol's 5th,
  giving F B C E instead of F A C E. With it, `G13` gives B E F A. Both
  are tested.
- **Loading.**
  - `Vocabularies::standard(sidecar)` loads the built-ins, then
    `user_dir()`, then `<sidecar>/voicings`. Later ones replace earlier
    ones by name.
  - Each directory's `.kdl` files are read in name order. A missing
    directory loads nothing.
  - Errors give the file, the line and the column.
- **Refactor.** The KDL helpers for line and column, arguments and
  properties moved from `sheet` into a shared `source` module.
- **Tests.**
  - 2 unit tests: the built-ins read, and ranges.
  - 10 integration tests:
    - quartal on D Dorian (bottoms D E A B), and on Gm9 under its
      default scale;
    - drop-2 of Cmaj7 in all four inversions;
    - shells;
    - So What, and an A-major upper structure over G7#11;
    - rootless picking order;
    - every built-in builds only available notes;
    - a user file replacing `quartal`;
    - refusals;
    - file errors with line and column.

  All four project checks pass.
