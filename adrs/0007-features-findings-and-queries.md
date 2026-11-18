# ADR 0007: Features, findings and queries

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Raw frames are too many for an agent to read: about 20 per second per
track. The agent needs answers ("what is muddy in chorus 2, and who is
causing it?") and the evidence behind them, and must be able to drill
down from a section to a track to a band.

## Decision

1. **Features per frame** (`mort-features`):
   - momentary loudness (400 ms) and short-term loudness (3 s), K-weighted
     per ITU-R BS.1770;
   - sample peak, and true peak (4× oversampled, BS.1770 Annex 2);
   - RMS and crest factor;
   - mid energy in 31 third-octave bands from 20 Hz to 20 kHz (ISO 266
     centres). Whether these come from an IIR filter bank or a long FFT is
     decided by measuring accuracy and cost in ticket 0010;
   - side energy per octave;
   - L/R correlation, and correlation below 150 Hz;
   - spectral centroid and spectral flatness;
   - onset strength (spectral flux), from which transient density is
     derived.
2. **Findings** are computed from the map over a range. Each one carries:
   - `kind`, `section`, `range`, `tracks`, `bands`;
   - `severity` (0–1);
   - `evidence`: the numbers that triggered it;
   - `suggest`: one sentence on the usual remedy, not an edit.

   Kinds in E001:
   - `masking`: two tracks with both significant energy (within 6 dB of
     each other and within 12 dB of the bus's level in that band) in the
     same bands for over 30 % of a section.
   - `mud`: bus or master energy from 200–500 Hz more than a set amount
     above a reference spectral tilt.
   - `harsh`: the same test from 2–5 kHz.
   - `headroom`: true peak above −1 dBTP on the master, or a bus clipping
     internally.
   - `loudness_jump`: short-term loudness moving more than a set amount at
     a section boundary with no plan to say it should (ADR 0008).
   - `low_end_width`: correlation below 150 Hz under 0.5.
   - `register_clash`: two MIDI parts sustaining notes within a minor
     second of each other in the same octave below C4. This is read from
     the MIDI, not the audio.
3. **Thresholds are measured.** Every threshold above is a starting point.
   Ticket 0013 calibrates them on Fenland's stems and mixes, where we
   already know what was fixed and why. Thresholds live in one table in
   `mort-map`, with the reason for each value.
4. **Queries** come at three levels:
   - `summary(range)`: one row per section, with loudness, tilt, width,
     density and finding counts;
   - `track(id, range)`: that track's features per section, and its share
     of each band in the bus it feeds;
   - `frames(id, range, features, step)`: raw frames resampled to `step`,
     for when the summaries are not enough.

   The CLI prints compact text by default and JSON with `--json`.
5. **Compare.** `compare(capture_a, capture_b, range)` gives, per track
   and section, the changes in features and the findings that appeared or
   disappeared. This is how an edit is judged.

## Consequences

- An agent can work at section level most of the time and drill down
  only where a finding points.
- Findings suggest remedies but never apply them. Deciding and editing
  stay with the agent or the person.
