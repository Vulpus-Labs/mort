# ADR 0011: Testing without REAPER

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

REAPER needs a logged-in GUI session and cannot run in automated tests.
Most of mort does not need it: features, map, findings, protocol and
CLI parsing are pure logic.

## Decision

1. **The bridge's REAPER calls go through a `Session` trait** in
   `mort-session`: project reads, edits, undo blocks, the state count,
   chunks, the tempo map and renders. `ReaperSession` implements it in
   `mort-bridge`. `FakeSession` implements it over an in-memory project
   model.
2. **The fake can render.** Its tracks hold test signals or a small set
   of DSP sources (sine, noise, filtered noise, a simple synth voice).
   Rendering a range runs the real `mort-features` code over them and
   feeds the real capture path, so the measurement and findings pipeline
   runs from start to finish in `cargo test`.
3. **Features are tested against references:**
   - loudness against the EBU Tech 3341 and 3342 test signals;
   - true peak against the BS.1770 test signals;
   - bands against analytic answers for sines and pink noise.
4. **The dispatcher and RPC server run over `FakeSession`** in tests, so
   the CLI is tested against a real socket.
5. **REAPER checks are manual and scripted.** `scripts/smoke.sh` drives a
   running REAPER through a set sequence and checks the results. It is
   run before closing tickets that touch `ReaperSession` or the probe,
   and the result is recorded in the close-out.

## Consequences

- `cargo test` covers everything except the REAPER and CLAP glue.
- The fake must keep up with the real thing. When the smoke script finds
  a gap, the fix goes into the fake as well.
