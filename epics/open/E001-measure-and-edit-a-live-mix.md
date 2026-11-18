---
id: E001
title: "Measure and edit a live mix from the command line"
status: open
created: 2026-11-18
---

## Goal

Drive a running REAPER session with `mort` commands alone. Build tracks
with instruments, write MIDI and automation, measure the result into a
mix map, read findings, edit, and show by comparison that the edits did
what they were meant to.

The proof (ticket 0015) is a session built from scratch by `mort`
commands:

- four to six tracks, at least two of them silkscreen instruments;
- two regions;
- a masking clash planted between bass and pad, and a planted mud
  build-up.

`mort findings` must report both. ReaEQ edits made with `mort` must clear
them, and `mort compare` must show the change. A ReaEQ edit the person
makes by hand must show the affected track and its buses as stale. One of
mort's edits, undone from REAPER's undo history, must restore the
earlier finding when re-measured.

Decisions: ADRs 0001–0007, 0009 and 0011. ADR 0008 (plans) and ADR 0010
(manifests) are E002.

## Tickets

| Ticket | What |
|---|---|
| 0001 | Workspace; `mort-proto`: ids, musical positions, RPC envelope, errors |
| 0002 | `mort-session`: the `Session` trait and `FakeSession` |
| 0003 | Bridge skeleton: extension loads, socket, main-thread dispatcher, jobs, `bridge.hello` |
| 0004 | CLI skeleton: finding the socket, `status`, `call`, output modes, exit codes, `install` |
| 0005 | Read model: project, tracks, routing, FX, params, regions, tempo map, items |
| 0006 | Edits: tracks, FX, params, mix controls; undo points, `expect_state`, the wheel, `undo`, `batch` |
| 0007 | Automation: read and write envelopes |
| 0008 | MIDI: items, notes and CC, in musical positions |
| 0009 | Transport and renders as jobs; run ids |
| 0010 | `mort-features`: features, reference tests, filter-bank choice, cost |
| 0011 | `mort-probe`: CLAP plugin, attach, ring, slot identity, stamping |
| 0012 | Captures: assembly, staleness, sidecar persistence, `measure --stale` |
| 0013 | Findings and queries; calibration on Fenland material |
| 0014 | `compare`, and `mort run` scripts |
| 0015 | End to end in REAPER: the proof above |

## Dependency order

0001 first. Then 0002 and 0010 in parallel. Then 0003, then 0004, then
0005. Then 0006 and 0009 in parallel (both need 0005). Then 0007 and 0008
(need 0006), and 0011 (needs 0009 and 0010). Then 0012 (needs 0011). Then
0013 (needs 0012 and 0008). Then 0014 (needs 0013 and 0007). Then 0015.
