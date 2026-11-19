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

- [ ] `automation.write` on a send with no volume envelope creates one,
      as one undo point, without changing anything else in either chunk.
- [ ] Smoke checks it on a track with two sends to the same receiver.
