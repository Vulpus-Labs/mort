---
id: E003
title: "Auditions feed the map; a live view of the whole map"
status: open
created: 2026-11-19
---

## Goal

Playback in REAPER, by the person or by mort, keeps the mix map current
without a `measure`. A page in the browser shows the whole map: how
recent each part is, what is stale, what was never measured, and where
the findings over the project's thresholds are. It updates while the
person listens.

The proof (ticket 0022) is a session in REAPER:

- play two sections by hand and watch them fill in on the page;
- solo a track and play: the track updates, its buses do not;
- edit an EQ by hand: the track and its buses turn stale, and playing
  the section again makes them fresh;
- lower a threshold on the page: a finding appears in both the page and
  `mort findings`.

Decisions: ADR 0012. It amends ADR 0006 §1 (solo), ADR 0007 §3
(overrides), ADR 0002 (`map.watch` off the main thread) and ADR 0009
(`mort view`, `mort thresholds`).

Starts after E001's ticket 0013, since findings and thresholds come from
there.

## Tickets

| Ticket | What |
|---|---|
| 0017 | Audition captures: stretches, validity rules, warm-up, cached hashes, solo out of the hash |
| 0018 | Freshness and the change feed: `map.view`, `map.watch`, findings cache per section |
| 0019 | Threshold overrides: `thresholds.toml`, `thresholds.get/set`, `mort thresholds` |
| 0020 | `mort view` server: loopback HTTP, token, SSE, JSON pass-through, embedded page |
| 0021 | The page: grid, freshness, stale, findings, master lane, cursor, panels |
| 0022 | End to end in REAPER: the proof above |

## Dependency order

0017 needs 0012 (closed). 0019 needs 0013. 0018 needs 0017 and 0019.
0020 needs 0018. 0021 needs 0020. 0022 needs 0021.
