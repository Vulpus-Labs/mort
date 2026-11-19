# ADR 0012: Auditions and the map view

- **Status:** Proposed
- **Date:** 2026-11-19

## Context

The mix map (ADR 0006) grows only when `mort measure` runs. But most
listening happens when the person plays parts of the project in REAPER,
or when mort plays them. The probes stay in the chains after a measure
and keep emitting frames during that playback. Those frames carry run 0,
and the bridge buffers them and then throws them away
(`probes::discard_except`).

The person also has no picture of the whole map: which parts were heard
recently, which are stale, which were never measured, and where the
findings are. `mort map` and `mort findings` print text for one range at
a time.

## Decision

1. **Audition captures.** Frames with run 0 are folded into the map as
   captures with source `audition`. Captures made by `measure` get source
   `render` or `play`. An audition capture covers one *stretch*: playback
   with no break in it. A stretch ends when:
   - the transport stops or pauses;
   - the frame stamps jump (a seek, a loop wrap, or a gap of more than
     1.5 frames);
   - the project state count moves (ADR 0004);
   - the solo set, the play rate or the record-monitoring set changes.

   The first three are read from the stamps and on the main thread's
   timer tick. The bridge already reads REAPER there (ADR 0002).
2. **What a stretch may replace.** A stretch replaces older data over its
   range, as in ADR 0006 §2, but only for the tracks whose frames are a
   fair measure of the mix:
   - The play rate must be 1.0. Otherwise nothing is kept.
   - Tracks armed with input monitoring on hear live input. Their frames
     are dropped, together with the frames of everything downstream of
     them.
   - While anything is soloed, a track or bus is kept only if nothing
     routed into it is silenced by the solo. A track with no inputs is
     always kept: its probe is pre-fader (ADR 0005), so solo elsewhere
     does not change what it hears.
   - Muted tracks send no frames (ADR 0006 §4). Their old data stays.
   - **Warm-up.** After a seek, the analyzer's windows still hold audio
     from before it. The first 683 ms of a stretch is dropped (the
     long-FFT window of ADR 0007 §1). Short-term loudness is marked
     unknown until 3 s into the stretch, and queries skip unknown values.
     Shorter stretches add nothing.
3. **Hashes for auditions.** Each audition capture records the chunk
   hashes of the state it was played under. Hashing every chunk is too
   slow to do at play time. The bridge therefore keeps hashes cached per
   state count and refreshes them on timer ticks after each change,
   within the tick budget. A stretch that starts before the refresh is
   done waits for it before it is accepted. Its frames stay buffered
   until then.
4. **Solo is not audio state.** The solo flag is dropped from the audio
   chunk (the `MUTESOLO` line keeps only its mute field). This amends
   ADR 0006 §1. A track's own sound does not change when it is soloed,
   and auditioning means soloing constantly. Mute stays in the hash.
5. **Persistence.** Audition captures go into the same sidecar as other
   captures (ADR 0006 §6). Stretches from one playback, from start to
   stop, are written together in one `.frames` file when the playback
   stops, not one file per stretch.
6. **Freshness.** Every map cell (track, time) knows the source and wall
   clock time of the capture that holds it, apart from whether it is
   stale. The view shows both. *Fresh* means "measured recently".
   *Stale* means "the project changed since" (ADR 0006 §3). A cell can
   be old without being stale.
7. **Thresholds can be overridden per project.** The defaults stay the
   measured table in `mort-map` (ADR 0007 §3). A project can override
   any of them in `song.mort/thresholds.toml`, along with the minimum
   severity at which a finding is shown. `mort findings`, the agent and
   the view all use the same overrides. Every finding notes which of its
   thresholds were overridden. `thresholds.get` and `thresholds.set`
   read and write the file, and `mort thresholds` is the CLI for them.
8. **Change feed.** The map carries a version that grows with every
   change: a capture added, data going stale, thresholds changing.
   - `map.watch {since, timeout}` returns when the version passes
     `since`, or when the timeout ends. It returns the new version, the
     tracks and spans that changed, and the transport position. It is
     answered on the socket thread from shared state, which the main
     thread's timer updates, so the main thread is never blocked.
   - `map.view {start, end, step}` returns the grid for the view: one
     row per track in folder order, and per column the coverage, the
     newest source and time, the stale reason, and findings over the
     threshold.
   - Findings are cached per section and recomputed only for sections
     whose data or thresholds changed.
9. **The view is a local web page.** `mort view` starts an HTTP server
   in the CLI process:
   - It binds `127.0.0.1` only, on port 7317 or the next free one, and
     prints a URL holding a random token. Requests without the token, or
     with a `Host` other than the loopback address, are refused. This
     stops other sites from reading the page or making changes through
     it via DNS rebinding.
   - It turns `map.watch` into server-sent events and passes `map.view`,
     `track` and `thresholds.*` through as JSON.
   - The page is plain HTML, CSS and JavaScript drawn on a canvas,
     embedded in the binary with no build step.
   - The only change the page can make is to thresholds. It makes no
     edits to the project.
10. **What the page shows:**
    - a grid of tracks (rows, nested by folder) against time (columns),
      with regions and bars across the top;
    - each cell shaded by the age of its data. Stale cells are hatched,
      with the reason on hover. Cells never measured are left empty, and
      cells left unmeasured for a reason (muted) are marked with it;
    - findings over the threshold as markers on the cells they name,
      coloured by severity;
    - a master lane with short-term loudness and true-peak overs;
    - the play cursor, so the person sees the map fill in while they
      listen;
    - a side panel: clicking a cell shows that track's `track` query for
      the section, plus the evidence for any finding there. A thresholds
      panel edits the overrides.

## Consequences

- Listening in REAPER keeps the map current without anyone asking. A
  `measure` run is needed only for parts nobody has played since they
  changed, and `measure --stale` finds those.
- The rules in §2 drop a lot during heavy solo work. That is deliberate:
  data the map holds must describe the mix, or it is worse than none
  (ADR 0006, Context).
- A track's own data survives soloing (§4). Buses fed by silenced tracks
  do not take new data while the solo lasts.
- The CLI gains an HTTP server and a page to maintain. The bridge gains
  the off-thread `map.watch` path and nothing that draws.
- To check in REAPER (ticket 0022), not assumed here:
  - whether REAPER resets plugins on seek, which would change §2's
    warm-up rule;
  - whether dragging an FX knob moves the state count only on release,
    so that the frames heard during the drag are wrongly kept;
  - whether non-soloed tracks still run their FX.
