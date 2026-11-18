# ADR 0010: Instrument manifests

- **Status:** Accepted (implemented in E002)
- **Date:** 2026-11-18

## Context

Through REAPER, the agent sees a plugin's parameters as names, normalised
values and display text. That is enough to turn knobs, but not to know
what a knob does: whether it is a filter cutoff on a log taper, which
parameters belong to which oscillator, or that a value of 0.62 means
"brighter". When Fenland was composed, the engines' meanings were known
directly.

## Decision

1. **A manifest is a JSON file per plugin id**, giving for each parameter
   or group of parameters:
   - a role, from a small vocabulary such as `filter.cutoff`, `env.attack`
     or `osc.level`;
   - a unit and a mapping from normalised value to real value;
   - its group;
   - a sentence of description.

   It may also describe presets and their character.
2. **Where manifests come from:**
   - plugins that ship one in their bundle at
     `Contents/Resources/mort-manifest.json`;
   - the user's directory `~/Library/Application Support/mort/manifests/`,
     which wins over the bundle.
3. **silkscreen plugins generate theirs** at build time from their typed
   parameter definitions, in line with silkscreen's preference for
   generated code. That work is ticketed in silkscreen, not here.
4. **The bridge attaches manifest data** to `params` results and accepts
   real units in `set` (`--cutoff 2.4kHz`) where a mapping exists.
5. **Without a manifest**, everything still works through names and
   display text.

## Consequences

- silkscreen instruments become first-class in mort, and third-party
  plugins stay usable.
- A hand-written manifest for a third-party synth is a reasonable way to
  teach mort about it.
