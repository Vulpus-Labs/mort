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

- [ ] `mort view [--port N] [--open]` binds 127.0.0.1 on 7317 or the next
      free port, and prints a URL holding a random token.
- [ ] Requests without the token, or with a non-loopback `Host`, get 403.
      Tested.
- [ ] `/events` is SSE driven by `map.watch`. `/api/view`, `/api/track`
      and `/api/thresholds` pass through to the bridge.
- [ ] Losing the bridge is reported on the page and retried. The server
      does not exit.
- [ ] The page's assets are embedded with `include_str!`. No npm, no build
      step. One small HTTP dependency at most, chosen in the close-out
      with its reason.
- [ ] CLI test over the fake: start the server, fetch the view, trigger
      an audition, receive an event.
