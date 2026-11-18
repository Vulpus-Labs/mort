# ADR 0003: Addressing elements, and positions in time

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Track and FX indices shift whenever anything is inserted, moved or deleted,
by the user or by mort. An agent that holds an index across two calls
will edit the wrong thing. Musicians think in bars and beats. Measurements
are made in seconds.

## Decision

1. **Ids are REAPER GUIDs.** Tracks, FX instances, items, takes and
   envelopes are named by GUID in the protocol. `mort-proto` wraps each in
   its own type (`TrackId`, `FxId`, …), so they cannot be mixed up.
2. **Names are accepted where unambiguous.** The CLI resolves `--track bass`
   to a GUID before calling. If a name matches more than one element, the
   call fails and lists the matches. The bridge itself takes only GUIDs.
3. **Parameters** are addressed by FX id plus index. Every parameter
   description also carries its name, so a client can check it has the
   right one. Values pass as normalised 0–1. The bridge also returns
   REAPER's formatted display text (`TrackFX_FormatParamValueNormalized`),
   and can set a value from display text where the plugin supports it.
4. **Positions** in the protocol are project seconds (`f64`). The CLI and
   `mort-proto` parse and print musical positions:
   - `9` or `9.1` is bar 9, beat 1. Beats are in units of the bar's
     time-signature denominator, so beat 4 of a 6/8 bar is the fourth
     eighth note.
   - `9.2.240` adds ticks at 960 per quarter note
   - `+4b` is four quarter notes after the previous position, and `+1.5s`
     is 1.5 seconds after it
   - `12.5s` is seconds

   A range is `start..end`, and a relative end is measured from the start,
   as in `9.1..+8b`.

   Conversion uses REAPER's tempo map (`TimeMap2_*`), which the bridge
   provides, so tempo changes are respected.
5. **Sections are regions.** A named REAPER region is a section of the
   piece. Findings and plans (ADR 0007, 0008) work at section level. With
   no regions, the whole project is one section.

## Consequences

- Clients can keep ids across edits. A deleted element gives a
  `not_found` error that names the id, never a silent mis-edit.
- The user's regions double as mort's vocabulary of structure, so the
  user and the agent mean the same thing by "the bridge" or "chorus 2".
