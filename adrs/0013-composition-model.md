# ADR 0013: The composition model

- **Status:** Proposed
- **Date:** 2026-11-20

## Context

We want to ask for guide parts, comping and drums that follow a chord
sequence, in a named style, and to change one aspect of them (a chord, a
rhythm, a voicing style) without rewriting the rest. mort can already
write MIDI notes given one by one (ticket 0008). An LLM can choose
musical material well: which voicing style, which groove, where the
exceptions go. Over 64 bars it is poor at the details: smooth
voice-leading, staying in range, writing the same output twice from the
same request.

Harmony and articulation are separate concerns. "Quartal voicings in the
verse" is a decision about which notes sound together. "Bass in octaves
in the left hand, chord fragments in a bossa nova rhythm" is a decision
about how those notes are played. Either should be changeable without
touching the other. Quartal writing also shows that a chord symbol is not
enough on its own. A stack of fourths often has no single symbol, and
modal writing works from a scale held over several bars.

## Decision

1. **A REAPER-free crate, `mort-compose`,** turns a song sheet into MIDI
   events. It runs in the CLI process and writes through `midi.write`,
   so the bridge gains nothing new. It is tested without REAPER (ADR
   0011). Its output is deterministic: the same inputs give the same
   notes. The only randomness is in performance (§8), and it is seeded.

2. **Four layers.** Each one reads the layer above it and nothing else:
   - **Harmony:** what may sound at each moment.
   - **Voicing:** which notes sound, as a stream of *blocks*.
   - **Articulation:** when and how the block notes are played.
   - **Performance:** timing and velocity feel.

   The stream of blocks between voicing and articulation is the central
   data structure. It can be shown and discussed on its own (§9).

3. **The song sheet** is a KDL file, `<project>.mort/sheet.kdl`, beside
   the plan of ADR 0008, which uses the same format. Sections are named
   by their regions, as in plans.
   - It holds the form (sections and their lengths in bars), the
     harmony, and the parts.
   - A part names its target track, its harmony source, its voicing
     spec, its articulation and its performance, each of which can be
     set per section.
   - The sheet is never written behind the person's back. Edits made by
     hand are read when the file changes, as for plans (ADR 0008 §4).

   An illustrative form (the syntax is settled in the first ticket):

   ```kdl
   section "verse" bars=16 {
       harmony "D dorian" bars=8
       harmony "Eb lydian" bars=8
   }
   section "chorus" bars=8 {
       harmony "Gm9" "C13" "Fmaj9" "Bbmaj7#11" bars=2
   }
   part "Pad"  track="Pad"  voicing="quartal" voices=4 range="C3..A4"
   part "Keys" track="Keys" voicing="drop2" range="G2..D5" {
       articulation "bossa-piano"
   }
   part "Bass" track="Bass" from="Keys" voice="bass" {
       articulation "walking"
   }
   ```

4. **Harmony.** At each moment the harmony gives a root, a scale or
   mode, the chord tones, the available tensions and the avoid notes.
   - A chord symbol implies a default scale (`m7` → Dorian, `maj7#11`
     → Lydian). Each chord family (major, dominant, minor, and so on)
     has a list of scales in order of preference, each with its reason.
     The chord takes the first scale that holds every note its symbol
     asks for, so the alterations choose: `G7` is Mixolydian, `G7b9`
     half-whole diminished, `G7b9b13` Phrygian dominant. The sheet can
     name a scale instead (`scale="aeolian"`, or a root and scale). Chord
     tones outside a named scale are allowed and stay chord tones.
   - A bare scale or mode ("D dorian for 8 bars") is a harmony in its
     own right, with no chord symbol. Its chord tones are its own 1st,
     3rd, 5th and 7th notes. Scales with other than seven notes take the
     nearest notes they have.
   - Avoid notes are scale notes a semitone above a chord tone. The
     other scale notes are the available tensions.
   - The chord-symbol grammar and the default-scale lists are data in
     the crate, with tests (ticket 0024).

5. **Voicing vocabularies are data.** A vocabulary is a rule for
   building candidate note stacks from a harmony. Examples:
   - close, drop-2, drop-3, shells, spread;
   - quartal (perfect fourths on scale degrees, keeping only stacks whose
     notes are all available), "So What" (three fourths and a major
     third);
   - upper-structure triads, clusters.

   Vocabularies are KDL definitions. Built-in ones ship in the crate.
   The user's (`~/Library/Application Support/mort/voicings/`) and the
   project's (`<project>.mort/voicings/`) are added to them and override
   built-ins of the same name. Adding a vocabulary needs no new release.

   A vocabulary builds in one of four ways (ticket 0025; the format is
   documented at the top of `mort-compose/voicings/builtin.kdl`):
   - `chord`: chord tones by role in order of preference, doubled if
     too few, arranged close, drop-2, drop-3 or drop-2-and-4, with a
     role alone in the bass if asked. Notes the symbol names are
     picked before the scale's tensions, so `Dm9` gives F A C E.
   - `step`: a stack in which each step is one of the given intervals
     (quartal is `step 5`, clusters `step 1 2`).
   - `steps`: fixed stacks, as alternatives ("So What" is `steps 5 5 5
     4`).
   - `lower` and `upper`: two layers, the upper wholly above the lower
     (upper-structure triads).

   `voices` limits the voice counts, `from` the bottom note's role, and
   `require` keeps only voicings holding given roles. Every note must
   be available in the harmony (a chord tone or a tension, never an
   avoid note). The built-ins also include `rootless`, for comping over
   a separate bass part.

6. **Voice-leading** chooses one candidate per harmony change, so that
   the whole sequence costs least. The search is Viterbi over the
   candidates.
   - **Hard constraints:** each voice's range, the voice count, a melody
     note required in the top voice, and literal voicings given in the
     sheet for particular bars.
   - **Soft costs, with weights in the part's spec:** total movement,
     leaps per voice, common tones kept, low-interval spacing (no thirds
     below about E2), parallel fifths and octaves, and voice crossing.
   - Ties are broken in a fixed order, so the result is deterministic.
   - When no candidate meets the hard constraints, the error names the
     section, bar and constraint, and suggests what to relax.

7. **Blocks** are sustained sonorities with a start and a length. Every
   note in a block carries two labels:
   - **its voice:** `bass`, `inner1`…`innerN`, `top`;
   - **its role:** `root`, `3rd` (or `2nd`/`4th` in sus chords), `5th`,
     `6th` or `7th`, a tension by degree (`9th`, `11th`, `13th`), or a
     non-chord tone (`passing`, `neighbour`, `suspension`,
     `anticipation`, `pedal`).

   Voice-leading may add non-chord tones between blocks: a passing tone
   filling a third, a suspension held over a change. This is switched on
   per part. Such tones are labelled, never left out, and articulation
   decides whether to sound them (§8).

8. **Articulation and performance patterns are data too:**
   - A pattern is a grid of *hits*, one or more bars long, anchored to
     the bar and not to the chord. When the chord changes in the middle
     of a pattern, the next hit takes the new block's notes, and the
     rhythm carries on.
   - A hit selects notes by label: voice (`bass`, `top`, `inner`), role
     (`3rd 7th`), or both. It never selects by pitch, so a pattern works
     under any harmony.
   - Each hit gives its position, length, accent, octave doubling or
     shift, and which hand or instrument plays it.
   - A hit can **anticipate**: it takes the next block's harmony instead
     of the current one, like the push on the "and" of 4 in bossa nova
     and pop.
   - Each pattern says how it treats non-chord tones. A block or strum
     pattern skips them; a line or arpeggio pattern uses them.
   - **Playability** is checked: a hand's span (by default a tenth), and
     each instrument's range. Hits that cannot be played are reported
     with their bar and beat. They are never silently moved.
   - **Drums** use the same machinery with no harmony. Hits name kit
     pieces (`kick`, `snare`, `hat_closed`), and a kit map turns them
     into note numbers for the drum instrument on the track.
   - **Performance** applies swing, timing deviations and velocity
     curves last, seeded per part and section.

9. **Inspection.** `mort compose show <section> --blocks` lists the
   voiced blocks with their labels. `--notes` lists the notes the
   articulation would write. Both are read-only, so harmony can be
   agreed before any rhythm exists, and a change can be previewed
   before it is written.

10. **Ownership and provenance.**
    - Each part writes one MIDI item per section on its track, covering
      the section exactly. mort owns those items.
    - For each item it records, in `<project>.mort/compose/`:
      - a hash of the inputs to each layer;
      - the seed;
      - a hash of the notes it wrote.
    - Changing a layer marks that layer and everything below it as
      stale, in the same way as the mix map (ADR 0006 §3):
      - a new articulation regenerates only the notes;
      - a new chord regenerates blocks and notes.
    - `mort compose write` regenerates stale items, one undo point per
      run (ADR 0004).
    - An item whose notes no longer match the recorded hash was edited
      by hand. mort will not overwrite it without `--force`, and says
      which items those are.
    - Composing needs a saved project, because the provenance lives in
      the sidecar. On an unsaved project the error says to save it first.

## Consequences

- An LLM states intent compactly: vocabularies, patterns, ranges and
  exceptions. mort guarantees consistency over the whole piece. The
  sheet is a readable record of musical decisions, as the plan is of
  mix decisions.
- Several parts can share one voiced harmony: a pad sustains the blocks,
  keys articulate them, the bass walks the bass voice. They agree
  harmonically by construction.
- Reharmonising, re-voicing and re-grooving are separate edits with
  separate, predictable effects.
- The vocabularies, patterns and kit maps are new data to curate. The
  built-in sets should be small and well tested. Breadth comes from user
  and project definitions.
- Not covered here:
  - melody writing (a melody is an input that constrains `top`, not an
    output);
  - audio parts;
  - analysing existing MIDI back into blocks (useful for reharmonising a
    part played by hand, but a separate decision).
- Settled in ticket 0024: the chord-symbol grammar, and the sheet's
  syntax for form and harmony (`meter`, `section`, `harmony`). Still
  open: the sheet's syntax for parts, the default cost weights (to be
  set by listening to the output, in the spirit of ADR 0007 §3), and
  whether the walking-bass line belongs in voicing (as a line generator
  with approach notes) or in articulation.
