---
id: "0020"
title: "mort view: loopback HTTP server"
priority: medium
created: 2026-11-19
epic: E003
depends: ["0018"]
---

## Summary

Serve the map view from the CLI process (ADR 0012 §9).

## Acceptance criteria

- [x] `mort view [--port N] [--open]` binds 127.0.0.1 on 7317 or the next
      free port, and prints a URL holding a random token.
- [x] Requests without the token, or with a non-loopback `Host`, get 403.
      Tested.
- [x] `/events` is SSE driven by `map.watch`. `/api/view`, `/api/track`
      and `/api/thresholds` pass through to the bridge.
- [x] Losing the bridge is reported on the page and retried. The server
      does not exit.
- [x] The page's assets are embedded with `include_str!`. No npm, no build
      step. One small HTTP dependency at most, chosen in the close-out
      with its reason.
- [x] CLI test over the fake: start the server, fetch the view, trigger
      an audition, receive an event.

## Close-out (2026-11-19)

- **`mort view`** (`crates/mort-cli/src/view.rs`):
  - Binds 127.0.0.1 on 7317 or the next free of 20 ports (`--port 0`
    takes any), and prints `http://127.0.0.1:PORT/?t=TOKEN`. The token is
    128 bits from `/dev/urandom`. `--open` opens it.
  - Every request needs the token and a `Host` of `127.0.0.1:PORT` or
    `localhost:PORT`, or it gets 403.
  - Routes: `/` (the page), `/events` (SSE), `/api/view`, `/api/track`,
    `/api/thresholds` (GET and POST).
  - Each connection runs on its own thread with its own bridge
    connection, so no locks are shared.
- **Events:** `change` when `map.watch` reports a new version, `cursor`
  with the position (watches wait 200 ms while playing, 15 s otherwise),
  and `bridge` with `up` false or true. Without a bridge the stream says
  so, pings every 2 s and keeps trying, and the API answers 503 with
  `bridge: false`. The server never exits on its own.
- **Dependency: none.** The server needs a request line, a few headers,
  a small POST body, fixed routes and an event stream, which come to
  about 150 lines on `std::net`. A crate such as `tiny_http` would bring
  its own threading and parsing, for nothing this needs, and would put
  the loopback and token checks behind its abstractions. The page is
  embedded with `include_str!` from `crates/mort-cli/assets/view.html`;
  0021 fills it in.
- **Tests (CLI, over the fake):**
  - URL and token; 403 without the token, with a wrong one, and with a
    foreign `Host`; the page; `/api/view`; threshold POSTs, good and bad;
    an audition arriving as a `change` event.
  - Without a bridge: 503, a `bridge` event, and still serving.
- ADR 0009 amended.
