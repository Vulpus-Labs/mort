# mort: working conventions

## Process

- Decisions live in `adrs/NNNN-slug.md`, one per topic. A change that alters
  a decision updates or supersedes the ADR in the same change.
- Work is tracked in `epics/` and `tickets/` (see `tickets/README.md`). Work
  tickets in dependency order (`depends:` frontmatter). When a ticket's
  acceptance criteria are met, tick them, add a `## Close-out (YYYY-MM-DD)`
  section and `git mv` it to `tickets/closed/`. Close an epic the same way
  when its last ticket closes.
- Commit per ticket, with the ticket number in the subject.

## Code

- REAPER's API is called only on the main thread, from the bridge's
  dispatcher (ADR 0002). Socket and job threads never call it.
- Probe audio-thread code must not allocate, lock or make system calls
  (ADR 0005).
- Logic that does not need REAPER goes in crates that do not depend on it,
  and is tested against the fake session (ADR 0011).
- Errors name the element they are about (track, FX, parameter, position)
  and say what to do about it.
- Feature definitions and finding thresholds are measured, not guessed
  (ADR 0007). Measure on real material before changing them.

## Verification

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```
