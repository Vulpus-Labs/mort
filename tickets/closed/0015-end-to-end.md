---
id: "0015"
title: "End to end in REAPER: the E001 proof"
priority: high
created: 2026-11-18
epic: E001
depends: ["0014"]
---

## Summary

Run the proof in E001's goal in a real REAPER session, using only `mort`
commands, and record it.

## Acceptance criteria

- [x] Session built from scratch: four to six tracks, at least two
      silkscreen instruments, two regions, MIDI and automation.
- [x] Planted bass/pad masking and a planted mud build-up are both
      reported.
- [x] ReaEQ edits through `mort` clear them, and `compare` shows it.
- [x] A ReaEQ edit made by hand shows that track and its buses as stale.
- [x] Undoing a mort edit from REAPER's undo history, then re-measuring
      with `--stale`, brings the finding back.
- [x] The full command log, with outputs, is kept in `docs/e001-proof.md`.

## Close-out (2026-11-19)

The proof ran in REAPER 7.67 on a new project, using only `mort` commands
except for the two hand steps the epic asks for. The full log, with every
output, is in [docs/e001-proof.md](../../docs/e001-proof.md), and the fix
script in [docs/e001-fix.kdl](../../docs/e001-fix.kdl).

- **Session:** Bass (vxn-1c), Pad (vxn-2b), Warm (Poly-Kritter) and Keys
  (ReaSynth); regions verse and chorus; MIDI on all four; volume
  automation on Keys.
- **Planted problems reported:**
  - Bass/Pad masking at 80–125 Hz (severity 0.57).
  - Mud on the master in the chorus: 2.9 dB over the tilt, Warm blamed.
  - mort also found an unplanted minor-second clash (Pad B2 against Warm
    C3) and Bass/Warm masking.
- **Fix:** one script, six ReaEQ edits in one batch. `compare` showed both
  findings gone, Pad down about 30 dB below 125 Hz and Warm about 14 dB
  at 250–400 Hz.
- **Hand edit:** a ReaEQ gain changed on Warm made Warm and the master
  stale, and nothing else.
- **Undo from REAPER's undo history:** going back past mort's fix made
  Pad, Warm and the master stale. `measure --stale` re-measured just those
  three, and both findings returned with the same values.
- **Smoke:** `scripts/smoke.sh` passes all 68 checks in REAPER, including
  the 0013 and 0014 query, compare and batch-reference checks, and a new
  rollback check.

Changes made for this ticket:

- **Rollback fix (ADR 0004 §7).** REAPER moves the state count only when
  an undo block closes, and does so even for an empty block. The
  dispatcher's test "did the count move inside the block" was therefore
  never true, and failed edits were never rolled back in REAPER. Edits are
  now judged by whether the top undo label became their own, and a label
  equal to the one on top is numbered `(2)`, `(3)`, …. The fake now moves
  the count the way REAPER does. Covered by a dispatch test and a smoke
  check.
- **`mort region add <name> <range|position>`**, with the RPC
  `region.add`, `Session::add_region`, and the fake and REAPER
  implementations.
- **Smoke:** the expectations for the silent smoke mix were corrected, and
  ReaEQ's real parameter names are used ("Gain-Band 2"; the fake's ReaEQ
  calls its one band "Gain-Band 1").

Notes:

- REAPER's `TrackFX_AddByName` matched "silkscreen Poly-Kritter" but not
  "Poly-Kritter".
- The silkscreen instruments ship hot: the first measure read +6.3 dBTP on
  the master before faders were set.
