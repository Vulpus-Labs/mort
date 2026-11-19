---
id: "0016"
title: "Create send volume envelopes"
priority: low
created: 2026-11-19
epic: E001
depends: ["0007"]
---

## Summary

mort writes a send's volume envelope only if it already exists. REAPER's
API has no call to create one. Its action for this works on the send under
the mouse, and the send's envelope lives on the receiving track's chunk
(`<AUXVOLENV` after the matching `AUXRECV` line). Creating one means
editing that chunk, which is safe to do only once the mapping from sender
send index to receiver `AUXRECV` line has been checked against REAPER.

Until then, `automation.write` on a send with no envelope fails with
`invalid` and tells the person to show the envelope in REAPER first.

## Acceptance criteria

- [x] `automation.write` on a send with no volume envelope creates one,
      as one undo point, without changing anything else in either chunk.
- [x] Smoke checks it on a track with two sends to the same receiver.

## Close-out (2026-11-19)

- **How it works.** `automation.write` on a send with no volume envelope
  now creates one: the bridge adds an `<AUXVOLENV` block to the receiving
  track's chunk, straight after that receive's `AUXRECV` line, and sets
  the chunk. This runs inside the edit's undo block, so creating and
  writing make one undo point. The fake already allowed it, so only
  `ReaperSession` changed.
- **What REAPER does, found in 7.67:**
  - It drops an envelope block that has no points: `SetTrackStateChunk`
    returns true, and the block is gone when the chunk is read back. A
    new envelope therefore starts with one point at time 0 holding the
    send's volume (fader scaling, `VOLTYPE 1`), which is what REAPER does
    when a person shows the envelope. The sound does not change until
    points are written.
  - The envelope is listed with the *sending* track's envelopes
    (`CountTrackEnvelopes` on the sender is 1, on the receiver 0), so
    mort's existence check from 0007 holds.
  - A send and its receive answer `P_ENV:<VOLENV` with the same pointer.
    The bridge matches a send to its receive that way, with no
    assumption about order. Order would not have been safe: Verb listed
    its receives as Kick, Kick, Bass although Bass's send was added
    between Kick's two, so receive order is not creation order.
- **Code.**
  - `chunk::add_send_volume_envelope` adds the block. It refuses a
    receive that already has a volume envelope, and lines inside the FX
    chain do not count as receives.
  - `raw` gains `receive_count`, `receive_envelope` and `new_guid`.
- **Tests.** One chunk unit test checks that the block goes after the
  right receive (ahead of an existing pan envelope) with one point, that
  every other line is unchanged, and the refusals.
- **Smoke.** `scripts/smoke.sh` passes all 75 checks in REAPER 7.67. The
  old "refused clearly" check is replaced by four new ones. Kick gets a
  second send to Verb, at 0.5. Then:
  - an envelope on that send is created and written (−6.02 dB, then
    −12.0 dB);
  - only send 1 has one;
  - Kick's and Verb's track listings, sends included, are unchanged;
  - one `undo` (`mort: smoke send env`) removes it.
- **Not checked in REAPER:** that every other chunk line is byte for byte
  the same. There is no chunk RPC to read it through, so the unit test
  covers this, and in REAPER only the observable state is compared.
- **Not measured:** whether setting the receiver's chunk makes REAPER
  re-create its FX, as setting a chunk can. If it does, creating a send
  envelope on a heavy bus during playback may cause a short dropout.
  `fx.set_state` sets chunks in the same way.
