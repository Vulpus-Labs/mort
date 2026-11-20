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

- [x] `mort compose show <section> [--blocks | --notes] [--part P]`,
      read-only, compact enough for an agent.
- [x] `mort compose write [--part P] [--section S] [--force]` writes one
      MIDI item per part per section, covering the section, through
      `midi.write`, in one undo point.
- [x] Sections map to regions by name. A section with no region, or a
      region whose length disagrees with the sheet, is an error that
      names both.
- [x] Provenance in `<project>.mort/compose/`: a hash of each layer's
      inputs, the seed, and a hash of the notes written. Only stale
      layers regenerate.
- [x] An item whose notes differ from the recorded hash is reported as
      edited by hand and left alone unless `--force` is given.
- [x] An unsaved project is refused, with the fix.
- [x] Tests over the fake: write, change a chord (only the dependent
      items change), change an articulation (only that part), edit a
      note by hand (protected), `--force`.

## Close-out (2026-11-20)

- **CLI** (`crates/mort-cli/src/compose.rs`).
  - `mort compose show SECTION [--part P] [--notes] [--sheet PATH]` lists
    the voiced blocks: harmony and scale, each note spelled with its
    role, and non-chord tones with their voice and span. With
    `--notes` it lists the notes each part would write, by bar and beat,
    with playability problems. Parts that play another's voices, and
    drum parts, are named as such.
  - `mort compose write [--part P] [--section S] [--force]` reads the
    sheet beside the project, checks each section against its region,
    plays every part, decides per part and section, converts the notes
    through `time.convert` (region start in seconds plus quarter notes,
    so tempo changes count), and writes them in one `batch` of
    `midi.write … mode=replace`, one undo point.
- **Record** (`mort_compose::record`, without REAPER, unit-tested). It
  lives in `<project>.mort/compose/state.json` and holds per part and
  section: the item, the hash of the generated notes, the hash of the
  notes read back after writing (rounded to 0.1 ms), and the layers'
  input hashes. `decide` gives new, unchanged, edited by hand, or
  changed with the layers that changed.
- **Deviation: output decides.** Rewrites are decided by the generated
  notes, not by the input hashes, because voice-leading crosses section
  boundaries. When only a neighbour moved the voicing, the reason says
  so. ADR 0013 §10 is amended.
- **Refusals before writing:**
  - MIDI in a section that mort did not write;
  - two parts on one track in one section;
  - a missing region, or one whose length disagrees with the sheet's
    bars;
  - a missing track;
  - an unsaved project.
- **Spelling.** `Harmony` keeps the root as written, and
  `Harmony::spell` spells by role, then by scale degree, then plainly
  (A7alt: C♯, B♭, B♯, D♯, F; E♭ lydian: B♭). ADR 0013 §9 is amended.
- **Merged:** the CLI's `parse_pitch` and `pitch_name` now use
  `mort_compose::pitch`, as 0024's close-out planned.
- **Drums** write on channel 10.
- **Tests.**
  - mort-compose: 3 record tests (decisions, stable hashes, round trip)
    and a spelling test.
  - CLI over the fake: `compose_show_and_write` covers show, the first
    write of 8 items, a no-op rewrite, a new articulation (only Keys,
    while Bass, which plays Keys' bass voice, stays unchanged), a new
    chord (the chorus parts), a hand edit left alone and then
    overwritten with `--force`, and undo of one write. The test
    `compose_refuses_what_it_should_not_touch` covers foreign MIDI with
    nothing written, a wrong region length, and an unsaved project.

  All four project checks pass.
- **Seen in the output, for 0029.** The default weights hold a quartal
  pad on E A D G across Gm9 C13 Fmaj9 Bbmaj7#11. This is noted in
  0029's ticket to judge by ear.
- **Not checked in REAPER:** that is 0029.
