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
     centres);
   - side energy per octave;
   - L/R correlation, and correlation below 150 Hz;
   - spectral centroid and spectral flatness;
   - onset strength (spectral flux), from which transient density is
     derived.

   Bands come from FFTs, not an IIR filter bank. Ticket 0010 measured
   both at 48 kHz on an M3. A full-rate bank of sixth-order Butterworth
   band-passes was accurate on sines (0.02 dB) but read pink noise up to
   0.33 dB high through its skirts, and cost 0.47 % of real time per
   stereo instance for the bands alone. One 32768-point FFT per frame was
   accurate (0.06 dB on sines, 0.12 dB on pink noise) but cost 0.31 %. We
   use two resolutions instead: an 85 ms FFT at the full rate for bands
   from 800 Hz up, and a 683 ms FFT of the signal decimated by 16 for bands
   up to 630 Hz, with each bin weighted into bands by how much of it lies
   inside. It is as accurate as the long FFT, and the whole analyzer,
   every feature included, costs 0.11 % per instance (40 instances use
   4.3 % of a core). The FFTs also give the centroid, flatness and onset
   strength, which a filter bank would need a separate FFT for. The price
   is time resolution: bands below 708 Hz describe the last 683 ms, not
   the last 50 ms. The frame layout is `mort_features::Frame`, 220 bytes.

2. **Findings** are computed from the map over a range. Each one carries:
   - `kind`, `section`, `range`, `tracks`, `bands`;
   - `severity` (0–1);
   - `evidence`: the numbers that triggered it;
   - `suggest`: one sentence on the usual remedy, not an edit.

   Kinds in E001:
   - `masking`: two inputs of the same bus (a folder's children, or the
     top-level tracks for the master) with both significant energy
     (within 6 dB of each other and within 12 dB of the bus's level in
     that band) in the same bands for over 30 % of a section, in at least
     three third octaves.
   - `mud`: bus or master energy from 200–500 Hz more than a set amount
     above the bus's own spectral tilt: a line fitted to its mean
     third-octave levels from 63 Hz to 8 kHz, leaving out the bands under
     test.
   - `harsh`: the same test from 2–5 kHz.
   - `headroom`: true peak above −1 dBTP on the master, or a bus's sample
     peak above 0 dBFS (clipping internally).
   - `loudness_jump`: master short-term loudness moving more than a set
     amount at a section boundary: its value just before the boundary
     against the first value whose 3 s window lies wholly after it. No
     plan can mark a jump as intended until E002 (ADR 0008).
   - `low_end_width`: correlation below 150 Hz under 0.5 in over 30 % of
     the frames that have low end, on the master and on tracks that are
     not buses.
   - `register_clash`: two MIDI parts sustaining notes a minor second
     apart, both below C4, overlapping for at least 0.1 s. This is read
     from the MIDI, not the audio.

   Sections are the project's regions within the range. Stretches that no
   region covers are sections named `(no region)`, and a range with no
   regions is one section named `range`. A track's part in a bus is its
   probe's measurement scaled by its fader and by its pan's effect on mid,
   taken as a balance control; muted tracks count as silent. Automation
   is not yet taken into account.
3. **Thresholds are measured.** Every threshold above is a starting point.
   Thresholds live in one table in `mort-map` (`thresholds::TABLE`, shown
   by `mort findings --thresholds`), with the reason for each value.
   - Ticket 0013 calibrated them on synthetic mixes with problems planted
     in them, at the user's choice: pink-noise parts in neighbouring
     ranges, with an overlapping part, a low-mid or upper-mid band, a
     louder chorus, a wide bass or clashing notes added one at a time. The
     calibration test prints every measurement, flagged or not.
   - That set the masking band count at three (clean crossovers reach
     two) and the mud and harsh excess at 2 dB. Clean mixes read up to
     1.2 dB; a band at the others' level reads 2.9–3.3 dB.
   - Calibration on Fenland's stems, where we know what was fixed and
     why, comes later (ticket 0023).
   - `mort findings --all` reports every measurement as if it were a
     finding, so a caller can see how close each one is to its
     threshold.
4. **Queries** come at three levels:
   - `summary(range)`: one row per section, with loudness, tilt, width,
     density and finding counts;
   - `track(id, range)`: that track's features per section, and its share
     of each band in the bus it feeds;
   - `frames(id, range, features, step)`: raw frames resampled to `step`,
     for when the summaries are not enough.

   They are the RPCs `map.summary`, `map.track` and `map.frames`, with
   `map.findings` for findings. The CLI has `mort summary`, `mort report
   <track>`, `mort frames <track> <range>` and `mort findings`. It prints
   compact text by default and JSON with `--json`. `mort findings` shows
   the 20 worst unless `--limit` says otherwise.
5. **Compare.** `compare(capture_a, capture_b, range)` gives, per track
   and section, the changes in features and the findings that appeared or
   disappeared. This is how an edit is judged.
   - Each side is the map as it was just after that capture: every kept
     capture up to it, laid over the tracks it measured, in order.
   - Each side is read with the track gains, buses and MIDI its capture
     recorded, so a fader move or a revoiced part shows as a change.
   - Two findings are the same problem when they share kind and section,
     and the same pair of tracks (masking, register clashes) or the same
     bus or track (the rest).
   - By default, the range is the later capture's, and the pair is the
     latest capture over it and the one before.
   - The RPC is `map.compare`; the CLI is `mort compare [range] [--a]
     [--b]`. It lists tracks that moved by 0.5 dB or more in loudness or
     in any third octave, with the bands that moved most. Then it lists
     findings gone, findings new, and findings kept, with their severity
     before and after.

## Consequences

- An agent can work at section level most of the time and drill down
  only where a finding points.
- Findings suggest remedies but never apply them. Deciding and editing
  stay with the agent or the person.
