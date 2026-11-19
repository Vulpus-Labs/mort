# The E003 proof

Ticket 0022, run on 2026-11-19 in REAPER 7.67 on macOS, with the person at
REAPER and the map view open in their browser. The session was built by
[e003-build.kdl](e003-build.kdl): Bass (vxn-1c), a Chords folder holding Pad
(vxn-2b) and Warm (Poly-Kritter, with ReaEQ), and Keys (ReaSynth); regions
verse (bars 1–5) and chorus (bars 5–9); MIDI on every track. Probes were added
with `mort probes ensure`; nothing was measured with `measure`. The map filled
from listening alone.

No screenshots were taken. The person checked the page by eye and reported
what it showed, as below.

## 1. Play two sections by hand

The person played the verse and the chorus with REAPER's transport. Each play
became one audition capture that grew every 2 s while it played:

```text
$ mort map
2 captures
  #  range        source    taken    tracks
  1  0.70–16.20s  audition  4m ago   6
  2  0.70–16.20s  audition  33s ago  6
```

The first 0.7 s of each play is the warm-up (ADR 0012 §2). **Found:** the
page fetched the view once and then kept asking for that first, short range,
so the grid stopped growing while the cursor ran on across the window. Fixed:
the page always fetches everything measured. The person confirmed the grid and
cursor then lined up and filled in live.

## 2. Solo a track and play

The person soloed Pad (inside Chords) and played the verse:

```text
Pad     [(0.9, 2.25, 7), (2.3, 4.25, 7), (4.3, 8.3, 7)]
Bass    [(0.9, 6.3, 7), (6.3, 8.3, 7)]
Warm    [(0.9, 6.3, 7), (6.3, 8.3, 7)]
Keys    [(0.9, 6.3, 7), (6.3, 8.3, 7)]
Chords  []
MASTER  []
```

Capture 7 updated the tracks, whose probes are pre-fader, and not Chords or
the master, which lost inputs to the solo. Nothing went stale: solo is not
audio state. **Found:** 50 ms holes in Pad's coverage where a growing capture's
next part started a frame late. Fixed: a growing capture carries each track on
from where it ended.

## 3. An EQ edit by hand, then play again

The person dragged Warm's ReaEQ Band 2 gain to -5.6 dB by hand:

```text
$ mort map
  track   covered      stale
  MASTER  0.70–16.20s  0.70–16.20s (Warm changed)
  Bass    0.70–16.20s  -
  Chords  0.70–16.20s  0.70–16.20s (Warm changed)
  Pad     0.70–16.20s  -
  Warm    0.70–16.20s  0.70–16.20s (Warm changed)
  Keys    0.70–16.20s  -
```

Warm and the buses it feeds went stale, and nothing else did. The person then
played 0.7–4.1 s, skipped forward and played 12.7–16.2 s. Those two stretches
became fresh and the skipped 4.1–12.7 s stayed stale, as it should. **Found:**
zero-length stale slivers ("0.70–0.70s") where two plays met, because two
playbacks' frame grids sit a few microseconds apart. Fixed: covers shorter
than half a frame are dropped.

## 4. Lower a threshold on the page

On the page's thresholds tab the person set `loudness_jump` to 0.2 LU. A
marker appeared on the master at the chorus, and the CLI saw the same:

```text
$ mort findings --kind loudness_jump
chorus 8.00–11.00s
  0.05 loudness_jump  short-term loudness moves -0.3 LU into chorus: if that is not meant, ride the level across the boundary *
1 finding

$ mort thresholds | grep loudness_jump
loudness_jump    0.2 LU *             6        ADR 0007's starting point. …
```

## The page

- **Readability:** the person found it readable in light and dark mode.
- **Speed:** after a refresh while playing, it redrew 45 changed cells in
  0.6 ms on this session. On the fake 46-track, 5-minute session
  (`examples/fake_bridge`, 600 columns of 0.5 s), it redrew 527 cells in
  2.3 ms, well under the 16 ms the ticket asks for.

## ADR 0012's open questions, measured

- **Does REAPER reset plugins on a seek?** Yes. The probe counts the host's
  CLAP `reset` calls (its read-only `resets` parameter). Across one play with a
  skip and a stop, every probe's count went from 12 to 22, and the smoke run
  shows one reset for each playback start. The warm-up rule stands: after a
  seek the analyzer starts afresh in any case, and so do plugins with tails.
- **Does dragging an FX knob move the state count during the drag?** Yes,
  about once a second while dragging, and again on release. With the state
  sampled every 50 ms during a 3 s drag and a 2 s hold, the count moved at
  +0 s, +1.4 s and +2.4 s, then at +4.1 s on release. The auditioner splits
  stretches at each move and drops the frames on the tick of each change, so
  audio heard during a drag lands in short captures under passing states,
  which are stale at once. It is never kept as fresh.
- **Do tracks that are not soloed still run their FX?** Yes. During a 2.5 s
  play with Synth soloed, Kick's probe pushed 55 frames. So a track's own data
  under solo describes it fairly (it is pre-fader), which is why ADR 0012 §2
  keeps leaves and drops only buses with silenced inputs.

## Also fixed on the way

- A `batch` answered a bare array, so it could not carry `state` and
  `undo_point`, and `mort run` reported "no undo point" for every batch. A
  batch now answers `{results, state, undo_point}` (ADR 0002 §7).
- `scripts/smoke.sh` has 72 checks, all passing in REAPER after the fixes:
  solo auditions, reset counts and whether unsoloed FX run, the view, the
  watch, and the batch shape.
