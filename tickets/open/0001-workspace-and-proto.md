---
id: "0001"
title: "Workspace; mort-proto: ids, musical positions, RPC envelope, errors"
priority: high
created: 2026-11-18
epic: E001
depends: []
---

## Summary

Set up the cargo workspace with the crates of ADR 0001, as empty crates
with docs. Write `mort-proto`: typed GUID ids, positions and their
parsing (ADR 0003), JSON-RPC request, response and notification types, and
the error shape of ADR 0002 §6.

## Acceptance criteria

- [ ] Workspace builds and passes the CLAUDE.md verification set.
- [ ] `TrackId`, `FxId`, `ItemId`, `TakeId`, `EnvelopeId`: distinct types,
      serialised as REAPER's `{GUID}` text.
- [ ] Positions: `9.1`, `9.2.240`, `+4b`, `12.5s` parse. Bad input errors
      show the input, the accepted forms and an example. Conversion to
      seconds takes a tempo-map trait, tested with a constant tempo and with
      a tempo change.
- [ ] RPC envelope round-trips, including `batch`. Error `data` has `kind`,
      `subject` and `fix`.
