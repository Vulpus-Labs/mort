# ADR 0001: Parts and how they connect

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The goal is to compose and mix in a REAPER session that is running, the way
Fenland was composed offline: configure instruments, drive them with
automation and MIDI, measure what they produce, keep a map of the mix over
time, and refine. REAPER's built-in remote routes (OSC, the web interface)
reach transport and a few track controls. They cannot reach FX chains,
envelopes, MIDI or rendering, and they cannot measure anything. ReaScript
can reach the whole API, but only from inside REAPER, and it cannot see
audio.

## Decision

1. **mort has three parts.**
   - `mort-bridge` is a native REAPER extension (`reaper_mort.dylib` in
     `UserPlugins`). It is written in Rust on reaper-rs's `reaper-medium`
     layer, which is the stable one. It owns the RPC server, the main-thread
     dispatcher, the captures and the findings.
   - `mort-probe` is a CLAP analysis plugin with no UI, built with clack
     like silkscreen. It measures audio and passes frames to the bridge.
   - `mort` is the command-line client.
2. **The client and the bridge talk JSON-RPC over a Unix socket**
   (ADR 0002). The client is a separate process and never links REAPER.
3. **The probe and the bridge share memory inside REAPER's process**
   (ADR 0005). The probe never talks to the client.
4. **Crates:**
   - `mort-proto`: RPC types, ids and positions. Serde only.
   - `mort-features`: feature extraction DSP. No REAPER, no CLAP.
   - `mort-map`: captures, staleness, findings and queries. No REAPER.
   - `mort-session`: the `Session` trait the bridge implements over
     REAPER, plus the fake used in tests (ADR 0011).
   - `mort-bridge`: the extension cdylib.
   - `mort-probe`: the CLAP cdylib.
   - `mort-cli`: the `mort` binary.
5. **REAPER only, macOS first.** Nothing is designed to stop Windows or
   Linux, but they are not tested in E001. No other DAW is a goal.

## Consequences

- The agent works through `mort` commands in a shell. It needs no
  integration beyond the CLI.
- Most logic (features, map, findings, protocol) runs and is tested
  without REAPER.
- Two binaries have to be installed into REAPER, and REAPER has to be
  restarted when either changes (see silkscreen's note on hot-swapped
  bundles). `mort install` does both copies and says when a restart is
  needed.
