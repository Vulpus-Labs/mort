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
   - for each measured track, bus and master, a hash of its state chunk
     (`GetTrackStateChunk`, which covers FX state, envelopes, items and
     routing).
2. **The mix map** is the latest capture data for each (track, time)
   point. A new capture over a range replaces older data for that range on
   the tracks it measured.
3. **Staleness.** A track's map data is stale when its chunk hash has
   changed since capture. A bus or the master is stale when it is, or when
   anything routed into it is. Tempo-map changes make everything stale.
   Every query result says which of its parts are stale. `mort measure
   --stale` re-measures only stale ranges and tracks.
4. **Running a measurement:**
   - `render` mode renders the range faster than real time, with a dry run
     if REAPER's API allows it, and otherwise to a scratch file in
     `$TMPDIR/mort/renders` that is deleted afterwards. It is the default.
   - `play` mode plays the range through the person's monitors in real
     time, for when the person wants to listen too.
5. **Persistence.** Captures are saved in a sidecar directory next to the
   project file, `<project>.mort/captures/`, in a compact binary format
   with a JSON index. They reload when the project opens. An unsaved
   project keeps its captures in memory only.
6. **Size:** a frame is about 200 bytes (ADR 0007). Five minutes of
   40 tracks is about 48 MB. Old captures that are fully replaced are
   deleted.

## Consequences

- After an edit to one track, re-measuring only that track and the buses
  it reaches is a short render, not a full playback.
- Edits made by the person are tracked as precisely as mort's own,
  because staleness comes from project state, not from mort's own log.
