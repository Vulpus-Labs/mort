# ADR 0005: The probe

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The bridge cannot hear audio. REAPER's API gives peak meters and nothing
finer. Measuring needs code in the signal path on each track, each bus and
the master, and the measurements must line up with the music's timeline
whether the project is playing in real time or rendering faster.

## Decision

1. **`mort-probe` is a stereo CLAP effect that passes audio through
   unchanged** and has no UI. The bridge inserts it last in each chain it
   measures (after the FX, before the fader) and on the master. It sets the
   instance's name to `mort probe` so a person can see what it is.
2. **Frames.** The probe emits one feature frame per 50 ms of audio.
   `mort-features` (ADR 0007 defines the features) is called from the audio
   thread with fixed-size state allocated on activate.
3. **Stamping.** Each frame carries the project position in seconds of its
   first sample, taken from the CLAP transport in the process call plus the
   sample offset. It also carries a `run` id the bridge sets before each
   play or render. Frames from blocks with no transport, or with transport
   stopped, are dropped. The same code therefore measures live playback and
   offline renders.
4. **Attach.** On activate, the probe asks the host for
   `cockos.reaper_extension`. Through `GetFunc` it looks up
   `mort_probe_attach`, which the bridge registers with
   `plugin_register("API_mort_probe_attach")`. Attach returns a slot
   number and a pointer to a single-producer, single-consumer ring the
   bridge owns. The probe writes frames into the ring. The bridge reads them
   on a timer.
5. **Identity.** The probe shows its slot number as a read-only parameter,
   `slot`. The bridge reads `slot` from every probe instance it finds in the
   FX chains, so it knows which track, bus or master each slot belongs to.
   No path from plugin to track is needed.
6. **Detached probes.** If `cockos.reaper_extension` or the attach function
   is missing (another host, a REAPER that has no bridge, a plugin run in a
   separate process), the probe passes audio through and its `attached`
   parameter reads 0. The bridge reports such instances as unmeasured.
7. **Audio-thread rules:** no allocation, no locks, no system calls. If the
   ring is full, the frame is dropped and counted, and the count is reported
   with each run.

## Consequences

- One plugin binary measures everything, and keeping it last in the chain
  means it hears what the track sends to the mix.
- Running REAPER's FX "in a separate process" defeats attach. The bridge
  says so per instance.
- Pre-fader placement means the bridge combines the measurements with fader
  and pan gain from the project to estimate each track's contribution to
  the mix. Bus and master probes measure the real sum.
