# ADR 0006: Captures and the mix map

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The agent needs to know what is happening in the mix at each point, and
whether that knowledge is still true after edits. Playing back a whole
piece to re-measure after every change is too slow. Measurements that no
longer match the project are worse than none.

## Decision

1. **A capture** is the set of frames from one run (a play or a render)
   over one time range. It records:
   - the run's range;
   - the time it was taken;
   - the project state count;
   - for every track, bus and master in the project, a hash of its
     *audio chunk*: its state chunk (`GetTrackStateChunk`, which covers FX
     state, envelopes, items and routing) without its name, selection,
     view and window lines, and without mort's probes. An FX chain left
     empty is dropped. The solo flag is dropped too (ADR 0012 §4), but
     mute stays. Renaming, selecting, soloing or adding a probe therefore
     leaves a track fresh;
   - for each measured track, the tracks routed into it at the time.
2. **The mix map** is the latest capture data for each (track, time)
   point. A new capture over a range replaces older data for that range on
   the tracks it measured.
3. **Staleness.** A track's map data is stale when its chunk hash has
   changed since capture. A bus or the master is stale when it is, or when
   anything routed into it is (by the routing then or now). A track
   routed in or out since capture makes it stale. Tempo-map changes make
   everything stale: the capture keeps a fingerprint of where each
   sixteenth note fell up to the range's end.
   Every query result says which of its parts are stale. `mort measure
   --stale` re-measures only stale ranges and tracks.
4. **Running a measurement:**
   - `render` mode renders the range faster than real time as a dry run:
     REAPER's action *File: Dry run render project, using the most recent
     render settings* (43349), with the project's `RENDER_*` settings
     borrowed for the range and master mix and then put back. No file is
     written. It is the default.
   - `play` mode plays the range through the person's monitors in real
     time, for when the person wants to listen too.
   - **Pre-roll** (ticket 0017). The analyzer's windows start empty, so a
     run that starts at the range's start reads its first frames wrong:
     the 63 Hz band 30 dB low at first, recovering over about 0.6 s, and
     short-term loudness climbing for 3 s. So a render starts 3 s before
     the range and a play 0.7 s before it, in whole frames, and the
     frames before the range are clipped. In play mode, short-term
     loudness is marked unknown (NaN) while its window reaches back
     before the pre-roll. No pre-roll is needed at the project's start,
     where the silence before is real.
   Measuring is an edit, because it may add probes (ADR 0004): it is
   refused while edits are held, and its undo point, if any, holds only
   the probes it added. It is the one kind of edit that queues a job. A
   batch still may not. REAPER does not run a muted track's FX by
   default, so a muted track is reported as unmeasured with that reason,
   and its old data stays, marked stale.
5. **Clipping.** REAPER processes a few blocks past the end of a play or
   render (about 150 ms of frames for a render in REAPER 7.67). A capture
   keeps only frames that start inside its range.
6. **Persistence.** Captures are saved in a sidecar directory next to the
   project file: `song.rpp` keeps them in `song.mort/captures/`.
   - `index.json` holds the format version, the next capture id, each
     kept capture's metadata (including each track's gain, bus and MIDI
     notes at the time), and each track's covers. A cover says which
     capture is current over which stretch.
   - Each capture has one `<id>.frames` file with the frames it took, in
     little-endian binary (the layout is in `mort_map::store`). A
     capture's frames never change, so its file is written once, as the
     capture is made. When
   the project's path changes, the bridge loads the sidecar beside the
   new path if there is one. If there is none, it keeps its map for the
   tracks the project still has and writes it there; this covers a first
   save and *Save as*. An unsaved project keeps its captures in memory
   only.
7. **Size and history:** a frame is 220 bytes (ADR 0007). Five minutes
   of 40 tracks is about 53 MB. A capture whose data newer captures have
   wholly replaced is *superseded*. The map keeps the 8 most recent
   superseded captures, so that `compare` can rebuild the map as it was
   after any of them (ADR 0007 §5). Older ones are deleted.

## Consequences

- After an edit to one track, re-measuring only that track and the buses
  it reaches is a short render, not a full playback.
- Edits made by the person are tracked as precisely as mort's own,
  because staleness comes from project state, not from mort's own log.
