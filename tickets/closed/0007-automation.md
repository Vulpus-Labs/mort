---
id: "0007"
title: "Automation: read and write envelopes"
priority: high
created: 2026-11-18
epic: E001
depends: ["0006"]
---

## Summary

Read and write track and FX-parameter envelopes, in musical positions,
as one undo point per write.

## Acceptance criteria

- [x] `envelope.get` for volume, pan, sends and any FX parameter, creating
      the envelope on write if it is missing.
- [x] Write points with shape (linear, square, slow, bezier with tension).
      Write a ramp or a hold over a range. Clear a range.
- [x] CLI `mort automate <track> <target> 9.1=0.2 13.1=0.8 --shape slow`
      and `--clear 9.1..17.1`.
- [x] Round-trip tests over the fake. Smoke in REAPER shows the envelope as
      expected.

## Close-out (2026-11-19)

- Bridge:
  - `Live` finds envelopes as follows:
    - volume and pan by chunk name (`<VOLENV2`, `<PANENV2`);
    - FX parameters with `GetFXEnvelope`;
    - sends with `P_ENV:<VOLENV`.
  - It identifies an envelope's target from its parent FX and parameter,
    or by matching those pointers.
  - It creates missing envelopes as follows:
    - FX-parameter envelopes with `GetFXEnvelope(create)`;
    - volume and pan with REAPER's actions *Toggle track volume/pan
      envelope visible* (40406, 40407), run on that track alone, with the
      person's track selection restored afterwards.
  - Values cross the wire as linear gain, pan −1 (left) to 1, and
    normalised parameters:
    - REAPER's fader scaling goes through `ScaleTo/FromEnvelopeMode`;
    - pan envelopes run the opposite way to track pan;
    - parameter envelopes use the parameter's native range.
- RPCs:
  - `envelopes.list`;
  - `envelope.points`, with REAPER's display text for each point
    (`Envelope_FormatValue`);
  - `automation.write`, which by default replaces the points between the
    first and last new point;
  - `automation.clear`;
  - `param.parse`, display text to normalised value.
- CLI:
  - `mort automate <track> <target> [POS=VALUE…] [--shape] [--tension]
    [--add] [--clear RANGE] [--text]`.
  - Targets are `volume`, `pan`, `send:<track>` or `<fx>/<param>`.
  - Positions chain (`+4b`) through `time.convert`.
  - Clearing and writing together go in one `batch`, so they make one
    undo point.
  - Listing shows bar.beat, seconds, REAPER's text and the shape.
- What REAPER actually does, found by the smoke run:
  - `GetTrackEnvelopeByChunkName` and `P_ENV:<VOLENV` return an envelope
    even when the track or send has none. An envelope counts as existing
    only if `CountTrackEnvelopes`/`GetTrackEnvelope` list it. Without this
    check, mort wrote into envelopes that do not exist.
  - Pan envelopes: −0.5 written shows as `50%L`, which confirms the sign
    flip.
- Deviation: mort does not create send envelopes. REAPER has no API for
  it. mort says to show the envelope once in REAPER, and deferred ticket
  0016 covers creating it.
- Smoke: 39 checks, all `ok` in REAPER 7.67. The 7 new ones cover volume
  created and written (shape and display), pan direction, a parameter
  envelope, clearing a range, `envelopes.list`, refusing a send envelope
  clearly, and undo.
- Tests: CLI 20, including volume, pan, params by text, sends, replace and
  `--add`, and clear-and-write as one undo point.
